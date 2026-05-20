# Pixel 6 Pro — AIDL vs HIDL service availability for rsbinder

**Device:** Pixel 6 Pro (`raven`, codename gs101), Android 16. Non-rooted;
the wart-host app runs in the `untrusted_app` SELinux domain.
**Date:** 2026-05-20.
**Purpose:** rsbinder talks to the regular binder (`/dev/binder`) +
`servicemanager` — i.e. **AIDL** services. It cannot talk to HIDL HALs,
which live on a separate binder domain (`/dev/hwbinder` +
`hwservicemanager`). This document records which services for
telephony / audio / microphone / camera are AIDL (reachable) vs HIDL
(not reachable), so rsbinder work can be planned.

---

## TL;DR

All four target areas are **doable via rsbinder** — because the layer
an app actually uses is the **AIDL framework service**, not the vendor
HAL. The HIDL HALs sit *below* the framework and you never call them
directly.

| Target | App-facing service | Transport | rsbinder |
|---|---|---|---|
| Telephony | `phone` / `isub` / `iphonesubinfo` / `isms` / `imms` / `carrier_config` … | AIDL | ✅ (permission-gated) |
| Audio (playback) | `media.aaudio` (AAudioService), `media.audio_flinger` | AIDL | ✅ |
| Microphone (capture) | `media.audio_flinger` / `media.aaudio` (AudioRecord path) | AIDL | ✅ (RECORD_AUDIO perm) |
| Camera | `media.camera` (ICameraService) | AIDL | ✅ (CAMERA perm) |

The vendor HALs underneath are mixed:

| Vendor HAL | format | rsbinder |
|---|---|---|
| `android.hardware.camera.provider` (v3) | **AIDL** | ✅ |
| `android.hardware.audio` + `audio.effect` | **HIDL** | ❌ |
| `android.hardware.radio` + `radio.config` | **HIDL** | ❌ |
| `android.hardware.soundtrigger` | **HIDL** | ❌ |

→ HIDL is only a wall if you want to bypass the framework and hit the
`audio` / `radio` / `soundtrigger` HAL **directly**. You don't need to.

---

## How this was determined

Three sources, no root required:

1. **`adb shell service list`** — every service registered with the
   regular `servicemanager`. Everything here is AIDL/binder and is what
   rsbinder can reach (subject to SELinux). HIDL HALs do **not** appear
   here.
2. **`/vendor/etc/vintf/manifest.xml`** and `/vendor/etc/vintf/manifest/*.xml`
   — the device VINTF manifest. Each `<hal>` block carries a
   `format="aidl"` or `format="hidl"` attribute; HIDL blocks also have
   `<transport>hwbinder</transport>`. This is the authoritative
   AIDL-vs-HIDL source.
3. `lshal` — **restricted** for the shell user on a non-rooted device,
   so it could not be used.

> **Caveat — manifest *filenames* lie.** The camera VINTF fragment is
> named `android.hardware.camera.provider@2.7-service-google-apex.xml`
> — the `@2.7` looks like a HIDL version — but its *content* is
> `<hal format="aidl">` (v3, `ICameraProvider/internal/0`). Always read
> the `format=` attribute, never trust the `@version` in a filename.

---

## The two-layer model

```
   app  ──(AIDL)──►  Framework service        e.g. AAudioService,
                     (regular servicemanager)      ICameraService, ITelephony
                            │
                            ▼
                     Vendor HAL               e.g. android.hardware.audio
                     (AIDL or HIDL)                 android.hardware.radio
```

- **Framework services** are always AIDL. They are the normal API
  surface for an app. rsbinder reaches them directly.
- **Vendor HALs** are AIDL or HIDL per device. The framework service
  forwards to the HAL; the app does not.

So "is X reachable from rsbinder" almost always reduces to "is the
*framework service* for X registered with `servicemanager`" — and it
always is, because framework services are AIDL by construction.

---

## Framework services (AIDL — rsbinder-reachable)

From `service list` on the device:

**Telephony**
- `phone` — `ITelephony`
- `isub` — `com.android.internal.telephony.ISub`
- `iphonesubinfo` — `IPhoneSubInfo`
- `isms` — `ISms`, `imms` — `IMms`
- `ions` — `IOns`
- `carrier_config` — `ICarrierConfigLoader`
- `econtroller` — `IEuiccController`, `euicc_card_controller` — `IEuiccCardController`

**Audio**
- `audio` — `android.media.IAudioService`
- `media.aaudio` — `aaudio.IAAudioService`
- `media.audio_flinger` — `android.media.IAudioFlingerService`
- `media.audio_policy` — `android.media.IAudioPolicyService`

**Camera**
- `media.camera` — `android.hardware.ICameraService`
- `media.camera.proxy` — `android.hardware.ICameraServiceProxy`
- `android.frameworks.cameraservice.service.ICameraService/default`

All AIDL. Microphone has no dedicated service — capture goes through
`media.audio_flinger` / `media.aaudio` (the `AudioRecord` path).

---

## Vendor HALs

### AIDL HALs — registered with `servicemanager` (rsbinder-reachable)

From `service list`, the `android.hardware.*` entries:

```
authsecret      biometrics(.fingerprint)  bluetooth(.audio/.finder/.ranging)
boot            camera.provider           contexthub   devicestate   display
drm             dumpstate                 fingerprint  gatekeeper    gnss
graphics.allocator   health   input(.processor)   lights   location
media.c2        memtrack    neuralnetworks   nfc   oemlock
power(.stats)   security.keymint/.secureclock/.sharedsecret
sensors         thermal     usb(.gadget)    uwb   vibrator   weaver
wifi(.supplicant)
```

(Tasks 16–21 already used several of these: vibrator, sensors, power,
thermal, lights — all AIDL, confirming the approach.)

### HIDL HALs — `hwbinder`, NOT reachable from rsbinder

From `/vendor/etc/vintf/manifest.xml` (verified `format="hidl"` +
`<transport>hwbinder</transport>`):

```
android.hardware.audio              (line 16)
android.hardware.audio.effect       (line 21)
android.hardware.radio              (line 62)
android.hardware.radio.config       (line 70)
android.hardware.soundtrigger       (line 87)
```

`manifest/manifest_radioext.xml` — the radio extension HAL — is also
`format="hidl"`. `manifest/shared_modem_platform.xml` is `format="aidl"`.

> A few other VINTF fragments have `@version`-style filenames
> (`gnss@2.1-service-brcm`, `cas@1.2-service`) and may be HIDL — not
> verified here as they are outside the four target areas. Read the
> fragment's `format=` to confirm (see the filename caveat above —
> note `gnss` *also* appears AIDL-registered in `service list`).

---

## Per-target analysis

### Telephony
- **HAL:** `android.hardware.radio` — **HIDL**. Not rsbinder-reachable.
- **Framework:** `phone` (`ITelephony`), `isub`, `iphonesubinfo`,
  `isms`, `imms`, `carrier_config`, eUICC controllers — **all AIDL**.
- **Plan:** talk to the AIDL telephony framework services; never touch
  the HIDL radio HAL. **Caveat:** most telephony calls are gated by
  signature/privileged permissions (`READ_PRIVILEGED_PHONE_STATE`,
  carrier privileges, etc.). An `untrusted_app` will get
  `EX_SECURITY` / `SecurityException` on privileged calls — the binder
  *transport* is fine (AIDL), the *authorization* is the limiter.
  Non-privileged surface (basic `READ_PHONE_STATE` data) is workable
  with the runtime permission granted.

### Audio (playback)
- **HAL:** `android.hardware.audio` — **HIDL**. Not reachable.
- **Framework:** `media.aaudio` (AAudioService) and `media.audio_flinger`
  — **AIDL**. Task 21 already drove `media.aaudio` end-to-end (440 Hz
  sine on the Pixel 2 XL). Same on the Pixel 6 Pro.
- **Plan:** use AAudioService / AudioFlinger over rsbinder. The HIDL
  audio HAL is AudioFlinger's concern, not the app's.

### Microphone (capture)
- **HAL:** the audio HAL (HIDL) handles mic input; `soundtrigger`
  (hotword) HAL is also **HIDL**.
- **Framework:** capture uses the same AIDL `media.audio_flinger` /
  `media.aaudio` path as playback (the `AudioRecord` / AAudio-input
  direction).
- **Plan:** rsbinder → AudioFlinger/AAudio for a capture stream.
  Requires the `RECORD_AUDIO` runtime permission. Hotword/always-on
  trigger via the soundtrigger HAL is HIDL → out of reach, but normal
  mic capture is not.

### Camera
- **HAL:** `android.hardware.camera.provider` — **AIDL** (v3,
  `ICameraProvider/internal/0`). Reachable.
- **Framework:** `media.camera` (`ICameraService`) — **AIDL**.
- **Plan:** camera is AIDL end-to-end on this device — fully
  rsbinder-reachable. Gated by the `CAMERA` runtime permission.
  This is the cleanest of the four (no HIDL anywhere in the stack).

---

## rsbinder guidance

1. Target the **AIDL framework services** (`service list` entries), not
   vendor HALs — that is the supported, transport-correct path and
   covers all four areas.
2. The blocker on a non-rooted device is **SELinux + runtime
   permissions**, not the binder transport:
   - `untrusted_app` is denied many system/vendor services outright
     (SELinux AVC denial → service lookup fails or `EX_SECURITY`).
   - Permission-gated calls need the runtime permission granted
     (`CAMERA`, `RECORD_AUDIO`) or are simply unavailable
     (privileged telephony).
3. HIDL only matters if a future task wants to bypass the framework and
   call the `audio` / `radio` / `soundtrigger` HAL directly — rsbinder
   cannot do that (it would need an `hwbinder` client, which rsbinder
   is not). No current plan requires it.
4. The Pixel 2 XL has *more* HIDL HALs than the Pixel 6 Pro; code
   written against AIDL framework services is portable across both,
   whereas anything reaching for a HAL directly is device-specific.
