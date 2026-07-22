---
name: feedback_wit_changes_need_approval
description: "RULE: never edit a WIT contract without the user's explicit approval — additive counts, ask first with what/why/who-consumes."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 25b6eb4c-9122-4870-8734-7e515af11a68
  modified: 2026-07-22T10:02:48.016Z
---

**Never change a WIT contract without asking first.** Anything under `wit/`,
`contracts/`, or `proposals/*/wit/`: a new verb, a new record, a new field, a
renamed type, a changed enum. Stop and ask — *including* when the change is
additive and obviously needed.

**Why:** a contract is not code. It is the boundary every guest and every
language binding compiles against, so an edit ripples into shipped apps, ABIs,
cached `.cwasm` (the AOT hash changes), the zygote image, and the WASI proposals
this project intends to publish. "Additive so nothing breaks" is a claim to be
approved, not an exemption: WIT record layout is positional, so adding a field to
an existing record breaks every consumer's ABI, and the additive-looking changes
are precisely the ones that get waved through. The user owns this boundary
because they own its consumers and its publication.

**How to apply:** propose, then wait. State (1) the verb or type you want,
(2) why the existing contract cannot express it, (3) who else consumes the
interface and what they'd have to rebuild. Prefer a NEW additive verb or a static
constructor over touching an existing record — but that shape still needs
approval, it just makes approval cheaper to give. Once approved, see
[[feedback_shared_wit_rebuild_all_consumers]]: a shared-type change means
rebuilding every importer and restarting the zygote, and
[[feedback_audit_wit_consumers_scan_binaries]] for finding who those importers
actually are.

**`wandr:*` IS OURS, `wasi:*` IS FOR EVERYONE (user, 2026-07-22).** The two are
held to different standards, and the split decides where a verb goes:

* `wandr:*` may carry PRAGMATIC surface — implementation names, enumeration,
  diagnostics, `require-hardware`-style knobs, and shapes forced on us by an
  existing ABI. It only has to serve wandr's own guests, and we can change it.
* `wasi:*` proposals must stay SPEC-CLEAN, because everyone else has to live with
  them. Concretely: no implementation names (`"vaapi"`, `"openh264"` are
  per-platform strings a portable guest must never branch on), no enumeration of
  backends (WebCodecs deliberately has `isConfigSupported(config)` instead — a
  query about ONE config), preferences as HINTS rather than requirements (a spec
  verb that fails for a reason the caller cannot handle is a bad spec verb), and
  no shape that exists only to dodge OUR migration constraints.

The test: "would this make sense on a platform that has never heard of wandr?"
If not, it goes in `wandr:*`. A workaround forced by our own shipped ABI —
e.g. a separate `open-accelerated` constructor because an existing positional
record cannot gain a field — is exactly the kind of thing that stays local; the
proposal takes the clean form (a field on the config) precisely because a DRAFT
with no consumers has no ABI to protect.

**Where this came from (2026-07-22, task 117):** asked to add codec listing and
HW/SW forcing to the host and player, I designed it as new verbs on
`wandr:video/decoder` (`list-decoders`, `open-accelerated`, `implementation`) and
edited `contracts/wit/video.wit` without asking — reasoning that additive verbs
plus an untouched `decoder-config` could not break the shipped call path. The
user stopped me and asked whether such a rule existed. It did not; rules 1-4
covered read-source-first, no-hardcoding, no per-app host hardcoding, and no new
branches. It is now rule 4 in CLAUDE.md.
