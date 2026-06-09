//! Task-67 Phase 2 item (1) verification driver.
//!
//! A Rust `wasi:cli/command` that imports `wandr:signal/chat` and drives the
//! signal-engine through nothing but that contract — the exact surface a real UI
//! uses. WAC-plugged onto the engine and run under `repros/wasi-tls-runner`
//! (which grants network + Signal CA). Pace: call `poll-events` every 50 ms,
//! print whatever the engine produced. First run prints a link QR — scan it in
//! Signal → Linked devices, then send a note-to-self / message and watch it
//! arrive. Proves the engine's background tasks survive across `poll-events`.
//!
//! (Rust command, not Kotlin — the Kotlin/Wasm command adapter throws at module
//! init; see the md-smoke-rust precedent.)

wit_bindgen::generate!({
    world: "driver",
    path: "wit",
    generate_all,
});

use std::time::{Duration, Instant};

use wandr::signal::chat::{self, Event};

fn print_qr(data: &str) {
    match qrcode::QrCode::new(data.as_bytes()) {
        Ok(code) => {
            let r = code
                .render::<qrcode::render::unicode::Dense1x2>()
                .quiet_zone(true)
                .build();
            eprintln!("{r}");
        },
        Err(e) => eprintln!("[smoke] QR render failed: {e}"),
    }
}

fn main() {
    eprintln!("[smoke] init engine…");
    chat::init();

    let start = Instant::now();
    let max = Duration::from_secs(300); // 5 min ceiling
    let mut connected = false;
    let mut drain_until: Option<Instant> = None;
    let mut last_state = String::new();

    loop {
        for ev in chat::poll_events() {
            match ev {
                Event::LinkUrl(url) => {
                    eprintln!(
                        "\n[smoke] scan this in Signal → Linked devices:\n{url}"
                    );
                    print_qr(&url);
                },
                Event::Linked(number) => {
                    eprintln!("[smoke] LINKED ✓ {number}");
                },
                Event::Connected => {
                    eprintln!("[smoke] CONNECTED ✓ — fetching contacts; send a message to watch it arrive");
                    connected = true;
                    // After connecting, run another 60 s to catch contacts + live messages.
                    drain_until = Some(Instant::now() + Duration::from_secs(60));
                },
                Event::Disconnected => {
                    eprintln!("[smoke] DISCONNECTED");
                },
                Event::Message(m) => {
                    eprintln!(
                        "[smoke] MSG #{} {} {}: {}",
                        m.id,
                        if m.outgoing { "(sent)" } else { "(recv)" },
                        m.sender,
                        m.text
                    );
                },
                Event::ContactsUpdated(n) => {
                    eprintln!("[smoke] CONTACTS ✓ {n} fetched + persisted to /state:");
                    for c in chat::contacts() {
                        eprintln!(
                            "  {} | {} | {} | pos {}",
                            c.id,
                            c.name,
                            c.phone.as_deref().unwrap_or("-"),
                            c.inbox_position
                        );
                    }
                },
                Event::GroupsUpdated(n) => {
                    eprintln!("[smoke] GROUPS ✓ {n} fetched + persisted to /state:");
                    for g in chat::groups() {
                        eprintln!("  {} | {} members", g.title, g.members.len());
                    }
                },
                Event::StatusChanged(ds) => {
                    eprintln!("[smoke] STATUS msg #{} → {:?}", ds.id, ds.status);
                },
            }
        }

        // Surface state transitions.
        let state = chat::state();
        if state != last_state {
            eprintln!("[smoke] state = {state}");
            last_state = state;
        }

        // Exit conditions: post-connect drain window elapsed, or overall ceiling.
        if let Some(deadline) = drain_until {
            if Instant::now() >= deadline {
                break;
            }
        }
        if start.elapsed() >= max {
            eprintln!("[smoke] ceiling reached without {}", if connected { "messages" } else { "connecting" });
            break;
        }

        std::thread::sleep(Duration::from_millis(50));
    }

    let history = chat::history();
    eprintln!(
        "[smoke] done — state={} history={} msg(s)",
        chat::state(),
        history.len()
    );
    for m in history.iter().rev().take(5).rev() {
        eprintln!(
            "  #{} {} {}: {}",
            m.id,
            if m.outgoing { "sent" } else { "recv" },
            m.sender,
            m.text
        );
    }
}
