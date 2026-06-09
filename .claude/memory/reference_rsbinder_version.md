---
name: reference_rsbinder_version
description: "wandr-host uses rsbinder + rsbinder-aidl pinned to git master 0.9.0 (rev 5e999e04a), NOT crates.io 0.8.0 — required to decode AudioPortFw; supersedes stale \"0.7.0/0.8.0\" notes."
metadata: 
  node_type: memory
  type: reference
  originSessionId: 60a5ba7d-3852-4a04-bc9b-dc30175ddbfb
---

`runtime/wandr-host/Cargo.toml` pins **rsbinder + rsbinder-aidl to git master,
rev `5e999e04a` (version 0.9.0)** — not the crates.io `0.8.0`. Adopted 2026-06-03
(task 76 #6, commit 1b20c1cb).

**Why:** rsbinder-aidl **0.8.0 mis-decodes `AudioPortFw`** (the framework audio
port parcelable returned by `IAudioPolicyService.listAudioPorts`/`getAudioPort`)
— a union-with-parcelable-variant (`AudioPortExt`) → the union template's
unknown-tag arm returns `StatusCode::BadValue`. Proven rigorously:
- bisect: `AudioProfile`/`AudioChannelLayout`/`AudioDevice` decode fine
  (`getDevicesForAttributes`, `getDirectProfilesForAttributes`); only
  `AudioPortFw` fails (`getAudioPort` single + `listAudioPorts` array).
- ruled out drift: a-03 LineageOS AIDL **byte-identical** (md5) to vendored r36.
- ruled out permission/protocol: server returns OK for valid args.
- **master/0.9.0 decodes it correctly** (device-verified: all 8 ports, exact id
  match to dumpsys).

**Implications / gotchas:**
- The `android_11_plus` feature still exists in 0.9.0 (umbrella
  `android_11..android_16`); the dep keeps it.
- 0.9.0 also reworked RPC/calling-identity/codegen — our kernel-binder usage
  compiled with **zero API churn**, and a full regression (AAudio openStream,
  getDevicesForAttributes, getPhoneState/getForceUse, volume, full stack boot)
  passed clean.
- **Stale memories to disregard:** any note saying "rsbinder-aidl 0.7.0" limits
  (e.g. recursive parcelable, @nullable). We're on 0.9.0; unions (incl. nested,
  with parcelable variants), `@nullable`→`Option`, arrays-of-parcelables all
  work. Recursive types need `@nullable(heap=true)`→`Box` (per docs).
- When a crates.io 0.9.x release lands, switch the git rev to the version pin.

Docs read: <https://moru.rs/rsbinder/> (overview, parcelable, enum-union,
print.html). See [[project_audio_capability_model]], [[project_audio_routing_arbiter]].


**Stub cleanup (30c984ab):** wandr-host now generates from the **real**
`libaudioclient/aidl/IAudioPolicyService.aidl` (all 106 methods, codegen-derived
indices) — the brittle hand-maintained positional slot-stub
(`vendor/aidl-stubs/android/media/IAudioPolicyService.aidl` + AudioPolicyForceUse
/ForcedConfig) is DELETED. build.rs includes `frameworks-av/aidl` for the
permission/VolumeShaper types. **rsbinder-aidl 0.9.0 parse quirk:** it can't
parse a `float[]` default with `0f` literals — `HeadTracking.aidl`'s
`float[6] headToStage = {0f,...}` (pulled via the spatializer methods) → build.rs
strips that default in-place (idempotent, self-healing, submodule stays pristine;
we never call the spatializer). The aidl-stubs dir keeps only the unrelated
AttributionSourceState / PersistableBundle / ParcelFileDescriptor stubs.
