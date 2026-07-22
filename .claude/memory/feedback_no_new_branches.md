---
name: feedback_no_new_branches
description: Never create a new git branch without asking first; user prefers few branches — merge-and-delete promptly.
metadata: 
  node_type: memory
  type: feedback
  originSessionId: efb9ba77-bb47-4ab5-bbac-3dcd59e2771e
---

Never create a new git branch without asking the user first. Work directly on the current branch (usually `main`) by default.

**Why:** User said "i don't like too many branches" after a feature branch (`openswiftui-eleev-2048`) sat around after its work was done. Now codified in `CLAUDE.md` working rule #4.

**How to apply:** If a task seems to warrant a branch (e.g. risky/experimental work), ask before creating one rather than defaulting to it. When a branch's work is finished and confirmed working, proactively merge it into `main` and delete it (local + remote) rather than leaving it around — don't wait to be asked for the cleanup, but do still confirm before the merge/push/delete itself since those are shared-state actions.
