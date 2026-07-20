# oxideav spike (task 117 M2)

Evaluating [oxideav](https://github.com/OxideAV/oxideav-workspace) — a pure-Rust
media framework with runtime-loaded HW bridges (VAAPI/NVDEC/VideoToolbox) — as a
**ready component** for wandr's media playback, rather than building codecs +
four HW backends ourselves.

**Read [FINDINGS.md](FINDINGS.md) for the result.** Short version: real decoders,
early integration layer, one showstopper (H.264 HW decodes 0 frames and reports
success without falling back). Not adopted yet; re-run per release.

## Files

- `fetch-samples.sh` — download 4 codec samples (H.264/H.265/VP9/AV1).
- `run-spike.sh` — decode each auto vs `--no-hwaccel`, report frames decoded.
- `FINDINGS.md` — the evaluation.
- `ws/` — the cloned workspace (gitignored; rebuild with the steps below).

## Build the CLI

```bash
git clone --depth 1 https://github.com/OxideAV/oxideav-workspace ws
cd ws && ./scripts/update-crates.sh      # clones ~135 sub-crates into crates/
cargo build --release -p oxideav-cli     # default features already include hwaccel
```

The prebuilt binary needs only glibc ≥ 2.35, so it copies to most Linux boxes
without a rebuild:

```bash
OXIDEAV_CLI=/path/to/oxideav ./run-spike.sh
```
