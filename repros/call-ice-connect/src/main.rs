//! Call engine — Stage 2b: ICE connectivity, in a wasm32-wasip2 guest.
//!
//! Two sans-IO `rtc-ice` Agents (controlling + controlled) exchange host
//! candidates + ICE credentials and run connectivity checks (STUN binding
//! request/response) until a candidate pair is selected. Proves the ICE agent
//! reaches connectivity in a guest. The checks flow over an in-memory loopback
//! wire (we hand each agent's `poll_write` to the other's `handle_read`); real
//! `wasi:sockets` UDP is de-risked separately (../wasi-udp-probe), and the final
//! assembly swaps this wire for it. DTLS (Stage 2a) then runs over the selected
//! pair.
//!
//! Run on-device: `wandr-host --run-once wandr.probe.ice`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use sansio::Protocol;

use rtc_ice::agent::agent_config::AgentConfig;
use rtc_ice::agent::Agent;
use rtc_ice::candidate::candidate_host::CandidateHostConfig;
use rtc_ice::candidate::{Candidate, CandidateConfig};
use rtc_ice::mdns::MulticastDnsMode;
use rtc_ice::state::ConnectionState;
use rtc_shared::{TaggedBytesMut, TransportContext, TransportProtocol};

const A_UFRAG: &str = "ufrAgentA";
const A_PWD: &str = "passwordApasswordApasswordA00";
const B_UFRAG: &str = "ufrAgentB";
const B_PWD: &str = "passwordBpasswordBpasswordB00";

fn host_candidate(port: u16) -> Candidate {
    CandidateHostConfig {
        base_config: CandidateConfig {
            network: "udp".to_owned(),
            address: "127.0.0.1".to_owned(),
            port,
            component: 1,
            ..Default::default()
        },
        ..Default::default()
    }
    .new_candidate_host()
    .expect("host candidate")
}

fn agent(ufrag: &str, pwd: &str) -> Agent {
    Agent::new(Arc::new(AgentConfig {
        local_ufrag: ufrag.to_owned(),
        local_pwd: pwd.to_owned(),
        multicast_dns_mode: MulticastDnsMode::Disabled,
        ..Default::default()
    }))
    .expect("agent")
}

fn connected(a: &Agent) -> bool {
    matches!(a.state(), ConnectionState::Connected | ConnectionState::Completed)
}

fn main() {
    let mut a = agent(A_UFRAG, A_PWD);
    let mut b = agent(B_UFRAG, B_PWD);

    // Each gathers its host candidate (one loopback UDP port stands in for the
    // socket the final assembly binds).
    let a_cand = host_candidate(40001);
    let b_cand = host_candidate(40002);
    a.add_local_candidate(a_cand.clone()).expect("a local");
    b.add_local_candidate(b_cand.clone()).expect("b local");

    // Signaling (done in-process here): swap candidates + credentials.
    a.add_remote_candidate(b_cand).expect("a remote");
    b.add_remote_candidate(a_cand).expect("b remote");

    a.start_connectivity_checks(true, B_UFRAG.to_owned(), B_PWD.to_owned()).expect("a start");
    b.start_connectivity_checks(false, A_UFRAG.to_owned(), A_PWD.to_owned()).expect("b start");
    println!("[ice] two agents started (A=controlling, B=controlled)");

    // Pump: fire timers (queues binding requests), then flush poll_write →
    // handle_read both ways until the request/response exchange settles.
    let mut datagrams = 0u32;
    let mut done_round = None;
    for round in 0..600 {
        let now = Instant::now();
        let _ = a.handle_timeout(now);
        let _ = b.handle_timeout(now);

        // Several drain passes so a request and its response both move this round.
        for _ in 0..6 {
            let mut moved = false;
            while let Some(t) = a.poll_write() { datagrams += 1; moved = true; deliver(&mut b, t); }
            while let Some(t) = b.poll_write() { datagrams += 1; moved = true; deliver(&mut a, t); }
            if !moved { break; }
        }
        while a.poll_event().is_some() {}
        while b.poll_event().is_some() {}

        if connected(&a) && connected(&b) {
            done_round = Some(round);
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    let round = done_round.expect("ICE did not reach connectivity");
    println!("[ice] connectivity in {round} rounds, {datagrams} STUN datagrams");
    println!("[ice] A state={:?} B state={:?}", a.state(), b.state());

    let (la, ra) = a.get_selected_candidate_pair().expect("A selected pair");
    println!("[ice] A selected pair: local={} ↔ remote={}", la.addr(), ra.addr());
    let (lb, rb) = b.get_selected_candidate_pair().expect("B selected pair");
    println!("[ice] B selected pair: local={} ↔ remote={}", lb.addr(), rb.addr());

    assert!(connected(&a) && connected(&b), "both agents must be connected");
    println!("ICE OK — two rtc-ice agents reached connectivity over the wire on wasm32-wasip2");
}

/// Deliver a sender's outgoing packet to the receiver, swapping the transport
/// context (the receiver sees it arriving from the sender).
fn deliver(rx: &mut Agent, t: TaggedBytesMut) {
    let msg = TaggedBytesMut {
        now: Instant::now(),
        transport: TransportContext {
            local_addr: t.transport.peer_addr,
            peer_addr: t.transport.local_addr,
            transport_protocol: TransportProtocol::UDP,
            ecn: None,
        },
        message: t.message,
    };
    let _ = rx.handle_read(msg);
}
