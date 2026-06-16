# Task 111 — Native IRadio client (cellular telephony under `--no-art`)

> Scoped 2026-06-16. Under `--no-art` the **entire Java telephony stack dies**
> (`TelephonyRegistry`/`telecom`/`isub` in system_server; `phone`/`isms`/
> `carrier_config` in the `com.android.phone` APK; **RILJ** `RIL.java` in that
> same process — all ART). Only the **vendor radio HAL survives** (rild/qcril,
> native C++). To get any cellular function — signal, registration, voice, SMS,
> data — wandr must provide a **native client on the radio HAL** that reimplements
> the slice of RILJ + the telephony services it needs.

## The shape-determining fact (verified on device)

On the Pixel 2 XL (taimen, LineageOS A15) the radio HAL is **HIDL `@1.4`**,
confirmed: the HAL service process is **`android.hardware.radio@1.4-service.legacy`**
(pid 3433) exposing `IRadio@1.4` on **`/dev/hwbinder`**, fronting **`rild`** (pid
1421, the Qualcomm RIL daemon → modem over QMI). No AIDL radio on `/dev/binder`.
The `.legacy` service is AOSP's wrapper that turns old-style `rild` into HIDL
`IRadio`. Also present: `ISap@1.0/1.1`, `com.qualcomm.qti.ims.radio@1.0` (IMS).
**`rsbinder` is AIDL/`/dev/binder`-only — it does not speak HIDL/hwbinder** (the
just-released `v0.9.0`, 2026-06-15, adds only AIDL-codegen + parcel-decode
hardening, no HIDL — see "rsbinder" below). Therefore the client **cannot be pure
Rust/rsbinder**. It must follow the **`wandr-sensormanager` pattern**: a **C++ HIDL
shim** (built on a-03 against vendored AOSP radio HIDL headers) that talks `IRadio`
over hwbinder and **bridges to Rust via a local AIDL service** on `/dev/binder`
(which rsbinder *can* consume). Same model already proven for sensors (HIDL) and
camera-EIS gyro.

> ⚠️ Dev/test needs a **SIM** — `gsm.sim.state=ABSENT` on the test phone, and the
> main `IRadio` interface isn't even registered without one. Get a (data-capable,
> CS-voice-capable) SIM before M0.

## Architecture (mirrors `wandr-sensormanager`)

```
guest dialer/messaging (wasi:canvas)        ← phone + SMS UI
        │  wandr:telephony WIT
        ▼
wandr-arbiter-radio  (Rust module/daemon)   ← state machine, request serialization,
        │  AIDL  (rsbinder, /dev/binder)        solicited/unsolicited split, WIT host impl
        ▼
wandr-radio  (C++ HIDL shim, built on a-03) ← IRadioResponse/IRadioIndication callbacks,
        │  HIDL  (/dev/hwbinder)                IRadio requests; registers `wandr.radio` AIDL
        ▼
rild / qcril  (vendor C++, survives --no-art)
        ▼
   modem
```

- **`wandr-radio` (C++ shim, a-03):** implement `IRadioResponse` + `IRadioIndication`,
  call `IRadio` requests (`setRadioPower`, `getSignalStrength`,
  `getVoiceRegistrationState`/`getDataRegistrationState`, `getOperator`,
  `getIccCardStatus`, `dial`/`hangup`/`acceptCall`/`getCurrentCalls`,
  `sendSms`/`acknowledgeLastIncomingGsmSms`, `setupDataCall`/`deactivateDataCall`).
  Target the device's confirmed **`android.hardware.radio@1.4`** (`IRadio@1.4` +
  `IRadioResponse@1.4` + `IRadioIndication@1.4`); vendor those HIDL headers.
  Registers a single AIDL bridge `wandr.radio`.
- **`wandr-arbiter-radio` (Rust):** owns the RIL-style request/response correlation
  (serial numbers), the unsolicited-indication fan-out, and a small telephony state
  model (radio power, reg state, signal, SIM, active calls). Exposes a new
  **`wandr:telephony` WIT** (host impl in `wandr-host`) + emits `wandr:events`
  topics so chrome (status bar signal icon) updates. Reuses the `wandr-hal-*` crate
  shape (cfg-android + no-op stub).
- **Guests:** a dialer (`apps/system/wandr.phone` or user app) + a messaging app,
  pure `wasi:canvas` + `wasi:input-handlers`.

## What can be reused vs must be written fresh

- **Reuse:** the HIDL-shim→AIDL-bridge→Rust pattern + a-03 build flow
  (`wandr-sensormanager`, `wandr-radio` naming); the artless **call-audio routing**
  (`project_artless_call_audio` — earpiece/speaker, focus::call_start) for voice
  audio; **wandr-net** + `netd` for the data path; the status-bar/`wandr:events`
  plumbing for signal/operator display.
- **Write fresh:** there is **no C++ RIL client to reuse** — RILJ is Java, so the
  request serialization, solicited/unsolicited handling, call/SMS/data state
  machines, PDU encode/decode, and carrier defaults must be reimplemented.
- **Unrelated:** `wandr-call` is IP/WebRTC VoIP — orthogonal to this CS/cellular
  path (though a future "calls" UI could unify both).

## Milestones

- **M0 — spike (sole-client proof):** C++ probe on hwbinder; `setRadioPower(on)`,
  `getIccCardStatus`, `getSignalStrength`, `getVoiceRegistrationState`. Prove we can
  drive the modem under `--no-art` with RILJ dead (we're the sole HAL client — watch
  for the single-client handoff race seen with sensors). **Gated on a SIM.**
- **M1 — read-only telephony:** radio power + signal + registration + operator +
  SIM status → `wandr:telephony` WIT → status-bar signal icon + a diagnostics guest.
- **M2 — voice calls:** `dial`/`acceptCall`/`hangup` + `getCurrentCalls` +
  call-state indications; wire audio via the artless call-audio recipe (earpiece/
  speaker, proximity screen-off already exists). Dialer guest.
- **M3 — SMS:** send + receive (GSM 7-bit/UCS2 PDU encode/decode, multipart),
  delivery reports; messaging guest + notify integration.
- **M4 — mobile data:** `setupDataCall` → bring the returned interface up via
  `wandr-net`/`netd` (route + DNS); data on/off + APN from carrier defaults.

## Risks / unknowns

- **HIDL version drift** — must vendor the exact `android.hardware.radio@1.x`
  headers the device ships; wrong minor = missing/renamed methods.
- **Single-client HAL handoff** — the radio HAL expected RILJ; reclaiming it under
  `--no-art` may hit the `DEAD_OBJECT`/ownership race we saw with sensorservice
  (`docs/artless-native-service-model.md`). Cold `--no-art` boot (no prior RILJ) is
  the clean entry.
- **VoLTE / IMS** — modern networks may be VoLTE-only (CS voice unavailable). IMS is
  a separate, much harder interface (`com.qualcomm.qti.ims.radio`) — **out of scope**
  initially; assumes a network with 2G/3G CS-voice or a CS-voice-capable SIM.
- **Emergency calls, multi-SIM, supplementary services (call waiting/forwarding),
  STK** — out of scope for now.
- **No SIM on the test device today** — blocks all of M0+.

## rsbinder (checked 2026-06-16)

`v0.9.0` tagged **2026-06-15** (`c257d63e`); our pin `5e999e04a` (2026-06-02) is on
the same line but **25 commits behind**. The delta is hardening only — **AIDL
codegen correctness** (negative byte-array defaults → `u8`, valid float constants
for non-finite defaults, reject non-compiling literals, parser hardening) +
**parcel-decode hardening** (`ParcelableHolder` stability, kernel-FD object flags) +
RPC/hub audit fixes + dep bumps. **No HIDL, no new Android version, no async/sync
change.** → Bump the pin to the `v0.9.0` tag **opportunistically** (the AIDL-codegen
+ parcel fixes benefit *all* our HAL/bridge codegen, incl. the `wandr.radio` AIDL),
but it **does not unblock IRadio** — the HIDL gap is the real blocker and is solved
by the C++ shim regardless of rsbinder version. Re-test the AudioPortFw-class
decode after the bump per `[[reference_rsbinder_version]]`.

## Status

🔲 Scoped, not started. Effort: **large** (a full RIL-client-equivalent). Gated on a
SIM. Recommend M0 as a standalone go/no-go spike before committing to M1–M4.
