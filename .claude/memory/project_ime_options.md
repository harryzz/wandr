---
name: project-ime-options
description: "Soft-keyboard / IME options for wandr given the \"total remove Java\" constraint — how Android IME actually works and which path fits"
metadata: 
  node_type: memory
  type: project
  originSessionId: be47cfff-188f-4f12-989d-c09046736d6a
---

Context for the IME / soft-keyboard question in the post-ART wandr
runtime. User constraint: **total remove Java** → no JNI to
`android.view.inputmethod.InputMethodManager`. Standing decision
(2026-05-26): **lean on the in-canvas Compose keyboard; treat real-IME
via rsbinder as roadmap, not next.**

## Why "no Java" doesn't mean "no IME"

Android's IME framework is **binder-based on the wire**. `InputMethodManager`
is a Java *façade* in the app process; the actual protocol is AIDL on
`IInputMethodManager`, `IInputMethodSession`, `IInputMethod`,
`IRemoteInputConnection`, `IInputMethodClient`. rsbinder (already used
for HALs in tasks 15–22) can speak all of these. The framework just
*happens* to live in Java.

## Five actors in the real Android IME flow

```
App (client) ──IInputMethodManager──► system_server (IMMS)
   │ ◄────IInputMethodClient──────────  │
   │                                    │ binds + dispatches
   │ ◄──IInputMethodSession ── IME APK (Gboard etc.) ─┐
   └─────IRemoteInputConnection ─────────────────────┘
```

Touches in the keyboard slot route to the IME (separate SurfaceFlinger
layer named `InputMethod` — visible in `dumpsys input`); commits come
back into the app as `IRemoteInputConnection.commitText("a")` /
`sendKeyEvent(...)`, **not** as InputFlinger events.

## Four options for wandr

| Option | Java-free | Effort | Outcome |
|---|---|---|---|
| **A. In-canvas Compose keyboard** | ✅ | days (polish only — already works) | ASCII / basic editing; deterministic; no system dep |
| **B. rsbinder → IMMS** | ✅ | 1–2 weeks; high ongoing maintenance | Real Gboard/voice/emoji/CJK/swipe |
| **C. Spawn an IME as a 2nd wasi guest** | ✅ | very large | Best long-term post-ART fit |
| **D. Hardware-keyboard only** | ✅ | 0 | What ships now |

## Recommendation (current standing decision)

**Default = A; (B) on the roadmap but not next; (D) covers the hardware
case; (C) is the post-ART north star.**

Rationale:
- **(A) is already device-verified** via [[feedback-softkeyboard]] —
  tap → in-canvas keyboard → commits via the same `on-key-event-v2`
  WIT call task 33 wired for hardware keys. Needs Shift toggle +
  numeric layout + maybe long-press accents to be "good enough." Days,
  not weeks.
- **(B) is technically possible** — you already vendor AIDLs for HALs
  — but the IMMS surface is much bigger and AIDL-unstable than any HAL
  shipped so far. `IRemoteInputConnection` alone is ~30 calls; IMMS
  has the highest AIDL-churn rate of any system service (Android 12,
  13, 14 each reshaped it). Maintaining it across versions is ongoing
  pain.
- **(B) assumes** the IME framework accepts a non-Activity client with
  a non-WindowToken input target — possible but unsupported edge case
  (focus, ime-on-top z-order, configChanges all need hand-rolling).
- **(C) is the architectural fit** for "the app *is* the wasi guest" —
  CJK input belongs naturally to a wasi-guest IME, not a binder
  bridge into the framework. But it's year-out work (port an IME to
  Compose-wasi, share a canvas / coordinate input routing).

## Concrete near-term: polish (A)

Smaller than a formal task — just track:
- Shift toggle (caps / latch) — Compose state, ~hour
- Numeric / symbol layout switch — Compose state machine, ~half-day
- Long-press accent menu — needs delay + popup; depends on
  [[popup-overlay]] resilience
- Show/hide via Compose's `SoftwareKeyboardController` instead of an
  explicit toggle button

Out of scope for (A): voice, emoji, CJK, autocorrect, swipe-typing —
accept these as "needs B or C" limitations.

## If (B) ever lands, the surface to vendor

(For future-me; don't start without re-deciding):
- `IInputMethodManager.aidl` + `IInputMethodClient.aidl` (registration / focus)
- `IRemoteInputConnection.aidl` (editor model — biggest, ~30 calls)
- `IInputMethodSession.aidl` (IME → app notifications)
- `EditorInfo.aidl`, `InputBinding.aidl`, `InputConnectionWrapper.aidl`
  parcelables
- Pin to one Android version (AOSP 15 on the taimen target) — re-vendor
  per upgrade.
- Fake out the Activity token / WindowToken that IMMS expects (it's
  used as a focus key — opaque to IMMS but must round-trip).

Related: [[feedback-softkeyboard]] (current in-canvas implementation),
[[feedback-ime-options]] (older summary that predates the no-Java
framing), [[project-standalone-keys]] (the hardware-key sibling that
already works).
