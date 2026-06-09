---
name: project_incoming_call_answerer_bug
description: "OPEN BUG: wandr-signal INCOMING calls (answerer role) never connect media — ICE stuck Checking/pair=none even after relay both-ways works. Outgoing works. Plus multi-ring coordination missing."
metadata: 
  node_type: memory
  type: project
  originSessionId: 60a5ba7d-3852-4a04-bc9b-dc30175ddbfb
---

**OPEN BUG (2026-06-03): incoming Signal calls don't connect.** Outgoing calls
work fully (incl. off-LAN via TURN, [[project_wandr_call]]). Incoming has been
broken ~1-2 sessions (pre-existing, not caused by the task-76/Phase-B work).

**Symptom:** peer calls wandr → wandr rings + answers → "TX Answer OK" (caller
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
for the answerer, but ICE still won't complete):** in `crates/wandr-call/src/transport.rs`
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

**REWORK DONE (the correct architecture) but incoming STILL not connecting.**
`transport.rs` now registers the relay as a first-class ICE LOCAL candidate
(`relay_candidate()` → `ice.add_local_candidate`, gated on TURN allocation via
`ensure_relay_local_candidate`) and routes by the transmit's `local_addr`:
ICE/media whose `local_addr == relayed` go through `relay.send_via`; inbound TURN
payloads are fed to `demux_payload(local=relayed, peer, …)` so the agent
associates them with the relay local candidate. `media_via_relay` set in
`maybe_advance` from the selected pair's local. The old `relay_remotes` hack +
relayed-receive `add_remote_candidate` are DELETED. Builds clean, 18 tests pass.
**Outgoing calls still work (device-verified) — no regression.** Committed
<this rework's commit>.

**✅ RESOLVED 2026-06-04 — incoming calls connect with two-way audio (device-
verified).** THREE source-grounded fixes (all from reading ringrtc `/tmp/ringrtc`,
not guessing): (1) **self-select** — the peer uses libwebrtc
`presume_writable_when_fully_relayed` and never sends USE-CANDIDATE, so the strict
controlled answerer never selected a pair; `rtc-ice Agent::self_select_best_pair()`
+ a transport answerer grace timer pick the best Succeeded pair so keys derive.
(2) **`accepted` RTP-data** — a ringrtc *caller* sits in `ConnectingBeforeAccepted`
and streams NO media until the *callee* sends `rtp_data.Message{accepted}` over
RTP PT 101 / SSRC 0xD (SRTP-encrypted, resent ~1 Hz); the answerer now sends it on
accept. (3) **self-hangup fix** — wandr (device_id 4) was hanging up on itself: the
caller's multi-device `Hangup{type=Accepted, device_id=4}` is coordination echoing
wandr's OWN accept; ringrtc ignores a hangup whose device_id is its own, so the
engine now drops a coordination hangup (Accepted/Declined/Busy) whose device_id ==
ours. SRTP KDF verified byte-identical to ringrtc `negotiate_srtp_keys` (NOT the
bug). Files: rtc-ice (submodule) + crates/wandr-call {transport,media,session,
signal/{mod,call}} + apps/user/wandr.signal/engine {call,engine}.rs. UNCOMMITTED;
rtc-ice delta should fold into tools/scripts/patch-rtc.sh. OPEN follow-ups:
diagnostics (counters/wire-log) can be trimmed; verify self-select still needed
(one call connected just before its grace fired — peer may nominate post-accept).
**MULTI-RING fully working both ways (device-verified 2026-06-04):** answer on
wandr → other devices stop; answer on another device → wandr stops. The CALLER
relays the coordination `Hangup{Accepted, device_id=winner}` to all the user's
devices, so wandr needs NO send-side change — just the device_id filter (ignore
when winner == our device_id, stop when it's a different device). Process lesson:
[[feedback_read_source_first]].

**⚠️ MAJOR CONFOUND (2026-06-03): the failing CALLER is Signal Desktop running
inside WSL behind Tailscale** — a triple-NAT (WSL→Windows→router) + VPN overlay.
PROOF = the candidates it advertised every call: `100.64.0.10` (Tailscale CGNAT
range), `172.20.40.244` (WSL2 internal net), `192.168.1.175` (Windows host LAN),
`77.70.64.156` (public srflx), `104.30.145.x` (Signal TURN). This topology
*alone* reproduces every symptom below: wandr's replies to the caller's
WSL/Windows-host address don't route back into WSL → the caller's pair never
becomes "writable" → it NEVER nominates (`usecand=0`) while it presume-writable-
relays a few SRTP packets (`seen=43`). User reports incoming likely WORKS when
the caller is a real Android Signal phone (normal network). **So the long-standing
"incoming calls don't connect" bug may be largely/entirely a TEST-RIG artifact,
not a wandr defect.** VALIDATE FIRST with a non-WSL caller before changing connect
logic. The candidate self-nominate / presume-writable fix is DEFERRED pending that.

**DEFINITIVE DIAGNOSIS (2026-06-03, instrumented rtc-ice + 2 live incoming
calls) — interpret in light of the WSL confound above.** Added 9 ICE counters to vendored `external/rtc/rtc-ice` (Agent
`IceDebug` + `ice_debug()`/`debug_pairs()`, surfaced through `conn_debug` →
`/state/calldbg.log`). Results KILLED the earlier symmetric-guard hypothesis:
`symdisc=0` (the guard at `agent_selector.rs:478` NEVER fires). The real break:
**the peer (controlling) NEVER sends USE-CANDIDATE (`usecand=0`)** so the
controlled side never selects. Many pairs reach `succ` — including a working
DIRECT-LAN pair `host->192.168.1.175(peer LAN, same WiFi)=succ` AND
`our-relay->peer-advertised-relay 104.30.145.x=succ`. So OUR checks succeed (we'd
connect if we were controlling — which is exactly why OUTGOING works); as
CONTROLLED we depend entirely on the peer nominating, which it never does. The
peer's inbound checks create prflx at X=`77.70.64.156:61856`/`192.168.1.175`
(≠ its advertised candidates; note srflx port 61856 vs advertised 61858); the
relay prflx pairs to X FAIL. Hypothesis under test: the peer (relay-forced
privacy) won't nominate the non-relay pairs that work, and the relay↔relay
nomination never completes because our relay responses to the peer's actual
checking source don't get back. Creds verified consistent (local_signaling ==
agent local ufrag/pwd — NOT the cause). GOTCHA fixed: `debug_pairs` indexed
local/remote candidate vecs that `close()` clears on hangup → index-OOB panic →
SIGILL host abort; now uses `.get()`. NEXT counters added: `reqrelay`/`reqhost`
(is the peer checking our RELAY candidate at all?) + `bsucctx` (are we sending
binding-success replies?) — one more live incoming call to read them.

**(SUPERSEDED) earlier hypothesis — STILL `ice=Checking pair=none` for the answerer** (relay_local_cand=true,
sent/recv climb, noperm plateaus). Since OUTGOING (controlling) connects with the
SAME relay-candidate routing, our outbound checks DO get matched responses over
the relay — so the break is specific to the CONTROLLED role: either our reply to
the peer's check isn't reaching it (so the peer's pair never Succeeds → it never
sends USE-CANDIDATE) or the peer's USE-CANDIDATE isn't applied. `pair=none` = no
SELECTED pair (controlled only selects on an inbound USE-CANDIDATE for a Succeeded
pair); a pair may be Succeeded-but-not-Selected. Likely still the peer's relay
SENDING addr (X) ≠ advertised (Y) breaking strict matching, even via
peer-reflexive. **NEXT: instrument vendored `external/rtc/rtc-ice`
`agent_selector.rs` (ControlledSelector) + `agent/mod.rs`: count peer-reflexive
remote-candidate creations, pairs reaching `Succeeded`, USE-CANDIDATE requests
received, symmetric-guard discards (`transaction_addr != remote_addr`), and
`set_selected_pair` calls — surfaced via a debug accessor → the `conn:` line.**
That definitively says which of {our reply not delivered / peer's pair not
Succeeding / USE-CANDIDATE not arriving / discarded} is the break. Best done in a
fresh focused session.

**SEPARATE bug (also open): multi-ring coordination missing.** User has 3 devices
on one account; all ring; answering on wandr stops wandr's ring but the other phone
+ desktop KEEP ringing. wandr never sends the ringrtc `Hangup{type=Accepted,
device_id=self}` to the user's own other devices (and to the caller). In
`apps/user/wandr.signal/engine/src/call.rs` `signal_to_call_message`, `Hangup.device_id`
is hardcoded `None`; there's no Accepted-hangup-to-self path. Fix after the connect
bug.

Diagnostics added (uncommitted/committed-as-WIP): `Transport::conn_debug()` +
relay sent/noperm/recv counters → surfaced through `SignalCall`/`CallEngine` →
engine logs `conn: ...` ~1 Hz to `/state/calldbg.log`.
