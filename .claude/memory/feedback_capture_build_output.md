---
name: feedback-capture-build-output
description: "Never pipe a build/long command through grep in a way that can hide errors, and NEVER re-run a build just to see the error — capture full output to a log/file and read THAT."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 7bad5d74-8e10-497a-8e63-4ca3f6bc0fbf
---

When running builds (or any long/expensive command), capture the **full** output to a
file/log, then `grep`/`Read` the log to inspect. Do NOT pre-filter the live command
through `grep "error:|..."` as the only capture: when the build fails in a format the
pattern doesn't match, the filter shows nothing and you've lost the error — forcing a
**second run just to see it** (for foreground commands the unmatched output is gone for good).

**Why:** the filter-then-maybe-rerun habit burns whole build cycles and the user's time
every session. There is NO rule requiring it — it was a self-imposed habit to keep build
noise out of context, and it backfires precisely when something breaks (which is when you
most need the output).

**How to apply:**
- Foreground: `cmd > /tmp/build.log 2>&1` (or `tee`), then `grep`/`Read` `/tmp/build.log`.
  Filtering a SAVED log is free and repeatable; filtering a live stream is destructive.
- `run_in_background: true` already saves the complete output to a task file — on
  failure, **Read that file** (tail enough lines to see the verbatim error). Never re-run
  the build to reveal an error.
- Keep grep for *narrowing* what you read from the log, never as the *only* sink for a
  command whose errors you can't predict.
