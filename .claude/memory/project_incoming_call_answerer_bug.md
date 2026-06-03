---
name: project_incoming_call_answerer_bug
description: "OPEN BUG: wart-signal INCOMING calls (answerer role) never connect media — ICE stuck Checking/pair=none even after relay both-ways works. Outgoing works. Plus multi-ring coordination missing."
metadata: 
  node_type: memory
  type: project
  originSessionId: 60a5ba7d-3852-4a04-bc9b-dc30175ddbfb
---

**OPEN BUG (2026-06-03): incoming Signal calls don't connect.** Outgoing calls
work fully (incl. off-LAN via TURN, [[project_wart_call]]). Incoming has been
broken ~1-2 sessions (pre-existing, not caused by the task-76/Phase-B work).

**Symptom:** peer calls wart → wart rings + answers → "TX Answer OK" (caller
receives it; reject/Hangup also works → signaling delivery is fine) → but the
call never connects media; caller UI sits at "ringing" forever. Diagnosed via the
`conn:` line in `/state/calldbg.log` (added this session): the answerer's ICE
stays `role=Answerer ice=Checking pair=none keyed=false` for the whole call —
**it never selects a candidate pair**, so keys never derive (Signal DH keys derive
in `Transport::maybe_advance` only once `get_selected_candidate_pair()` is Some).

**Why answerer ≠ offerer:** offerer is ICE-**controlling** (connects off its OWN
checks succeeding + it nominates); answerer is **controlled** (needs to RESPOND to
the peer's checks so the peer's pair succeeds → peer sends USE-CANDIDATE → we
select). The controlled+relay path was never exercised. Signal forces **relay-only**
candidates (privacy) — even same-WiFi has no host pair — so incoming is always
relay-to-relay.

**Fixes landed this session (real, partial — relay transport now works both ways
for the answerer, but ICE still won't complete):** in `crates/wart-call/src/transport.rs`
`handle_datagram` TURN branch + `RelayState::drain_events`, for every address we
receive a relayed packet FROM (the peer's *actual* relay sending addr X, which
differs from the advertised relay candidates Y1-3 — TURN permissions are per-IP so
we can receive, but rtc_turn gates SENDING per ip:port):
  1. `relay_remotes.insert(X)` — route our replies to X THROUGH the relay (was sent
     direct → silently dropped).
  2. `RelayState::permit(X)` — create a TURN permission so `send_to(X)` is allowed
     (was ErrNoPermission → `noperm` counter climbed, `sent` stuck).
  3. `ice.add_remote_candidate(host_candidate(X))` — so our agent checks X directly
     (else it only checks advertised Y; the response arrives from X and ICE's
     symmetric-NAT guard `transaction_addr(Y) != remote_addr(X)` discards it).
Result: `relay[sent↑ recv↑ noperm=3-stable]` (exchange works) but STILL `pair=none`.

**ROOT CAUSE (confirmed by reading rtc-ice) + CORRECT FIX:** our whole relay
approach is wrong — we keep the TURN relay OUT-OF-BAND (a hand-rolled `RelayState`
+ manual `relay_remotes` routing + injection) and leave the ICE agent with only a
HOST local candidate. So the agent's address bookkeeping (which local candidate a
check came from / goes to, the symmetric-NAT guard) never matches the relayed
reality → the controlled (answerer) role can't validate a pair. The offerer
(controlling) connected anyway because controlling only needs its OWN checks to
succeed; controlled is stricter (needs to respond correctly + be nominated).

**rtc-ice ALREADY supports relay as a first-class ICE candidate** — exactly how
ringrtc/libwebrtc do it: `candidate/candidate_relay.rs` `CandidateType::Relay` /
`CandidateRelayConfig`, `AgentConfig.urls` w/ `SchemeType::Turn`, and
`candidate_relay_test.rs` does a full **relay-only** connect (both agents gather
relay candidates from a TURN server). The async `ice` Agent owns the TURN client;
the relayed addr is a proper LOCAL candidate so matching is correct by
construction. **Fix = rework to use this** instead of the out-of-band hack: add
the relay as a real ICE LOCAL candidate (`new_candidate_relay`) and route the
agent's transmits for that candidate through the TURN client (our `Transport` is
sans-io, so we still own the socket + must wrap relay-local transmits as TURN
ChannelData and unwrap TURN receives back to the agent as arriving on the relay
local candidate — but with the candidate registered, the agent's pairing/matching
is correct). Then DELETE the `relay_remotes` routing hack + the
relayed-receive `add_remote_candidate`/`permit` patches added this session (commit
WIP <fill>). Check the async `ice` crate's relay wiring (or ringrtc) for the exact
sans-io seam. This fixes BOTH roles, not just the answerer.

Partial fixes landed this session (relay now exchanges ICE both ways for the
answerer — sent/recv climb, noperm fixed — but pair still never selected; see
above for why): in `transport.rs` on relayed-receive we insert into
`relay_remotes`, `permit(peer)`, and `add_remote_candidate(host_candidate(peer))`.
These are stopgaps to be REPLACED by the proper relay-candidate integration.

**SEPARATE bug (also open): multi-ring coordination missing.** User has 3 devices
on one account; all ring; answering on wart stops wart's ring but the other phone
+ desktop KEEP ringing. wart never sends the ringrtc `Hangup{type=Accepted,
device_id=self}` to the user's own other devices (and to the caller). In
`apps/user/war.signal/engine/src/call.rs` `signal_to_call_message`, `Hangup.device_id`
is hardcoded `None`; there's no Accepted-hangup-to-self path. Fix after the connect
bug.

Diagnostics added (uncommitted/committed-as-WIP): `Transport::conn_debug()` +
relay sent/noperm/recv counters → surfaced through `SignalCall`/`CallEngine` →
engine logs `conn: ...` ~1 Hz to `/state/calldbg.log`.
