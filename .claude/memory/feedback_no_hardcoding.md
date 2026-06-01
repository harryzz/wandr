---
name: feedback_no_hardcoding
description: "STANDING RULE — do not hardcode magic values. Derive from first principles / real inputs; a constant must be a single named, justified source of truth, and computed/config beats baked-in."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: b4642c38-ac22-459b-92dd-7b4430418889
---

**Do NOT hardcode.** Default to deriving values from first principles or real
runtime inputs, not baked-in magic numbers.

- Prefer **computed** (from real geometry / metrics / counts) over a literal.
  Keyboard height → `rows × comfortable_key_height × density`, or a fraction of
  the *measured* screen size — never a fixed pixel count per orientation.
- Prefer **resolution/device-independent**: read the actual screen / surface /
  density and scale, so it works on any panel — don't assume 1440×2880.
- If a constant is genuinely unavoidable, it must be **ONE named, documented,
  justified single source of truth** (and ideally configurable), not a literal
  sprinkled across call sites.
- Put the value in the **layer that owns the policy**, derived from that layer's
  real needs — not a guess in the host to paper over a guest's fragility.

**Why:** the user repeatedly called this out (task 71, 2026-05-31). I hardcoded
the IME keyboard height (`864`/`640` px), a host `MIN_CONTENT_PX = 500` floor, a
landscape `×11/10` fudge, and `30/42` percents — band-aids with magic numbers
instead of principled designs. The non-hardcoded versions: the IME derives its
size from the **real screen geometry** it reads via `display` (resolution-
independent); the rotation-crash floor belongs in the **guest's own clamp guard**
(`min ≤ max`), not a host magic number; chrome insets come from measured strip
heights, not constants.

**How to apply:** before writing any numeric literal in logic, ask "what real
input is this a function of?" and compute it. If you can't, name it once, justify
it in a comment, and flag it to the user as a knob — don't silently bake it in.
Relptes to [[feedback_clean_library_usage]] (don't leak plumbing / keep it
portable).
