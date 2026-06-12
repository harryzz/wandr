# wasi:input-handlers 0.0.2 — union check + fixes (NOT WIRED)

> Proposal only (2026-06-12). Companion to
> `proposals/wasi-canvas/REDESIGN-0.0.2.md`, bound by the same rules
> (R1–R5 there) and by the same user-fixed GOAL: **architecturally
> clean, without overlapping functionalities, acceptable to WASI, and
> 100% consumable by the reference UI libraries** (final check: §last). This is the six-consumer union check the 0.0.1 events
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

**Multi-touch semantics (normative, W3C Pointer Events model):** each
contact is an independent event stream correlated by `id` — host-
assigned, stable for the contact's lifetime, reusable after up/cancel;
`0` = first contact / the mouse; `enter`/`leave` apply to hovering
devices. There is deliberately NO snapshot event carrying all active
pointers: snapshot-camp frameworks (Compose's PointerEvent.changes)
assemble one guest-side from the id map — derivable (R5), the same
adapter every browser-hosted framework runs on W3C pointer events, and
it keeps the wire at one contact per event instead of N× per move.

**Named deferrals**:

| Deferral | Why | Re-entry lane |
|---|---|---|
| Trackpad gestures (pinch/rotate — Flutter PointerPanZoom*, egui Zoom/Rotate, Avalonia magnify) | produce NO touch points on any platform — not expressible as pointer streams, and not pointer fields either | **designed below** as the optional `gesture-handler` interface; ADDITIVE (new interface), so it lands whenever a host grows trackpad support, never as a version event |
| W3C contact-ellipse width/height, tangential-pressure | no consumer of the six meaningfully consumes them | version event (R3) |

## Adoption

Rides the canvas-0.0.2 event (path B in the stage-4 sequencing): hosts
serve 0.0.1 + 0.0.2 side-by-side; shipped consumers migrate lazily.
After this bump, both packages are frozen for all six consumers —
everything further is additive (R2) or a deliberate version event (R3).

## `gesture-handler` — designed now, additive whenever (wasm-tools-validated)

```wit
/// OPTIONAL — platform-resolved continuous gestures (trackpad
/// pinch/rotate class). Delivered ONLY for gestures the platform
/// provides no pointer streams for; touchscreen gestures arrive as
/// pointer contacts and are recognized by the GUEST framework (its
/// policy). This exclusivity is what keeps the package overlap-free.
interface gesture-handler {
    enum phase { begin, update, end, cancel }

    record gesture-event {
        phase: phase,
        /// Focal point, surface units.
        x: f32,
        y: f32,
        /// Multiplicative scale delta since the previous event (1.0 = none).
        scale: f32,
        /// Rotation delta in degrees since the previous event.
        rotation: f32,
        /// Pan delta since the previous event, surface units.
        pan-dx: f32,
        pan-dy: f32,
    }

    on-gesture: func(ev: gesture-event);
}

world gesture-handler-world { export gesture-handler; }
```

Shape decisions: one event carries pan+scale+rotation together (the
Flutter PointerPanZoomUpdate shape; egui decomposes it trivially);
per-event DELTAS, not cumulative (cumulative is guest-derivable by
accumulation — R5; the reverse is not). Six-consumer check: Flutter
PointerPanZoom* ✓, egui Zoom/Rotate ✓, Avalonia magnify ✓,
Compose-desktop transformable ✓; Slint/dioxus simply don't export it
(probe-only world).

## Overlap audit + final acceptance check (2026-06-12)

### Overlap audit

| Pair | Verdict |
|---|---|
| `key-event.text` vs the IME slice | NOT overlap — by the package's exclusivity rule (hosts route each input type to exactly ONE bound path); with an editor attached the IME path owns text composition, key-handler delivers raw keys. The rule IS the no-overlap mechanism, stated normatively. |
| `on-resize` vs `canvas.width/height` | notification vs state query — the standard windowing pair, not duplication (chrome guests use the query; Compose uses the push) |
| `kind: scroll` vs a separate wheel handler | one handler, kind-discriminated — no parallel delivery path exists |
| `enter/leave` vs `cancel` | distinct semantics: hover boundary vs gesture abort |
| `pressure` vs `buttons.primary` for touch | W3C-anchored convention documented: touch contact sets the primary flag; both fields kept because pens report pressure with no buttons held |
| `frame-handler` vs a future `wasi:surface` pull profile | the AXIS, not overlap — this package is the PUSH delivery of the shared event vocabulary (surface-convergence proposal); the eventual "extract shared event records" refactor is the named lane, changing packaging, not shapes |
| frame *pacing* (when ticks are wanted) | correctly OUT — scheduling policy, its own optional interface (today my:skiko-gfx/frame-pacing), not input delivery |

### Final acceptance check

| Criterion | Status | Evidence |
|---|---|---|
| Architecturally clean | **PASS** | delivery-only scope (no IME composition, no density/configure policy, no pacing); probe-only worlds per handler; the push half of the two-profile event model, vocabulary shared with the future pull profile |
| No overlapping functionality | **PASS** | audit above; the exclusive-routing rule resolves the one real hazard (keys vs IME) normatively |
| WASI-acceptable | **PASS (design level)** | input devices are in the Subgroup Charter scope; exported handlers follow wasi:http's incoming-handler precedent; guests hold zero ambient authority (pure push); probe-worlds = modularity; interposition = re-exporting wrappers; shapes wasm-tools-validated; the R4 floor covers the export side (big exported records with strings = the proven freeAll→lift path; flags lift exists in the Kotlin generator) |
| 100% reference-library consumable | **PASS with 0.0.2** | Compose: multi-touch/pressure/keys/frame shipped, + desktop buttons/hover now covered; dioxus + Slint: shipped on 0.0.1 shapes, unaffected; Avalonia: RawPointerEventArgs fully expressible (device kind, all five buttons, tilt/twist, leave). Prospects: egui (buttons, PointerGone; pinch-zoom derivable from multi-touch ids — R5), Flutter (PointerEvent kind/buttons/tilt/hover; a11y/semantics is a different layer by design) |

0.0.1's pointer-event fails criterion 4 — which is the whole reason
0.0.2 exists; with it, the package freezes alongside canvas 0.0.2.
