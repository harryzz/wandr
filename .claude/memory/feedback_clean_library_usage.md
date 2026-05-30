---
name: feedback-clean-library-usage
description: "IMPORTANT RULE: when consuming a library keep the app/consumer code clean — depend only on the library's public API; never leak backend/plumbing into app code; if you must go outside the lib's scope, ASK FIRST"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 2b58a1a7-2e85-4748-b34a-e9b89ab2de87
---

**IMPORTANT — read every session.** When building something on top of a
library/framework, keep the consumer (app) code **as clean as possible**: it
should depend **only on that library's public API** and contain only its own
domain logic. Do NOT let the library's backend/plumbing leak into the app.

**Why:** 2026-05-30, the dioxus guest was a mess — the app's `lib.rs` carried
`wit_bindgen::generate!`, a hand-copied `.wit` file, a `HostSink` adapter, the
`Guest` impls, and `export!`. That's the *library's* component/WIT/backend
plumbing living in the app, which makes the app non-portable (can't recompile for
another target without editing source) and duplicated across every guest. The
user's standing requirement: **an app written against the lib must compile for
another target with no source change, depending only on the library.**

**How to apply:**
- Put all backend/target/codegen glue in the **library** (use a macro that
  *expands* in the consumer crate when symbols must land in the final artifact —
  e.g. wasm component exports — so the consumer source stays a single call).
- The app file should be just its UI/logic + one entry line
  (`lib::launch!(app)`), no transport/WIT/component code.
- If something **genuinely cannot stay inside the library's scope** (a real
  design exception), **stop and ASK the user explicitly** before pushing it into
  the consumer — don't silently leak it.
- Feature-gate target backends in the library, not the app.

Related: [[reference-dioxus-taffy-rust-ui]], [[reference_on_demand_rendering]],
[[feedback-check-latest-versions]].
