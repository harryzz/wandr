# Bug note — keyguard/IME overlay corruption + wart-app audio UI-block + IME audio leak

> Found 2026-06-07 during device verification of the C1/C2/C3 sensor fixes
> (`docs/sensor-access-conflicts-no-art.md`), Pixel 2 XL, `--no-art`. All three
> bugs below are **independent of C1/C2/C3** — they live in the keyguard/IME/overlay
> and wart-app audio paths, none of which the sensor fixes touched. The user's
> reported "wart-app freezes after hours" decomposed into these.

## Ruled out first (what the "freeze" is NOT)
- **Memory leak** — wart-app host RSS is flat (~222 MB, plateaus after warmup; a
  2-min sampler over the lead-up showed no upward slope). `SwapFree` constant.
- **OOM** — no `lowmemorykiller`/OOM in `dmesg`.
- **Hang/wedge** — when "unresponsive," the wart-app process is alive, threads idle
  (`hrtimer_nanosleep`), main thread NOT stuck in `binder_ioctl`, ~0–8 % CPU. It's
  sleeping waiting for input, not deadlocked.
- **C1/C2/C3** — exonerated (see each bug). The pre-C1 revert "fixed" the press-freeze
  only because the reverted wart-app has **no Play button** (bug 2).

---

## BUG 1 (primary) — keyguard lock with an active editor+IME corrupts the arbiter foreground/IME/editor state — OPEN, root-caused
**Symptoms**
- (A) IME keyboard renders **on top of the lockscreen** (screenshot: keyguard clock
  visible with a full QWERTY drawn over it, no "swipe up to unlock").
- (B) After unlock, the foreground app (wart-app) has **no touch/scroll**; a *fresh
  launch* fixes it, a full Background→Foreground round-trip does **not** (the role
  transition fires + the host re-sets its `sf_surface` input region, but touch stays
  dead). Other apps (Signal, dioxus) are unaffected.

**Evidence (arbiter `list` after an idle keyguard lock):**
```
com.example.wart-app … [editor:text]        ← editor still focused, but NOT [fg] (demoted)
war.ime.keyboard     … [fg] [ime]           ← IME marked FOREGROUND (wrong → draws on top)
war.keyguard         …                       ← present, NOT [fg]
```
Stable across re-lists. (SF z-order couldn't be dumped — the hosts own SurfaceControls
directly; the arbiter state is authoritative here.)

**Root cause:** the keyguard lock handler (`wart-arbiter-keyguard`) demotes the
foreground **app** → Background and shows the keyguard surface, but does **not**
(a) hide the **active IME overlay** or (b) clear the **editor focus**. The IME overlay
is left `[fg]`/visible (its render layer sits above the keyguard → keyboard over
lockscreen), and the foreground/focus/editor model stays corrupt — so on unlock the
foregrounded app's input routing isn't cleanly restored (token/window/focus tangled),
hence the touch-loss that only a fresh process resets.

**Fix direction:** on lock — hide the active IME (clear set-ime / hide the IME overlay),
clear or suspend the focused editor, and make the keyguard the top foreground surface
(it must supersede the IME in the fg/visibility model). On unlock — restore the prior
app→Foreground and re-resolve its editor/IME/input cleanly. Confirm next repro with
wart-inputflinger logging turned up (window block + pid→token resolution at dead-touch).

---

## BUG 2 — wart-app "Play Tone" button blocks the render/UI thread (synchronous audio) — OPEN
This button is the **audio-output verification aid added during this session**
(`apps/user/wart-app/.../RealComposeApp.kt`, `playToneAndRelease()` + `PlayToneCard`,
in the un-landed wart-app change), **not** a C1/C2/C3 fix.

**Symptoms:** each Play press freezes the UI **2–3 s**; pressing it **twice** wedges
wart-app input (unresponsive, only fresh launch recovers); sometimes no sound.

**Root cause:** `playToneAndRelease()` runs `createTrack` / `writePcmF32` / `start`
(and the deferred `close`) **synchronously on the Compose render/UI thread** (the
`onClick`). These are blocking binder calls to audioserver; under contention they
stall the render thread for seconds. Blocking the UI mid-gesture/frame corrupts
Compose's input/gesture state → after the 2nd press, dispatch is dead even though the
loop is otherwise idle. Worsened by bug 3 (the IME hogging the exclusive-MMAP
endpoint → `createTrack` contends; `dumpsys media.aaudio` showed `XRuns` piling up).
Confirmed balanced after the fact (`ExclusiveOpenCount=3 / CloseCount=3`, 0 leaked) —
so it's the *blocking*, not a stuck stream.

**Fix direction:** do the audio off the render thread (worker/coroutine), keep the
`close`; **or drop the button** (it was only a verification aid — recommended).

---

## BUG 3 — pristine IME leaks an AAudio MMAP stream on startup — OPEN (blocked by task 30)
**Symptom:** after the IME launches, audioserver pumps ~8 % CPU continuously and holds
the **exclusive MMAP endpoint** (which aggravates bug 2).

**Evidence:** `dumpsys media.aaudio` AAudioClientTracker shows the **IME** (its pid)
owning 1 MMAP stream; wart-app owns 0. The IME's leftover task-21 `android-audio smoke`
does `createTrack` + `start` and **never closes** the track.

**Root cause:** leftover startup audio-smoke in the IME guest (`Main.kt`), same as the
one removed from wart-app — but it **cannot be removed from the IME**: any source edit
to that guest trips the **task-30 wasi-adapter State corruption** (SIGILL in
`kotlin.wasm.internal.KProperty1ImplBase.get`), so only the byte-for-byte pristine IME
runs. See `docs/sensor-access-conflicts-no-art.md` history / task 30.

**Fix direction:** remove the IME startup audio smoke — **blocked** until the task-30
guest-edit fragility is resolved. Interim: kill the IME to clear the leak (loses the
keyboard), or live with the ~8 % pump.

---

## Relationship
Bugs 2 and 3 are an **audio cluster** (IME hogs the HW endpoint → wart-app's blocking
Play-button stalls worse). Bug 1 is the **keyguard/IME/editor state** corruption and is
the real cause of the "unresponsive after idle" the user chased. The two presented
together because the test app had a focused text field (IME up) when the device idle-locked.
