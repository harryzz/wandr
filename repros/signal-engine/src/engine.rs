//! The Signal engine behind the `wart:signal/chat` export. A single background
//! task (spawned on the persistent [`wart_step_executor`]) links or resumes, then
//! runs the receive+send loop; `chat.poll-events` advances that task one
//! non-blocking step and drains whatever it produced. State shared between the
//! export functions and the background task lives in a thread-local `Shared`
//! (single-threaded guest ⇒ `Rc`/`RefCell`).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;

use base64::Engine as _;
use futures::future::FutureExt;
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
    generate_registration_id, link_device, SecondaryDeviceProvisioning,
};
use libsignal_service::push_service::PushService;
use libsignal_service::receiver::MessageReceiver;
use libsignal_service::sender::MessageSender;
use libsignal_service::websocket::Unidentified;
use uuid::Uuid;

use crate::exports::wart::signal::chat::{Event, Message};
use crate::persist;
use crate::store::MemStore;

// ---- shared state ----------------------------------------------------------

struct Shared {
    events: RefCell<VecDeque<Event>>,
    history: RefCell<Vec<Message>>,
    outbox: RefCell<VecDeque<String>>,
    state: RefCell<String>,
    next_id: RefCell<u64>,
}

impl Shared {
    fn push_event(&self, e: Event) {
        self.events.borrow_mut().push_back(e);
    }

    fn set_state(&self, s: impl Into<String>) {
        *self.state.borrow_mut() = s.into();
    }

    fn next_id(&self) -> u64 {
        let mut n = self.next_id.borrow_mut();
        *n += 1;
        *n
    }

    /// Record a message in history and emit it as a live event.
    fn add_message(&self, m: Message) {
        self.history.borrow_mut().push(m.clone());
        self.push_event(Event::Message(m));
    }
}

thread_local! {
    static SHARED: RefCell<Option<Rc<Shared>>> = const { RefCell::new(None) };
}

fn shared() -> Option<Rc<Shared>> {
    SHARED.with(|s| s.borrow().clone())
}

// ---- export entry points (called from the Guest impl in lib.rs) ------------

pub fn init() {
    if shared().is_some() {
        return; // idempotent
    }
    wart_step_executor::init();

    // Preload persisted history, assigning stable ids.
    let mut history = Vec::new();
    let mut id = 0u64;
    for m in persist::load_messages() {
        id += 1;
        history.push(Message {
            id,
            sender: m.from,
            text: m.text,
            ts: m.ts,
            outgoing: m.outgoing,
        });
    }

    let s = Rc::new(Shared {
        events: RefCell::new(VecDeque::new()),
        history: RefCell::new(history),
        outbox: RefCell::new(VecDeque::new()),
        state: RefCell::new("starting".to_string()),
        next_id: RefCell::new(id),
    });
    SHARED.with(|slot| *slot.borrow_mut() = Some(s));

    // Detach: the root task must outlive this call (it cancels on drop).
    wart_step_executor::spawn(run()).detach();
}

pub fn poll_events() -> Vec<Event> {
    if shared().is_none() {
        return Vec::new();
    }
    // Advance the background task(s) without blocking the frame.
    wart_step_executor::step();
    shared()
        .map(|s| s.events.borrow_mut().drain(..).collect())
        .unwrap_or_default()
}

pub fn send(text: String) -> Result<(), String> {
    let shared = shared().ok_or("engine not initialized")?;
    if text.trim().is_empty() {
        return Err("empty message".to_string());
    }
    // Local echo into history + live feed; the actual wire send happens in the
    // background task within ~200 ms (see `receive_and_send`).
    let msg = Message {
        id: shared.next_id(),
        sender: "me".to_string(),
        text: text.clone(),
        ts: now_ms(),
        outgoing: true,
    };
    let _ = persist::append_message(&to_stored(&msg));
    shared.add_message(msg);
    shared.outbox.borrow_mut().push_back(text);
    Ok(())
}

pub fn history() -> Vec<Message> {
    shared()
        .map(|s| s.history.borrow().clone())
        .unwrap_or_default()
}

pub fn state() -> String {
    shared()
        .map(|s| s.state.borrow().clone())
        .unwrap_or_else(|| "uninitialized".to_string())
}

// ---- background task: link/resume, then receive + send ---------------------

async fn run() {
    let shared = match shared() {
        Some(s) => s,
        None => return,
    };

    let setup = match persist::load_account() {
        Some(account) => resume(account),
        None => link(&shared).await,
    };
    let (store, credentials, aci, device_id) = match setup {
        Ok(v) => v,
        Err(e) => {
            shared.set_state(format!("error: {e}"));
            shared.push_event(Event::Disconnected);
            return;
        },
    };

    if let Err(e) =
        receive_and_send(&shared, store, credentials, aci, device_id).await
    {
        shared.set_state(format!("error: {e}"));
        shared.push_event(Event::Disconnected);
    }
}

/// First-run link as a secondary device. Emits `link-url` (the QR payload) then
/// `linked`, persisting the account so later runs `resume` instead.
async fn link(
    shared: &Rc<Shared>,
) -> Result<(MemStore, ServiceCredentials, Uuid, u32), String> {
    shared.set_state("linking");

    let password = gen_password();
    let aci_store = MemStore::new(
        IdentityKeyPair::generate(&mut seed_rng()),
        generate_registration_id(&mut seed_rng()),
    );
    let mut pni_store = MemStore::new(
        IdentityKeyPair::generate(&mut seed_rng()),
        generate_registration_id(&mut seed_rng()),
    );
    let mut aci_for_task = aci_store.clone();

    let push =
        PushService::new(SignalServers::Production, None, "wart-signal-engine");
    let (tx, mut rx) = futures::channel::mpsc::channel(1);
    let pw = password.clone();
    let task = wart_step_executor::spawn(async move {
        let mut csprng = seed_rng();
        link_device(
            &mut aci_for_task,
            &mut pni_store,
            &mut csprng,
            push,
            &pw,
            "wart",
            tx,
        )
        .await
    });

    let mut registration = None;
    while let Some(step) = rx.next().await {
        match step {
            SecondaryDeviceProvisioning::Url(url) => {
                shared.push_event(Event::LinkUrl(url.to_string()));
            },
            SecondaryDeviceProvisioning::NewDeviceRegistration(reg) => {
                registration = Some(reg);
            },
        }
    }
    task.await.map_err(|e| format!("link: {e}"))?;
    let reg = registration.ok_or("no registration received")?;

    let identity =
        IdentityKeyPair::new(reg.aci_public_key, reg.aci_private_key);
    aci_store.set_identity(identity, reg.registration_id);

    let account = persist::Account {
        aci: reg.service_ids.aci,
        pni: reg.service_ids.pni,
        number: reg.phone_number.to_string(),
        password: password.clone(),
        device_id: u32::from(reg.device_id),
        registration_id: reg.registration_id,
        identity_b64: b64().encode(identity.serialize()),
    };
    persist::save_account(&account)
        .map_err(|e| format!("save account: {e}"))?;
    persist::save_snapshot(&aci_store.snapshot_bytes())
        .map_err(|e| format!("save snapshot: {e}"))?;

    shared.set_state("linked");
    shared.push_event(Event::Linked(reg.phone_number.to_string()));

    let credentials = ServiceCredentials {
        aci: Some(reg.service_ids.aci),
        pni: Some(reg.service_ids.pni),
        phonenumber: reg.phone_number,
        password: Some(password),
        device_id: Some(reg.device_id),
    };
    Ok((aci_store, credentials, reg.service_ids.aci, u32::from(reg.device_id)))
}

/// Re-authenticate a previously linked device from the persisted account.
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
    Ok((store, credentials, account.aci, account.device_id))
}

/// Open the authenticated message socket, then loop: receive+decrypt envelopes
/// and, on a short timer, drain the outbox over the same connection.
async fn receive_and_send(
    shared: &Rc<Shared>,
    store: MemStore,
    credentials: ServiceCredentials,
    aci: Uuid,
    device_id: u32,
) -> Result<(), String> {
    let pni = credentials.pni.unwrap_or(aci);
    let push = PushService::new(
        SignalServers::Production,
        Some(credentials.clone()),
        "wart-signal-engine",
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

    // Build the sender once, reusing the pipe's identified socket. If the
    // unidentified socket can't be opened, receiving still works; sends are
    // disabled and reported via `state`.
    let device =
        DeviceId::try_from(device_id).map_err(|e| format!("device id: {e}"))?;
    let mut sender = match push
        .clone()
        .ws::<Unidentified>("/v1/websocket/", "/v1/keepalive", &[], None)
        .await
    {
        Ok(unidentified_ws) => Some(MessageSender::new(
            pipe.ws(),
            unidentified_ws,
            push.clone(),
            cipher.clone(),
            store.clone(),
            aci,
            pni,
            store.identity(),
            None,
            device,
        )),
        Err(e) => {
            shared.set_state(format!("send unavailable: {e}"));
            None
        },
    };
    let self_id = ServiceId::Aci(aci.into());

    shared.set_state("connected");
    shared.push_event(Event::Connected);

    let mut stream = Box::pin(pipe.stream());
    loop {
        let tick = wart_step_executor::sleep(Duration::from_millis(200));
        futures::select! {
            item = stream.next().fuse() => match item {
                Some(Ok(Incoming::Envelope(env))) => {
                    let content = match cipher.open_envelope(env, &mut csprng).await {
                        Ok(c) => c,
                        Err(e) => {
                            shared.set_state(format!("decrypt error: {e}"));
                            None
                        },
                    };
                    // The ratchet may have advanced — persist after every envelope.
                    let _ = persist::save_snapshot(&store.snapshot_bytes());
                    if let Some(content) = content {
                        if let (Some(text), outgoing) = extract(&content.body) {
                            let msg = Message {
                                id: shared.next_id(),
                                sender: format!("{:?}", content.metadata.sender),
                                text,
                                ts: now_ms(),
                                outgoing,
                            };
                            let _ = persist::append_message(&to_stored(&msg));
                            shared.add_message(msg);
                        }
                    }
                },
                Some(Ok(Incoming::QueueEmpty)) => {},
                Some(Err(e)) => return Err(format!("message pipe: {e}")),
                None => {
                    shared.push_event(Event::Disconnected);
                    return Ok(());
                },
            },
            _ = tick.fuse() => {
                if let Some(sender) = sender.as_mut() {
                    let pending: Vec<String> =
                        shared.outbox.borrow_mut().drain(..).collect();
                    for text in pending {
                        let now = now_ms();
                        let dm = DataMessage {
                            body: Some(text),
                            timestamp: Some(now),
                            ..Default::default()
                        };
                        match sender
                            .send_message(&self_id, None, dm, now, false, false)
                            .await
                        {
                            Ok(_) => {
                                let _ = persist::save_snapshot(
                                    &store.snapshot_bytes(),
                                );
                            },
                            Err(e) => shared.set_state(format!("send error: {e}")),
                        }
                    }
                }
            },
        }
    }
}

// ---- helpers ---------------------------------------------------------------

fn extract(body: &ContentBody) -> (Option<String>, bool) {
    match body {
        ContentBody::DataMessage(dm) => (dm.body.clone(), false),
        ContentBody::SynchronizeMessage(sm) => (
            sm.sent
                .as_ref()
                .and_then(|s| s.message.as_ref())
                .and_then(|m| m.body.clone()),
            true,
        ),
        _ => (None, false),
    }
}

fn to_stored(m: &Message) -> persist::StoredMessage {
    persist::StoredMessage {
        from: m.sender.clone(),
        text: m.text.clone(),
        ts: m.ts,
        outgoing: m.outgoing,
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

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
