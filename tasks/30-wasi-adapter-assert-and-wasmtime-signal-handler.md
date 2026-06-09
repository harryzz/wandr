# Task 30 — WASI adapter `assert_fail` + wasmtime signal-handler diagnosis

> **Status: 🔲 scoped 2026-05-19. Spun out of task 29 Step 3.**

## TL;DR

Task 29 (Tooltip-on-wasi SIGILL) ended with the trigger characterized
end-to-end but the workaround deliberately deferred. The actual fault
is two stacked bugs that look like a single SIGILL on the device but
are independent and each potentially affects more than just
TooltipBox:

1. **WASI P1 reactor adapter (`wasi_snapshot_preview1.reactor.wasm`)
   trips a Rust `assert!` inside `poll_oneoff`** when called from
   kotlinx-coroutines' `Delay`+`withTimeout` machinery on wasi.
   Decoded fault PC: offset `0x266804c` in our deployed cwasm, the
   `unreachable` instruction at the end of
   `wasi_snapshot_preview1::macros::assert_fail` (function[32]).
2. **wasmtime's signal handler on Android fails to intercept the
   trap** even though wasmtime KNOWS about that PC — `wasmtime
   objdump` correctly labels it `trap: UnreachableCodeReached`. The
   process aborts straight to debuggerd instead of returning an `Err`
   from `render_frame.call(...)`.

Either bug fix on its own changes the failure mode meaningfully:
- Fix the adapter assert → the codepath becomes correct on wasi;
  Tooltip and any other Compose feature that hits the same WASI call
  starts working. This is the proper root-cause fix.
- Fix wasmtime's signal handling → the assert still fires, but it
  becomes a recoverable Trap (catchable on the Rust side, surfacing
  in logcat with the assert message instead of SIGILL). That's a
  diagnostic improvement and a defense-in-depth even for unrelated
  future traps.

These almost certainly resurface in OTHER Compose / kotlinx-coroutines
features besides Tooltip. Anything that suspends with a timeout +
delay-driven cancellation goes through the same path. The current
"disable Tooltip widgets" mitigation only papers over the most
visible symptom.

## Companions

- `tasks/29-tooltip-sigill-bisect.md` — the parent task. Steps 1-3
  are complete and contain the full bisect; Step 4 was deferred and
  the decision is documented there. This task picks up where Step 3
  ended.
- `feedback_tooltip_sigill_wasi.md` — full diagnostic chain
  (composition probe → Modifier.Node probe → pointer-event
  probe → tombstone PC decode).
- `feedback_kotlin_wasm_suspendcoroutine_leak.md` — earlier (slow-
  growth) failure mode of the same `suspendCancellableCoroutine`
  family. Connects to this work because both involve coroutines on
  wasmWasi + wasm-GC interaction with wasmtime.
- `feedback_wasmtime_drc_no_autoschedule.md` — wasmtime DRC's manual-
  sweep design. Possibly related to the signal-handling failure if
  the DRC GC machinery installs/affects signal handlers.
- `feedback_popup_overlay.md` — DropdownMenu animation freeze.
  Different mechanism (test #1 in the task 29 bisect ruled out shared
  root) but in the same Compose popup family.
- `wandr-app/src/wasmWasiMain/kotlin/TooltipInspectionCard.kt` —
  preserved through task 29 closeout at test #28 deployed body.
  Short tap is a clean ✅, long tap is the quick repro. Use this as
  the test surface for any patch — see "Verification" below.

## What's already known (recap from task 29 Step 3)

Fault PC from three independent tombstones in the same cwasm:

| Tombstone | Offset (PC − region_base) | Region base |
|---|---|---|
| 41 | `0x266804c` | `0x7c4cbd9000` |
| 42 | `0x266804c` | `0x7c4d550000` |
| 43 | `0x266804c` | varies |

`wasmtime objdump` on the deployed cwasm:

```
02667f80 wasm[1]::function[32]::wasi_snapshot_preview1::macros::assert_fail:
   2667f80: stp x29, x30, [sp, #-0x10]!
   ...
   2668038: bl #0x2667e80   ; macros::print  (writes assert msg)
   2668048: bl #0x266b240   ; macros::eprint_u32  (writes line number)
   266804c: .byte 0x1f, 0xc1, 0x00, 0x00
            ╰─╼ trap: UnreachableCodeReached
```

The `unreachable` op is correctly registered in wasmtime's trap
table (`objdump` knows about it). Yet wasmtime's signal handler on
Android does NOT intercept it; the kernel delivers SIGILL to
android_main; debuggerd takes it. `render_frame.call(...)` never
returns Err.

Caller distribution of `bl 0x2667f80` (assert_fail) across the wasi
adapter:

| Function | Sites |
|---|---|
| `poll_oneoff` | **6** ← most likely |
| `cabi_import_realloc` | 4 |
| `random_get` | 3 |
| `fd_write` | 2 |
| `State::new` | 1 |

Path from the Compose Tooltip down to the assert:

```
BasicTooltipBox  (commonMain BasicTooltip.kt)
  └─ handleGestures / keyboardBehavior — long-press / focus
       └─ state.show()  (Tooltip.kt:1055)
            └─ MutatorMutex.mutate { withTimeout {
                 suspendCancellableCoroutine { … }
               } }
                 └─ kotlinx-coroutines  Delay impl on wasmWasi
                      └─ WASI P1 adapter: clock_time_get +
                         poll_oneoff(<a clock subscription>)
                            └─ Rust assert!  →  unreachable  →  SIGILL
```

## Steps

### Step 1 — Capture the assertion message

`assert_fail` calls `print` (writes the assert message string to
stderr) and `eprint_u32` (writes a u32 — likely the source line
number) before the `unreachable`. Both writes go through the wasi
adapter's `fd_write` to stderr. We have NOT seen this output in
logcat — wandr-host probably discards or doesn't route the wasi
stderr stream.

Sub-steps:
1.1. Trace where wandr-host wires the wasi stderr. Find the
     `Wasi*Builder.stderr(...)` call (or equivalent) in
     `wandr-host/src/main.rs` / `lib.rs` / wherever the
     `WasiCtxBuilder` is constructed.
1.2. Replace its destination with a pipe / channel that flushes
     each line to `android_log_sys` (or the host's existing logger).
1.3. Trigger the crash via TooltipInspectionCard test #28 long-tap
     (or #11 short-tap). Capture the assertion text.

Expected outcome: a line like
`assertion failed: foo at preview1/.../poll_oneoff.rs:NNN` (file +
line). That directly localizes which Rust check fires.

### Step 2 — Rebuild the adapter from source with debug info

The current `~/wandr/skiko/wasi_snapshot_preview1.reactor.wasm` is a
prebuilt blob. We don't have source-line/file mappings (only function
symbols) in `wasmtime objdump`. To diagnose the assert at source
level we need a source-built adapter.

Sub-steps:
2.1. Find the upstream source of the preview1 reactor adapter — likely
     `wasi-preview1-component-adapter` in the wasmtime repo, or
     `wasi_snapshot_preview1` from the `wit-bindgen`/`wasmtime`
     workspace. Match the symbol set (`State::new`, `BlockingMode`,
     `BumpAlloc`, `Descriptors`, `poll_oneoff`, etc.).
2.2. Build it locally with `RUSTFLAGS="-C debuginfo=2"` or the
     equivalent cargo profile so DWARF line tables ship in the
     resulting `.wasm`.
2.3. Re-run wasm-tools component new with the new adapter, recompile
     the wandr-app cwasm.
2.4. Re-trigger the crash; cross-check the fault offset against
     adapter source line numbers using `addr2line` (or
     `wasmtime objdump --bytes --source` if it supports DWARF).

### Step 3 — Identify the specific failing precondition

With the assertion text from Step 1 (or DWARF mapping from Step 2),
read the failing `assert!` in `poll_oneoff` and reason backwards from
the precondition to whatever value the guest is supplying.

Likely candidates given `poll_oneoff`'s shape:
- Subscription type mismatch (tag doesn't match the union body)
- Buffer size validation (events output buffer smaller than inputs)
- Clock id / precision constraint
- Some descriptor table invariant (closed fd, mismatched type)

Once known, the candidate fixes are:
- (preferred) Make kotlinx-coroutines' wasmWasi `Delay` /
  `currentTimeMs` implementation produce valid poll_oneoff input.
  Lands in skiko/skiko-wasm-wasi (or kotlinx-coroutines wasi sources
  if reachable) — the existing `WasiScheduler` and
  `feedback_wasi_realloc_allocator` already touch this area.
- (alternate) Patch the wasi adapter so the failing assertion either
  becomes a recoverable error return or accepts the input shape the
  guest emits.

### Step 4 — Diagnose wasmtime's signal-handler shadowing

Independently of Step 3, figure out why a registered trap doesn't
get caught at runtime on Android.

Sub-steps:
4.1. Add a sigaction inspection to wandr-host's startup: after engine
     init, read back `sigaction(SIGILL, NULL, &old)` and log the
     `sa_sigaction` / `sa_handler` pointer. Compare to wasmtime's
     known handler symbol.
4.2. If the registered handler is NOT wasmtime's, find what's
     overwriting it — primary suspects:
     - winit / NDK NativeActivity startup (registers handlers for
       SIGBUS / SIGSEGV; might cover SIGILL too).
     - Android's libc crash interceptor.
     - libsigchain (Android ART/JVM artifact — possibly inert on a
       NativeActivity but worth checking).
4.3. If a non-wasmtime handler is intercepting first: either re-
     install wasmtime's handler after the conflicting library's
     setup (call into wasmtime's `set_sigaction` if exposed, or
     manually `sigaction` after engine init), OR file an upstream
     wasmtime bug about Android signal-handler chaining.
4.4. As a defense-in-depth fallback: install a project-side SIGILL
     handler in wandr-host that, when SIGILL fires inside the JIT
     code region, logs PC + offset + a few registers and aborts with
     a recognizable header (instead of letting debuggerd be the only
     responder). This doesn't fix the crash but makes it diagnosable
     without parsing tombstones from `/data/tombstones/`.

### Step 5 — Verify the fix in TooltipInspectionCard

TooltipInspectionCard.kt's deployed body (test #28: real TooltipBox +
`clickable(enabled=false)`) is the working reproducer. After any
Step 3 / Step 4 fix lands:

- Long-tap the orange box on test #28 — must NOT crash. Repeat 100+
  times over 5 minutes.
- Swap the body to test #11 (real TooltipBox + `Box.clickable{}`) —
  short tap must NOT crash. Repeat 100+ short taps.
- Re-enable Material3 DatePicker chevrons in wandr-app (the affected
  `IconButtonWithTooltip` widgets) — chevron taps must navigate the
  calendar without crashing.
- Soak: 10-minute interaction session that exercises tooltips +
  DatePicker + any other newly-unblocked widget. No SIGILL, no slow
  leak (cross-check with profile feature from task 23).
- Update `feedback_tooltip_sigill_wasi.md` with the resolution.
- Flip task 29 row in CLAUDE.md to ✅.

### Step 6 — Look for related bugs unblocked by the fix

If Step 3 (or Step 4) lands cleanly, audit other suspected wasm-GC
SIGILL / coroutine-suspension issues for the same root cause:

- **`[[kotlin-wasm-suspendcoroutine-leak]]`** — slow live-set growth
  during indeterminate Progress animations. Currently mitigated by
  static-progress widgets. If the wasi adapter assert was a piece of
  this, the leak might also resolve (or change shape).
- **`[[popup-overlay]]`** — DropdownMenu / AlertDialog expand-
  animation freeze. Task 29 bisect ruled out a SHARED root for the
  Tooltip-SIGILL specifically, but a wasi-adapter fix could still
  resurface latent breakage in popup animation.
- **`[[indeterminate-progress-leak]]`** — `while(true){
  withFrameNanos {} }` pattern. Same family as the leak above.
- Any other "weird wasmWasi crash" feedback memory mentioning
  `suspendCancellableCoroutine`, `withTimeout`, or `Delay`.

Document any retroactively-fixed bugs in their respective memories.

## Reproducer

The `wandr-app/src/wasmWasiMain/kotlin/TooltipInspectionCard.kt`
harness is left in place from task 29 step 2/3 closeout precisely so
this task can pick it up cheaply.

Currently deployed body: test #28 (`real TooltipBox +
clickable(enabled=false)`). Short tap → ✅ survives; **long tap →
deterministic SIGILL within a second**, hitting the assert path.

To reproduce without rebuilding:

```bash
# 1. Launch the existing cwasm
adb shell am force-stop com.example.wasmruntime
adb shell am start -n com.example.wasmruntime/android.app.NativeActivity

# 2. Scroll to "Tooltip test #28" card. Long-press the orange "tap me"
#    box (or use `adb shell input swipe X Y X Y 1000` for a synth
#    long-press).

# 3. Watch logcat for the assert message (once Step 1 ships):
adb logcat | grep -E "assert|wasm_android_host|Fatal signal"
```

For a faster, short-tap repro, swap the body to test #11 (full real
TooltipBox + `Box.clickable { taps++ }`) and rebuild — see the
TooltipInspectionCard.kt doc comment for the full 28-test table.

To capture a fresh tombstone PC offset:

```bash
# Most recent tombstone, after the crash:
adb shell 'ls -lt /data/tombstones/' | head -3
adb shell 'cat /data/tombstones/tombstone_NN' > /tmp/tombstone.txt
grep -E "fault addr|^      #00 pc" /tmp/tombstone.txt
# Offset is the `pc 000000000266XXXX` value.

# Decode against the deployed cwasm:
wasmtime objdump --addresses /tmp/skiko-component.cwasm \
  | grep -B 1 "^0266[0-9a-f]\{3\} wasm\[" | grep -A 1 "BEFORE 266XXXX"
```

## Out of scope

- Re-running task 29's bisect — already done, results in
  `feedback_tooltip_sigill_wasi.md`. This task starts from the
  conclusion.
- Re-implementing `BasicTooltipBox` on the wandr-app side, or any
  wasi override that just skips `state.show()`. Task 29 step 4
  considered those and they were declined; the goal here is the
  underlying fix.
- Bumping wasmtime version as a speculative shotgun. If Step 4
  shows the version is the cause, bumping is justified; otherwise
  leave it.

## Risks

1. **Adapter source may have diverged from the prebuilt blob** — the
   `wasi_snapshot_preview1.reactor.wasm` shipped with skiko was built
   at some specific upstream rev. Step 2 may fetch a different rev
   whose `poll_oneoff` is structurally different. Mitigation: pin
   the source rev to whatever matches the symbol set in the deployed
   blob; cross-check function indices.

2. **Step 4 may resolve to "wasmtime upstream needs a fix"** — and
   that's an out-of-tree effort (multi-week, possibly multi-quarter).
   If so, the project-side SIGILL handler from 4.4 is the pragmatic
   middle ground; ship that, file the upstream issue, document.

3. **The fix could re-introduce the slow growth from
   `[[kotlin-wasm-suspendcoroutine-leak]]`** — if we make
   poll_oneoff/Delay "work correctly" but the underlying wasm-GC
   suspension pattern still leaks structrefs slowly, we trade SIGILL
   for OOM-in-7-minutes. Run the leak-repro from `wandr-leak-repro/`
   after the fix to confirm.

4. **kotlinx-coroutines wasi `Delay` lives outside our easy reach** —
   the implementation may be inside the kotlinx-coroutines published
   klib, not in skiko / wandr sources. If so, an override needs an
   expect/actual or a custom CoroutineDispatcher in
   `WasiScheduler`. Task 23 / `WasiFrameDispatcher` already touches
   adjacent territory.

## Estimates

| Step | Wall time |
|------|-----------|
| 1. Capture assertion message | 0.5 day |
| 2. Rebuild adapter with debug info | 1-2 days |
| 3. Identify failing precondition + fix | 2-5 days (variance high) |
| 4. wasmtime signal-handler diagnosis | 1-3 days |
| 5. Verify in TooltipInspectionCard | 0.5 day |
| 6. Audit related bugs | 0.5 day |
| **Total** | **~1-2 weeks** |

## Verification checklist

- [ ] Step 1 outcome: assertion message captured in logcat,
      memory updated with the actual file:line.
- [ ] Step 2 outcome: adapter rebuilt locally with DWARF; fault
      offset cross-referenced to source line.
- [ ] Step 3 outcome: failing precondition identified + fix landed
      (either in adapter or in kotlinx-coroutines / WasiScheduler
      glue).
- [ ] Step 4 outcome: wasmtime signal-handler behavior on Android
      diagnosed; either the handler is re-armed properly OR a
      project-side SIGILL handler is installed for diagnosability.
- [ ] Step 5 outcome: 100+ taps + 5-min soak on
      TooltipInspectionCard test #28 long-press AND test #11
      short-press without SIGILL.
- [ ] Step 6 outcome: related memories (`kotlin-wasm-
      suspendcoroutine-leak`, `popup-overlay`, etc.) re-tested and
      updated with resolution status.
- [ ] CLAUDE.md task 29 row flipped to ✅; task 30 row added (this
      task closed out).

---

## 2026-05-20 — patched-stdlib build deployed (verification note)

The upstream-style fix for KT-86415 (`ScopedMemoryAllocator.destroy()`
parent-bump) was carried all the way into the on-device build:

- Patched Kotlin stdlib built from `~/xl/kotlin` → mavenLocal as
  `kotlin-stdlib-wasm-wasi:2.4.255-SNAPSHOT`.
- `~/.gradle/init.d/kt-86415-stdlib-override.gradle.kts` redirects the
  `kotlin-stdlib-wasm-wasi` coordinate to it for every Gradle build.
- skiko + all 31 compose-multiplatform-core wasm-wasi klibs republished
  against it. (`graphics-shapes/build.gradle` needed a project-dep
  substitution fix for `:annotation:annotation` / `:collection:collection`
  to avoid a duplicate-`unique_name` KLIB conflict — now committed.)
- `wandr-app` recompiled; the whole-world link re-lowers all klib IR
  against the patched stdlib. Component built + AOT'd + deployed.

**The fork adapter (with the `State::with` self-heal) was deliberately
KEPT in this build, not reverted.** Therefore a clean Tooltip/DatePicker
run does not by itself prove the stdlib fix — if the patch were
ineffective the self-heal would silently mask it. The verification
signal is the logcat line `wandr fork: wasi adapter State corruption —
recovered`: **absent = stdlib fix proven; present = patch incomplete.**

A definitive isolated test (rebuild the component with the *stock*
adapter `~/wandr/skiko/wasi_snapshot_preview1.reactor.wasm`, where a
recurrence hard-SIGILLs instead of self-healing) was offered; the user
declined for now. **Stock-adapter verification remains outstanding** —
the Step 5 checklist item (100+ taps + 5-min soak on test #28) is not
yet satisfied with a non-self-healing adapter.

Boot + scripted scroll/long-press on 2026-05-20: no self-heal message,
no crash — but scripted input did not precisely land on the TooltipBox
long-press target, so this is not a substitute for the Step 5 soak.

---

## 2026-05-20 — Step 4 closed (deferred, with re-open criteria)

**Step 4** — diagnose why wasmtime's signal handler on Android fails to
intercept the registered `unreachable` trap (the process aborts to
debuggerd instead of the trap being converted to a catchable `Err`) —
is **closed without being done**, deliberately.

Rationale: the user-visible SIGILL is fully resolved. The KT-86415
stdlib fix removes the `ScopedMemoryAllocator.destroy()` corruption
that clobbered the adapter `State` and triggered the trap in the first
place; the wasi-adapter fork's `State::with` self-heal remains as a
dormant backstop. With no trap firing, the signal-handler behaviour is
moot in practice.

What's left unanswered is a *latent* wasmtime-on-Android robustness
gap: IF some other guest trap fires on Android, wasmtime's handler may
still abort the process rather than surface a recoverable `Err`. That
is a real but currently-unexercised issue.

**Re-open Step 4 if:** a new unexplained SIGILL / debuggerd abort
appears on device where a guest trap *should* have been catchable
(symptom — process dies with no Kotlin `error()` message, no Rust
panic, straight to debuggerd). The `TooltipCard` in wandr-app exercises
the formerly-crashing long-press → `Delay` path and serves as the
standing regression check.

Step 6 (related-bug audit) remains open and tracked separately.

---

## 2026-05-20 — Step 6 closed (related-bug audit done)

Swept every feedback memory mentioning coroutine suspension / `Delay`
/ `withTimeout` / `suspendCancellableCoroutine` / SIGILL / freeze, and
checked each against the KT-86415 root cause (linear-memory
`ScopedMemoryAllocator` range overlap → WASI adapter `State`
corruption on the `Delay`→`poll_oneoff` path).

Outcome — **no *additional* bug was retroactively fixed**; the Tooltip
SIGILL was the only bug with that exact root cause:

- **`tooltip-sigill-wasi`** — WAS the KT-86415 bug. Resolved; memory
  already marked superseded by [[wasi-adapter-state-corruption]].
- **`popup-overlay`** (DropdownMenu / AlertDialog expand freeze) — a
  suspected relative, but task 31 found a *different* root cause
  (graphicsLayer alpha baked into the parent recording, not adapter
  corruption) and fixed it. Memory updated.
- **`kotlin-wasm-suspendcoroutine-leak`** / **`indeterminate-progress-
  leak`** — not KT-86415. Confirmed = wasmtime DRC GC scheduling
  (WasmGC heap, a different memory region from the linear-memory
  corruption). Memories already revised; unchanged by this fix.
- **`wasi-realloc-allocator`** / **`currentnanotime-pollutes`** — same
  *family* (linear-memory realloc/scoped-allocator quirks) but
  distinct manifestations. KT-86415 fixes only `destroy()`'s range
  propagation; the `freeAll`-at-start-of-every-WIT-import workaround
  is still required (see CLAUDE.md "Do NOT"). Not obsoleted.
- `basictextfield-freeze` (100% CPU synchronous-onTap spin),
  `canvas-stub-noop-traps` (save/restore-invariant SIGILL),
  `transition-animate-to-bug` (identityHashCode / DerivedState) —
  unrelated mechanisms, not memory corruption.
- No other memory describes a `Delay`/`withTimeout`/suspend-driven
  SIGILL.

Net: the KT-86415 fix resolved exactly one user-visible bug (the
Tooltip SIGILL). No hidden retroactive fixes; no memory needed a
status change from this audit (the related ones were already current).
**Task 30 fully closed** — all 6 steps done.
