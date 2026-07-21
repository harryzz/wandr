---
name: feedback_check_means_verify
description: "When the user says \"check\", actually verify (search/read source) — do not answer from prior knowledge"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: ed607dbd-a38a-49fd-b8ec-bfd92f150821
---

When the user asks me to **"check"** something, they mean *go verify it against a
real source* — search the internet, read the code, run the probe — NOT answer
from what I already believe I know.

**Why:** I claimed "VAAPI does NOT exist on Windows" and "the Windows libva port
is Intel-GPU-only" from memory. Both were wrong: since libva 2.17 + Mesa 22.3 the
`libva-win32` node + Mesa **VAOn12** driver run VA-API on Windows over D3D12
Video, **cross-vendor** (Intel/AMD/NVIDIA), and FFmpeg mainline has supported it
since Apr 2023. Answering from stale knowledge sent the user down a wrong design
fork (Media Foundation as a forced separate Windows backend).

**How to apply:** On "check" / "recheck" / "did you check…", run the actual
verification FIRST (WebSearch/WebFetch, Read the source, run the command) and base
the answer on what it returns. State findings as "the source says X", and if it
contradicts what I said before, say so plainly. This is the same spirit as
[[feedback_read_source_first]] and [[feedback_humility_proven_vs_guessed]] —
extended to explicit "check" requests: verification is not optional there.

**Second instance (task 117 VA-API):** I declared Intel UHD 620 "impossible for
WSL GPU video accel, driver updates won't help" — reasoning from Microsoft's
*supported-hardware TABLE* (11th-gen+). The user ran `GALLIUM_DRIVER=d3d12
glxinfo` → `D3D12 (Intel UHD 620)`: the D3D12 driver DOES drive the GPU, it just
isn't auto-selected (defaults to llvmpipe). **An official "supported/tested" list
is not a "physically works" list** — treat vendor support matrices as the floor
for *auto-enablement*, never as proof something can't work when forced. Before
saying "not possible," run the falsifying command (here: force the driver and
look) — the dmesg ioctl errors I cited were the default-path fallback, not a hard
capability wall.
