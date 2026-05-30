---
name: feedback-check-latest-versions
description: "IMPORTANT RULE: before using any lib/crate/component, check the LATEST version available online and use it — don't reuse age-old pinned/stale versions or inspect stale local caches"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 2b58a1a7-2e85-4748-b34a-e9b89ab2de87
---

**IMPORTANT — read every session.** Before adding or building on **any**
dependency (crate, npm package, component, tool), first **check the latest
published version online** and use that. Do not silently reuse an old version
that happens to be pinned in the repo or sitting in a local cache.

**Why:** 2026-05-30, I started building a new macro/abstraction against
`wit-bindgen` **0.46** — a stale pin left over from tasks 59/60 — when **0.57.1**
was current. Worse, I then inspected `wit-bindgen-rust-macro-0.57.0` from my local
registry cache and presented it as if it were 0.57.1; the real 0.57.1 simply
hadn't been fetched yet. Both are the same mistake: working off old/stale
versions instead of the current one. The user (rightly) caught it twice.

**How to apply:**
- Before depending on X, check its **latest** version (crates.io / npm / GitHub
  releases / `cargo search` / `cargo add X` which picks latest) and prefer it.
- When a repo pins an old version, treat that as a smell — bump to current unless
  there's a documented reason to hold (and if there is, note it).
- Don't reason from a stale local `~/.cargo/registry` copy — `cargo fetch` the
  exact version you'll use and read THAT source.
- A facade crate's version (e.g. `wit-bindgen` 0.57.1) may differ from its
  sub-crates' cached versions — verify the actual resolved version.

Related: [[feedback-clean-library-usage]].
