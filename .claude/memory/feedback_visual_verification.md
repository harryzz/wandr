---
name: feedback-visual-verification
description: "For subjective/visual outcomes (scroll smoothness, animation, render quality, fps feel) set up the test and ask the user to inspect — don't claim 'verified' from CPU/logs alone"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 2b58a1a7-2e85-4748-b34a-e9b89ab2de87
---

When a change's success depends on how it **looks or feels** — scroll smoothness,
animation fluidity, frame pacing, render correctness, fps "feel" — do NOT declare
it verified from CPU numbers, logcat, or process state alone. Those prove the
mechanism is *active*, not that the result is *good*.

**Why:** 2026-05-30, task 64 fps-cap — I reported the cap "device-verified" from
top/logcat, but at 30 fps the user found scrolling not smooth. CPU dropped as
intended (mechanism worked) yet the user-facing result was bad, and I'd run my own
tests without asking them to look.

**How to apply:** for visual/subjective outcomes, (1) build + deploy + set up the
scenario, (2) state clearly what's objectively confirmed (mechanism active, CPU,
no crash) vs what needs eyes, (3) hand the user concrete steps to inspect (e.g.
"launch X, scroll the list at fps=30/60, tell me which feels smooth") and wait for
their judgment before calling it done. Default the risky knob conservatively (e.g.
fps cap default 60, not 30) until they confirm. Related: [[reference-on-demand-rendering]].
