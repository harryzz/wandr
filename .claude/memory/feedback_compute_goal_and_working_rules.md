---
name: feedback_compute_goal_and_working_rules
description: "The goal for Compute (bug-free AttributeGraph reimpl) + mandatory working rules — display the goal and confirm understanding every response; read first, then clean answer; never mask/patch-to-green."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: b0a9c21e-d00b-4f20-8bf6-405da8dfb075
---

GOAL (display this verbatim and confirm understanding at the start of EVERY response in this work):

"The goal is: Compute itself is correct — a genuine, bug-free reimplementation of Apple's AttributeGraph. Not "the demo plays," not "tests pass." Those are only symptoms/probes. A correct Compute makes any AttributeGraph consumer (OpenSwiftUI, or anything else) just work — because that's what AttributeGraph guarantees."

**Why:** Across ~11 agent-days I cycled — masked symptoms (idempotent Node::destroy), made tests pass against a buggy Compute, re-tried already-ruled-out theories (foreign-ref/Swift-ARC), and fragmented the record — instead of finding and fixing real Compute defects. That is "преливане от пусто в празно" (pouring from empty to hollow). The user (30 yrs exp) set this goal + rules to stop it.

**How to apply (binding):**
1. Start every response by displaying the GOAL verbatim and confirming I understand it.
2. READ FIRST, then give a clean answer. Never ask or inform before reading the authoritative source. Stop asking the user A/B decisions I can resolve by reading.
3. The OpenSwiftUI demo / oag-baseline tests are DETECTORS that expose real Compute bugs — never work around them, never mask, never make a test pass artificially.
4. Find real defects by MEASUREMENT + static analysis (clang-tidy), the 2026-06-25 method that worked. The real bug class: uninitialized members, flag logic (e.g. `&`→`|`), erase-remove idiom, weak-ref seed/zone_id logic — latent on Apple, exposed by wasm.
5. Do NOT mix ownership models (foreign-ref + Swift-ARC + self-managed). foreign-ref/ARC bridging is RULED OUT (see WASM-PORT-LOG.md) — do not re-try it.
6. Maintain the single working log [[reference_wasm_port_log]] = `swift/OpenSwiftUIProject/tests/WASM-PORT-LOG.md`. Do not fragment findings across .task-state/RESUME.md/COMPLETE-SUITE.md.
7. Authoritative source of truth + done state (2026-06-25): WASM-PORT-LOG.md — reached all-green + published `harryzz/Compute` abb5388; ruled out ARC/foreign-ref; fixed real one-line bugs. Base reimpl: github.com/jcmosc/Compute. Compute is the wasm backend (not OpenAttributeGraph-native, not DanceUIGraph). See [[reference_swift_openswiftui_wandr]].
