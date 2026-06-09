---
name: feedback_no_posting_without_authorization
description: "Never post/publish on the user's behalf (PRs, issues, comments, pushes to public remotes, forks, emails, any outward-facing send) until they explicitly authorize that specific action."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: d04324f7-02b4-4277-bb43-167a1ccfb82b
---

Do NOT post or publish anything on the user's behalf before they explicitly
authorize that specific action: opening PRs/issues, posting comments/reviews,
pushing to public remotes, forking public repos, sending emails/messages, or
any outward-facing publish.

**Why:** these create public, attributable artifacts under the user's identity
(e.g. `harryzz` on GitHub) and are hard to retract. The user wants to review +
approve each one first. A general "go ahead with X" earlier in a task does NOT
extend to the public-publish step — confirm again at the moment of posting.

**How to apply:** prepare everything locally (branch, commit, diff, PR
title/body draft) and present it for review, but stop before `gh pr create` /
`gh repo fork` / `git push <public remote>` / any send. Then ask for explicit
go-ahead. **Pre-authorized (no per-action confirmation needed):** committing AND pushing
to the user's OWN codeberg repo (`codeberg.org/harryzz/wandr`, `origin`).
Clarified 2026-05-29 — the explicit-confirmation gate is *only* for
outward-facing / third-party publishing, not the user's own repo. So push
task work to `origin` freely (follow normal hygiene: only the task's own files,
sensible commit message). The line is *third-party / public* surfaces
(others' GitHub repos, public forks, comments, emails).

Triggered 2026-05-29 (task 60 upstream-dioxus PR — I had a "B+C" go-ahead but
the user reined in the public PR step; then clarified own-repo pushes are fine).
