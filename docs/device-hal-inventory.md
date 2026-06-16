# Device HAL inventory — HIDL vs AIDL across wandr test devices

> Discovery memo, 2026-06-16. Captures the **HAL transport** (HIDL on
> `/dev/hwbinder` vs AIDL on `/dev/binder`/`/dev/vndbinder`) on each test device,
> because it decides whether a wandr HAL client can be **pure Rust (rsbinder,
> AIDL-only)** or needs a **C++ HIDL shim** (the `wandr-sensormanager` pattern).
> Background: `docs/artless-native-service-model.md`, `tasks/111-native-iradio-client.md`.

## The one rule

A HAL is HIDL on a device because of the **vendor freeze level**
(`ro.vendor.api_level` / `ro.product.first_api_level`) — i.e. *when the
device/chipset launched* — **not** the system/framework version. Vendor HAL
implementations are frozen at launch (GRF); the framework keeps HIDL *client*
support for legacy upgraders. New-chipset HIDL allowance: frozen at A11, →0 new
HIDL at A15, **HIDL transport removed (no hwservicemanager) for A16 launches**.

## Discovery method (reproducible)

```sh
adb shell getprop ro.product.device
adb shell getprop ro.product.first_api_level   # vendor freeze point
adb shell getprop ro.vendor.api_level
# HIDL surface (served = flag "Y" with a pid; "?" = manifest-only, NOT running):
adb shell "lshal 2>/dev/null | grep -E '@[0-9]+\.[0-9]+::' | awk '\$2==\"Y\"'"
# AIDL surface (framework domain, /dev/binder):
adb shell "service list | grep -oE 'android\.hardware\.[a-zA-Z0-9._]+'" | sort -u
# Vendor AIDL HALs live on /dev/vndbinder and are INVISIBLE to service list /
# dumpsys (those query the framework servicemanager). Presence of /dev/vndbinder
# + a HAL absent from both `service list` and the served-HIDL list ⇒ it's an
# AIDL vendor HAL in the vendor domain (e.g. radio on Pixel 6).
```

⚠️ `lshal` lists every interface in the **device manifest** with a `?` status even
when nothing serves it (hwservicemanager can't be reached on A16). **Only `Y` +
pid means actually-running HIDL.** Don't count the `?` rows.

---

## Pixel 2 XL — `taimen`

- System **Android 15** (LineageOS), **vendor frozen at API 26 (Android 8)**.
- **Everything is HIDL.** Confirmed live: `android.hardware.radio@1.4`
  (`android.hardware.radio@1.4-service.legacy` pid 3433 → `rild` pid 1421),
  `gnss@1.0`, `bluetooth@1.0`, `gatekeeper@1.0`, `keymaster@3.0`, sensors (HIDL),
  camera-EIS gyro (HIDL). `/dev/hwbinder` active with a working hwservicemanager.
- This is why wandr carries C++ HIDL shims here: **`wandr-sensormanager`**
  (sensors), the camera-EIS gyro shim, and (scoped) **`wandr-radio`** for IRadio.

## Pixel 6 Pro — `raven`

- System **Android 16 (SDK 36)**, **vendor frozen at API 31 (Android 12)**.
  SIM `LOADED` (telephony active).
- **HIDL is effectively dead.** Only **13 interfaces actually served**, all legacy
  media + libhidl infrastructure — **no functional device HAL is HIDL**:
  - `android.hardware.cas@1.0/1.1/1.2` (broadcast CAS)
  - `android.hardware.media.c2@1.0/1.1/1.2` (Codec2 software store)
  - `android.hardware.media.omx@1.0` (legacy OMX)
  - `android.hidl.allocator@1.0` (ashmem), `android.hidl.manager@1.0/1.1/1.2`,
    `android.hidl.token@1.0`
  - The radio@1.0–1.6 / audio@7.x / composer@2.x / soundtrigger HIDL rows lshal
    prints are **manifest-only `?`** — not served. hwservicemanager returns null;
    `lshal --types=hidl` no longer exists ("for AIDL HALs, see dumpsys").
- **All functional HALs are AIDL** (framework domain, `/dev/binder`): `sensors`,
  `camera.provider` + `frameworks.cameraservice`, `gnss`, `power` + `power.stats`,
  `thermal`, `vibrator`, `lights`, `wifi` + `supplicant`, `gatekeeper`,
  `security.keymint` + `secureclock` + `sharedsecret`, `weaver`, `drm`, `health`,
  `nfc`, `uwb`, `usb`, `neuralnetworks`, `contexthub`, `biometrics.fingerprint`,
  `bluetooth.IBluetoothHci` + ranging/finder, plus Google vendor AIDL
  (`vendor.google.*`: battery, bluetooth_ext, wifi_ext, wireless_charger).
- **Radio is AIDL on `/dev/vndbinder`.** The HIDL radio is manifest-only; the AIDL
  `android.hardware.radio.{network,sim,voice,data,messaging,modem,config}`
  interfaces aren't in `service list`/`dumpsys` (those query the *framework*
  servicemanager) but `/dev/vndbinder` exists and telephony is fully functional →
  the radio HAL is running as an **AIDL vendor HAL**.

---

## Comparison (wandr-relevant HALs)

| HAL | Pixel 2 XL (taimen, vendor A8) | Pixel 6 Pro (raven, vendor A12 / sys A16) |
|---|---|---|
| **radio (telephony)** | **HIDL** `@1.4` (rild + `.legacy`) | **AIDL** (`android.hardware.radio.*`, vndbinder) |
| **sensors** | **HIDL** (needs `wandr-sensormanager` shim) | **AIDL** `android.hardware.sensors.ISensors` |
| **camera** | HIDL (provider + EIS-gyro shim) | **AIDL** `camera.provider.ICameraProvider` |
| **gnss** | HIDL `@1.0` | **AIDL** `android.hardware.gnss.IGnss` |
| **audio** | HIDL `@x` (AudioFlinger path) | AIDL (`audio.core` vendor domain) |
| **composer/graphics** | HIDL `@2.x` | AIDL (`composer3`, vendor domain) |
| power / thermal / vibrator / lights | HIDL/mixed | **AIDL** (all) |
| gatekeeper / keymint | HIDL (`gatekeeper@1.0`, `keymaster@3.0`) | **AIDL** (`gatekeeper`, `keymint`) |
| **served HIDL count** | many (full HIDL device) | **13** (legacy media + hidl infra only) |

## Implications for wandr

- **The HIDL C++ shims are a Pixel-2 (vendor-A8) tax.** On the Pixel 6 Pro — and
  any device with vendor API ≥ 31, especially A16 systems — sensors, camera, gnss,
  **and radio** are all AIDL, so they're reachable by **pure-Rust rsbinder** with
  **no C++ shim, no a-03 builds**. `wandr-sensormanager` / the camera-gyro shim /
  the scoped `wandr-radio` shim would all collapse into plain `wandr-hal-*` crates.
- **Task 111 (IRadio) is far cheaper on the Pixel 6.** Radio is AIDL there →
  reimplement RILJ's slice as a Rust rsbinder client against
  `android.hardware.radio.{modem,sim,network,voice,data,messaging,config}` — no
  HIDL/hwbinder, no C++. Caveat: it's a **vendor HAL on `/dev/vndbinder`**, so the
  client must target that binder device (rsbinder supports a custom device path
  since 0.5; otherwise it's the same AIDL flow as the other `wandr-hal-*` crates).
  The state-machine/PDU work is unchanged; only the transport gets simpler.
- **Caveat — porting the rest of the stack:** the Pixel 6 is `raven` (Tensor/Mali
  GPU, A16) vs `taimen` (Snapdragon 835/Adreno, A15). Moving `--no-art` there means
  re-validating the native-survivor model on AIDL HALs (mostly a *simplification*),
  plus the usual GPU/EGL/Skia + libsf_surface bring-up on a new SoC. Media codecs
  remain partly HIDL (`media.c2`/`omx`), matching what wandr already drives.
- **A16 survivor note:** with hwservicemanager gone, anything still HIDL elsewhere
  would be unreachable — but on A16 there's essentially nothing functional left on
  HIDL, so the `--no-art` survivor set is cleaner (all AIDL on binder/vndbinder).

## Which device to use (Pixel-specific nuance)

The freeze rule (HIDL = vendor launch level) is the rule for **third-party GRF
devices**. **Pixels are special: Google updates the *vendor* partition too**, so a
Pixel's HAL transport tracks the **OS build you run**, not its launch version. Two
consequences, both confirmed above:

- **The Pixel 6 Pro is HIDL-free *because it runs A16*, not because it's old.** By
  the freeze rule a vendor-A12 device "should" still carry A12-era HIDL — but Google
  migrated its Pixel vendor HALs HIDL→AIDL across the A13→A16 updates, and the A16
  *system* removed the HIDL transport (null hwservicemanager). So **any Pixel 6 or
  newer on a current OS build is already AIDL-clean** — no need to hunt a specific
  "A15-launch" device.
- **The Pixel 2 is the outlier**, not the norm: Google **froze its vendor at
  Android 8 and stopped updating it**, so LineageOS A15 runs a new *system* on top
  of frozen-A8 *vendor* blobs it can't re-AIDL → full HIDL. It is the **only** class
  of device that forces the C++ HIDL shims.

**Pixel launch versions** (for picking a "born-AIDL" device):

| Pixel | Launched on | Notes |
|---|---|---|
| Pixel 6 / 6 Pro | Android 12 | now AIDL-clean on A16 (vendor updated) — your current test unit |
| Pixel 7 / 8 | Android 13 / 14 | AIDL |
| Pixel 9 / 9 Pro | **Android 14** (Aug 2024; A15 came later as an update) | AIDL |
| **Pixel 9a** | **Android 15** (Apr 2025) | the *only* Pixel that launched on A15 |
| Pixel 10 series | **Android 16** (Aug 2025) | binder-only by design |

Takeaway: for wandr's AIDL/pure-rsbinder path, **the Pixel 6 Pro already on hand is
a fine target** — and anything Pixel 6→10 on a current build works. Don't optimize
for "launched with A15"; optimize for "running a current OS build" (which kills the
HIDL transport and gives Google's AIDL vendor HALs).

## Bottom line

> HIDL on a device is driven by **(vendor launch level) for GRF devices** but by
> **(the OS build you run) for Pixels** — Google updates Pixel vendor HALs.
> Confirmed empirically: taimen (frozen vendor A8) = full HIDL; raven (vendor A12 but
> *running A16*, vendor updated) = HIDL dead except legacy media. **A current-OS
> Pixel 6+ ⇒ AIDL everywhere ⇒ pure-rsbinder, no C++ HIDL shims.** For telephony
> specifically, the Pixel 6 Pro turns task 111 from "C++ HIDL shim + Rust" into
> "Rust-only AIDL client."
