# Task 116 — BUG: IME password keyboard — `123` key dead, special symbols missing

> Reported 2026-07-08 (user, on-device). **Pre-existing — NOT a task-115/p3
> regression** (user-confirmed old). Filed for the record; not yet investigated.

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
