---
name: feedback_change_detection_test_primitive
description: "When the symptom is \"nothing updates / change-detection fails / views don't re-render,\" go straight to the compare/equality PRIMITIVE and test it in isolation — never trace the consumer up the stack."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 50ea929c-30b1-4a02-a37f-3cad3c8adbb2
---

When the bug looks like **"state changes but the UI/graph doesn't update"** —
frozen lists, no reuse, stale renders, "re-creates instead of reuses" — the
governing primitive is the **value-comparison / equality** the engine uses for
change detection (e.g. Compute `compare_values` → `LayoutDescriptor::compare` /
`compare_existential_values` / `compare_bytes`). Go read and **unit-test that
primitive in isolation FIRST.** Do not trace the consumer (OpenSwiftUI ForEach /
DynamicContainer / view-reuse guards) up the stack.

**Why:** a single wrong byte/pointer in a compare function shrapnels into a dozen
unrelated-looking high-level symptoms. Real case (2026-06-27, OpenSwiftUI-on-wasm
2048 freeze): `compare_existential_values` projected `rhs_value` from `lhs`
(`type.project_value((void *)lhs)` twice) → every `any ViewList`/`any View`
compared equal-to-itself → graph never propagated changes → board frozen.
**Cost of not following this: ~16 rebuild-and-trace cycles across a week+, in the
wrong layer (OpenSwiftUI), refuting symptom after symptom** (isValid guards,
transitions, Subgraph.index, render drive) — none of which were the bug. The fix
was one char (`lhs`→`rhs`) in Compute, found only after going DOWN to the compare
code.

**How to apply:**
- Symptom = change-detection failure → suspect the compare primitive, not the consumer.
- Write a 5-line isolation test: do two **different** values (esp. existentials /
  structs holding changing data) compare **unequal**? If not, the primitive is broken.
- Coverage gap to avoid: a Compute/engine test that exercises Subgraph/Attribute
  dataflow but NOT existential value comparison gives FALSE "engine is clean"
  confidence (my `oag-baseline` had exactly this hole).
- Reverting consumer-layer guards and seeing NO change = strong signal you're at
  the wrong layer; drop down, don't keep patching the consumer.

See [[feedback_read_source_first]] (read the governing path first) and
[[feedback_humility_proven_vs_guessed]] (run the falsifying/controlling test first).
Live detail: `repros/openswiftui-wasm/RESUME.md` top section.
