# Task 116 — BUG: IME password keyboard — `123` key dead, special symbols missing

> Reported 2026-07-08 (user, on-device). **Pre-existing — NOT a task-115/p3
> regression** (user-confirmed old). ✅ **FIXED + USER-VERIFIED 2026-07-08** —
> deployed to device; user confirmed the 123 key works and symbols show in a
> real password field.

## Symptoms (Pixel 2 XL, wandr.ime.keyboard)

When an editor with the **password** input-type attaches (observed in the WiFi
app's password prompt):

1. The **`123` layout-switch key does nothing** — cannot reach the numeric
   layout from the password keyboard.
2. **Special symbols are missing** — no path to the symbols layout either, so
   passwords containing digits/symbols cannot be typed.

## Where to start (not yet verified)

- The password editor-type arrives via `wandr:ime` `on-editor-attached(info)`
  (`input_type = password`); layout choice happens in the keyboard app.
  See `[[feedback_ime_layout_arbitration]]` — editor-type override vs the 🌐
  cycle, per-plugin WIT packages.
- Layouts come from the keyboard app + the lang-plugin deps
  (`wandr.lang.bg` / `wandr.lang.fr`); check whether the password layout is a
  special case that skips the numeric/symbols pages or whether the `123` key's
  handler is wired only for the text layouts.
- App source: `apps/system/wandr.ime.keyboard/` (Compose guest).


## Root cause + fix (2026-07-08)

`pickLayout` (`ImeKeyboard.kt`) HARD-returned `"Password"` whenever the editor
type was PASSWORD, ignoring `userRequestedLayout`. So the `123` key set
`userRequestedLayout = "Symbols"` but pickLayout overrode it straight back to
Password — the key was dead and the Symbols page (special symbols) unreachable.
Same for EMAIL/URL (also QWERTY layouts with a `123` key). The code even
documented the behavior as intentional ("matches Android IME") — wrong for
password/email/url, which need digits + symbols.

**Fix:** split editor types into STRICT (Number/Phone — numeric-only, layout
stays locked) vs SOFT (Password/Email/Url — open on their default but the
123/ABC keys must work). New `editorDefaultLayout()` seeds `userRequestedLayout`
to the editor's default on attach (via `LaunchedEffect(currentEditorType)`), so
a password field still OPENS on Password; `pickLayout` then only locks
Number/Phone and honors `userRequestedLayout` for everything else.
`isEditorTypeOverride` (disables 🌐) narrowed to Number/Phone. Files:
`apps/system/wandr.ime.keyboard/src/wasmWasiMain/kotlin/ImeKeyboard.kt`.

**Deploy gotcha:** reinstalling a system-app while the zygote holds its
preloaded cwasm → the forked child SIGSEGVs on the stale mapping
(`[[reference_missing_instance_error_stale_zygote]]`). Bounce the wandr layer
(`run-hybrid-stack.sh --wandr-only`) after `pack-ime-keyboard.sh` so the zygote
re-preloads. (pack-ime-keyboard.sh could do this itself — follow-up.)

## Verify — ✅ DONE (user, 2026-07-08)

WiFi app → secured network → password prompt: tapping `123` shows the Symbols
page; digits + symbols type into the password field. User-confirmed on device.
