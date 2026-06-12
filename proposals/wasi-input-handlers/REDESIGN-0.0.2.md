# wasi:input-handlers 0.0.2 — union check + fixes (NOT WIRED)

> Proposal only (2026-06-12). Companion to
> `proposals/wasi-canvas/REDESIGN-0.0.2.md` and bound by the same rules
> (R1–R5 there). This is the six-consumer union check the 0.0.1 events
> never got — done now, while a version bump can ride the same single
> migration event as canvas 0.0.2, so neither package changes again
> after.

## The check (six consumers, exported-record breaking class)

Event records are EXPORTED (host→guest) — the most freeze-critical
shapes in the system. Checked against: Compose, dioxus, Slint (shipped);
Avalonia `RawPointerEventArgs`, Flutter `PointerEvent`, egui `RawInput`
(prospective).

**`pointer-event` 0.0.1 FAILS the union** — it was shaped by a
touch-phone snapshot (the same class of mistake as "Kotlin stays on
my:skiko-gfx"; the desktop host already has a mouse):

| Gap | Who needs it | Class |
|---|---|---|
| `button` (changed) + `buttons` (held set; host-authoritative — guests miss ups outside the surface, which is why W3C has both) | Avalonia/egui/Flutter (right/middle click, context menus); Compose-on-desktop too | record fields — BREAKING |
| `device` (mouse/touch/pen) | Avalonia, Flutter, Compose InputType | record field — BREAKING |
| `tilt-x/tilt-y/twist` (pen; 0 when unreported, the pressure convention) | Avalonia, Flutter pen support | record fields — BREAKING |
| `enter`/`leave` kinds (hover lifecycle) | egui PointerGone, Avalonia LeaveWindow, Flutter hover, Compose hover exit | enum cases — BREAKING |

Fixed shapes validated with wasm-tools 2026-06-12 (flags ≤ 64 bits and
an exported ~18-field record are inside the R4 binding floor — the
Kotlin generator's FlagsLift and the proven freeAll→lift export path
cover them).

**Judged sufficient as-is (rationale recorded, not gaps):**

- `key-event {code, text}` spans the W3C model: `text` is the
  layout-resolved produced text (= W3C `key` for printables); for
  non-printables W3C `key` is 1:1 derivable from `code`. Flutter
  logicalKey / Avalonia KeySymbol / egui Key all reconstruct from
  these two.
- `frame-handler` (on-frame, on-resize) is universal across all six.
  Density/scale-change events are surface/configure concerns
  (`docs/surface-convergence-proposal.md`), not input.
- IME/composition stays in the platform remainder (the wandr IME
  slice), consistent with the convergence proposal's layering — egui's
  IME events, Avalonia's TextInputMethodClient and Compose's editor
  protocol all sit above this package.

**Named deferrals** (no consumer of the six meaningfully consumes them;
re-entry = version event, R3): W3C contact-ellipse width/height,
tangential-pressure.

## Adoption

Rides the canvas-0.0.2 event (path B in the stage-4 sequencing): hosts
serve 0.0.1 + 0.0.2 side-by-side; shipped consumers migrate lazily.
After this bump, both packages are frozen for all six consumers —
everything further is additive (R2) or a deliberate version event (R3).
