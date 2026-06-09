//! Task-67 Phase 1 — Signal client (link + receive) with pure-Rust file
//! persistence over WASI fs (`/state`). First run links + persists; later runs
//! resume the linked device (no re-link), load message history, and keep
//! receiving + decrypting + persisting incoming texts. wasm32-wasip2 guest, all
//! transport over task-66 host wasi:tls.
//!
//! Desktop run (state in ./signal-state, persists across runs):
//!   cargo build --target wasm32-wasip2 --release
//!   (cd ../wasi-tls-runner && cargo run --release -- \
//!       ../signal-link/target/wasm32-wasip2/release/signal-link.wasm signal-state)

use base64::Engine;
use futures::StreamExt;
use rand::SeedableRng;

use libsignal_service::cipher::ServiceCipher;
use libsignal_service::configuration::{
    ServiceConfiguration, ServiceCredentials, SignalServers,
};
use libsignal_service::content::ContentBody;
use libsignal_service::messagepipe::Incoming;
use libsignal_service::proto::DataMessage;
use libsignal_service::protocol::{
    DeviceId, IdentityKeyPair, ProtocolAddress, ServiceId,
};
use libsignal_service::provisioning::{
    generate_registration_id, link_device, NewDeviceRegistration,
    SecondaryDeviceProvisioning,
};
use libsignal_service::push_service::PushService;
use libsignal_service::receiver::MessageReceiver;
use libsignal_service::sender::MessageSender;
use libsignal_service::websocket::Unidentified;
use uuid::Uuid;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

mod persist;
mod store;
use store::MemStore;

fn b64() -> base64::engine::general_purpose::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

fn seed_rng() -> rand_chacha::ChaCha20Rng {
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).expect("wasi getrandom");
    rand_chacha::ChaCha20Rng::from_seed(seed)
}

fn gen_password() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("wasi getrandom");
    b64().encode(bytes)
}

fn print_qr(data: &str) {
    match qrcode::QrCode::new(data.as_bytes()) {
        Ok(code) => {
            let r = code
                .render::<qrcode::render::unicode::Dense1x2>()
                .quiet_zone(true)
                .build();
            eprint!("{r}\n");
        },
        Err(e) => eprintln!("[signal-link] QR render failed: {e}"),
    }
}

async fn do_link(
    mut aci: MemStore,
    mut pni: MemStore,
    password: String,
) -> Result<NewDeviceRegistration, String> {
    let push =
        PushService::new(SignalServers::Production, None, "wandr-signal-link");
    let (tx, mut rx) = futures::channel::mpsc::channel(1);
    let task = wart_step_executor::spawn(async move {
        let mut csprng = seed_rng();
        link_device(
            &mut aci, &mut pni, &mut csprng, push, &password, "wandr", tx,
        )
        .await
    });

    let mut registration = None;
    while let Some(step) = rx.next().await {
        match step {
            SecondaryDeviceProvisioning::Url(url) => {
                eprintln!("[signal-link] scan this in Signal → Linked devices:");
                eprintln!("{url}");
                print_qr(url.as_str());
            },
            SecondaryDeviceProvisioning::NewDeviceRegistration(reg) => {
                registration = Some(reg);
            },
        }
    }
    task.await.map_err(|e| format!("link: {e}"))?;
    registration.ok_or_else(|| "no registration received".to_string())
}

/// Returns the store + credentials + (aci, device_id) for the receive loop.
async fn first_run_link(
) -> Result<(MemStore, ServiceCredentials, Uuid, u32), String> {
    let password = gen_password();
    let aci_store = MemStore::new(
        IdentityKeyPair::generate(&mut seed_rng()),
        generate_registration_id(&mut seed_rng()),
    );
    let pni_store = MemStore::new(
        IdentityKeyPair::generate(&mut seed_rng()),
        generate_registration_id(&mut seed_rng()),
    );

    eprintln!("[signal-link] no saved device — starting link…");
    let reg = do_link(aci_store.clone(), pni_store, password.clone()).await?;

    let identity =
        IdentityKeyPair::new(reg.aci_public_key, reg.aci_private_key);
    aci_store.set_identity(identity, reg.registration_id);

    let line = format!(
        "[signal-link] LINKED ✓  number={} device_id={}\n",
        reg.phone_number,
        u32::from(reg.device_id),
    );
    eprint!("{line}");

    // Persist so the next run resumes instead of re-linking.
    let account = persist::Account {
        aci: reg.service_ids.aci,
        pni: reg.service_ids.pni,
        number: reg.phone_number.to_string(),
        password: password.clone(),
        device_id: u32::from(reg.device_id),
        registration_id: reg.registration_id,
        identity_b64: b64().encode(identity.serialize()),
    };
    persist::save_account(&account).map_err(|e| format!("save account: {e}"))?;
    persist::save_snapshot(&aci_store.snapshot_bytes())
        .map_err(|e| format!("save snapshot: {e}"))?;

    let credentials = ServiceCredentials {
        aci: Some(reg.service_ids.aci),
        pni: Some(reg.service_ids.pni),
        phonenumber: reg.phone_number,
        password: Some(password),
        device_id: Some(reg.device_id),
    };
    Ok((aci_store, credentials, reg.service_ids.aci, u32::from(reg.device_id)))
}

fn resume(
    account: persist::Account,
) -> Result<(MemStore, ServiceCredentials, Uuid, u32), String> {
    let identity = IdentityKeyPair::try_from(
        b64()
            .decode(&account.identity_b64)
            .map_err(|e| format!("identity b64: {e}"))?
            .as_slice(),
    )
    .map_err(|e| format!("identity decode: {e}"))?;

    let store = MemStore::new(identity, account.registration_id);
    if let Some(snap) = persist::load_snapshot() {
        store.load_into(&snap)?;
    }

    let credentials = ServiceCredentials {
        aci: Some(account.aci),
        pni: Some(account.pni),
        phonenumber: account
            .number
            .parse()
            .map_err(|_| "bad saved phone number".to_string())?,
        password: Some(account.password),
        device_id: Some(
            DeviceId::try_from(account.device_id)
                .map_err(|e| format!("device id: {e}"))?,
        ),
    };
    eprintln!(
        "[signal-link] resuming linked device {} (device_id={})",
        account.number, account.device_id
    );
    Ok((store, credentials, account.aci, account.device_id))
}

async fn do_receive(
    store: MemStore,
    credentials: ServiceCredentials,
    aci: Uuid,
    device_id: u32,
) -> Result<(), String> {
    let pni = credentials.pni.unwrap_or(aci);
    let push = PushService::new(
        SignalServers::Production,
        Some(credentials.clone()),
        "wandr-signal-link",
    );
    let mut receiver = MessageReceiver::new(push.clone());
    let pipe = receiver
        .create_message_pipe(credentials, false)
        .await
        .map_err(|e| format!("open message ws: {e}"))?;

    let trust_roots = ServiceConfiguration::from(SignalServers::Production)
        .unidentified_sender_trust_roots;
    let local_address = ProtocolAddress::new(
        aci.to_string(),
        DeviceId::try_from(device_id).map_err(|e| format!("device id: {e}"))?,
    );
    let mut cipher =
        ServiceCipher::new(store.clone(), trust_roots, local_address);
    let mut csprng = seed_rng();

    // --- sending: drain /state/outbox.txt as notes-to-self ---
    let outbox = persist::take_outbox();
    if !outbox.is_empty() {
        eprintln!("[signal-link] sending {} queued message(s)…", outbox.len());
        let identified_ws = pipe.ws();
        let mut push_send = push.clone();
        match push_send
            .ws::<Unidentified>("/v1/websocket/", "/v1/keepalive", &[], None)
            .await
        {
            Ok(unidentified_ws) => {
                let mut sender = MessageSender::new(
                    identified_ws,
                    unidentified_ws,
                    push.clone(),
                    cipher.clone(),
                    store.clone(),
                    aci,
                    pni,
                    store.identity(),
                    None,
                    DeviceId::try_from(device_id)
                        .map_err(|e| format!("device id: {e}"))?,
                );
                let self_id = ServiceId::Aci(aci.into());
                for text in outbox {
                    let now = now_ms();
                    let dm = DataMessage {
                        body: Some(text.clone()),
                        timestamp: Some(now),
                        ..Default::default()
                    };
                    match sender
                        .send_message(&self_id, None, dm, now, false, false)
                        .await
                    {
                        Ok(_) => {
                            let line =
                                format!("[signal-link] SENT ✓ {text}\n");
                            eprint!("{line}");
                            let _ = persist::append_message(
                                &persist::StoredMessage {
                                    from: "me".into(),
                                    text,
                                    ts: now,
                                    outgoing: true,
                                },
                            );
                        },
                        Err(e) => {
                            eprintln!("[signal-link] send error: {e}")
                        },
                    }
                }
                let _ = persist::save_snapshot(&store.snapshot_bytes());
            },
            Err(e) => {
                eprintln!("[signal-link] could not open unidentified ws: {e}")
            },
        }
    }

    eprintln!("[signal-link] message socket open — receiving…");
    let stream = pipe.stream();
    futures::pin_mut!(stream);
    while let Some(item) = stream.next().await {
        match item {
            Ok(Incoming::Envelope(env)) => {
                let content =
                    match cipher.open_envelope(env, &mut csprng).await {
                        Ok(c) => c,
                        Err(e) => {
                            eprintln!("[signal-link] decrypt error: {e}");
                            None
                        },
                    };
                // Persist the store after every envelope — the ratchet may have
                // advanced sessions / consumed prekeys.
                let _ = persist::save_snapshot(&store.snapshot_bytes());

                if let Some(content) = content {
                    let (text, outgoing) = match &content.body {
                        ContentBody::DataMessage(dm) => {
                            (dm.body.clone(), false)
                        },
                        ContentBody::SynchronizeMessage(sm) => (
                            sm.sent
                                .as_ref()
                                .and_then(|s| s.message.as_ref())
                                .and_then(|m| m.body.clone()),
                            true,
                        ),
                        _ => (None, false),
                    };
                    if let Some(text) = text {
                        let from =
                            format!("{:?}", content.metadata.sender);
                        let _ = persist::append_message(
                            &persist::StoredMessage {
                                from: from.clone(),
                                text: text.clone(),
                                ts: 0,
                                outgoing,
                            },
                        );
                        let line = format!(
                            "[signal-link] MESSAGE ✓ {} from {from}: {text}\n",
                            if outgoing { "(sent)" } else { "(recv)" },
                        );
                        eprint!("{line}");
                    }
                }
            },
            Ok(Incoming::QueueEmpty) => {
                eprintln!("[signal-link] queue drained — waiting for new messages…")
            },
            Err(e) => return Err(format!("message pipe: {e}")),
        }
    }
    Ok(())
}

// The libsignal transport now runs on the persistent `wart-step-executor` (the
// engine in `repros/signal-engine` needs it to survive across `chat.poll-events`;
// the shared fork binds to it). This CLI drives it the simple way: spawn the app
// future, then `step()` to completion. See [[project_wart_step_executor]].
fn main() {
    wart_step_executor::init();
    let done = std::rc::Rc::new(std::cell::Cell::new(false));
    let d = done.clone();
    wart_step_executor::spawn(async move {
        run_app().await;
        d.set(true);
    })
    .detach();
    while !done.get() {
        wart_step_executor::step();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

async fn run_app() {
    let history = persist::load_messages();
    eprintln!("[signal-link] {} message(s) in history", history.len());
    for m in history.iter().rev().take(5).rev() {
        eprintln!(
            "  [{}] {}: {}",
            if m.outgoing { "sent" } else { "recv" },
            m.from,
            m.text
        );
    }

    let setup = match persist::load_account() {
        Some(account) => resume(account),
        None => first_run_link().await,
    };
    let (store, credentials, aci, device_id) = match setup {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[signal-link] setup ERROR: {e}");
            return;
        },
    };

    if let Err(e) = do_receive(store, credentials, aci, device_id).await {
        eprintln!("[signal-link] receive ERROR: {e}");
    }
}
