# System theme — Compose MaterialTheme follows Android night-mode

> **Status:** ✅ device-verified 2026-05-26. wart-app's MaterialTheme
> branches dark/light at composition time based on the host's
> `my:skiko-gfx/theme.get-night-mode` (backed by `cmd uimode night`).
> No clipboard companion — that's task 42, deferred.

## What landed

Tiny-WIT addition + small Compose plumbing:

- `wit/skiko-gfx.wit` — new `interface theme { get-night-mode -> enum
  { auto, off, on }; get-accent-color -> u32 }`. Added to skiko-ui
  world.
- `wart-host/src/theme_impl.rs` (new, ~50 LoC) — `impl Host for HostState`
  shells out to `cmd uimode night`, parses "yes"/"no"/"auto".
  Accent returns 0 (Pixel 2 XL is pre-Material-You; defer).
- `wart-app/src/wasmWasiMain/kotlin/ThemeImports.kt` (new, ~30 LoC) —
  hand-written Kotlin bindings.
- `wart-app/src/wasmWasiMain/kotlin/RealComposeApp.kt` — replaces
  hardcoded `MaterialTheme(colorScheme = darkColorScheme())` with a
  `remember` block reading night-mode + picking dark/light scheme.
  Auto → dark (historical default).

## Device evidence

Logcat:
```
theme: read night-mode=NightMode::On (raw="Night mode: yes")
real-compose: system night-mode=ON → scheme-applied
```

Then `adb shell cmd uimode night no` + relaunch:
```
theme: read night-mode=NightMode::Off (raw="Night mode: no")
real-compose: system night-mode=OFF → scheme-applied
```

Screenshots show distinctly different palettes — dark backgrounds
+ light-purple accent vs cream backgrounds + deep-purple accent —
same widget layout.

## Limitations / out of scope

- Read once at composition; live re-theming would need a watcher.
  Acceptable since user rarely flips theme mid-session.
- Accent color not read (Material You needs JNI to Resources or a
  binder call into the theme service). Returns 0 = fallback palette.
- Light theme uses Compose's default Material3 lightColorScheme —
  the accent comes from there, not from the system. Once accent
  pulls from device, this gets richer.

## Related

- `tasks/42-system-clipboard.md` — the companion that DIDN'T land
  this session (rsbinder + ClipData parcelable; 4-6h, deferred).
- post-art-roadmap §3 — Boundary B (HAL/runtime services) the same
  pattern as vibrator/sensors/etc., but via cmd shell-out instead of
  rsbinder for v1.
