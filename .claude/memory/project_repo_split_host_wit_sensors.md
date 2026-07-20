---
name: project_repo_split_host_wit_sensors
description: "wandr-host, the WIT contracts and the sensorservice client are now separate GitHub repos consumed by wandr as submodules — paths, CI, and the traps the split exposed"
metadata: 
  node_type: memory
  type: project
  originSessionId: efb9ba77-bb47-4ab5-bbac-3dcd59e2771e
---

**Done 2026-07-19.** `wandr-host` is clonable + buildable on its own, per platform, with
GitHub Actions. Three public repos, all consumed back into `wandr` as submodules:

| Path in `wandr` | Repo |
|---|---|
| `runtime/wandr-host` | github.com/harryzz/wandr-host |
| `runtime/wandr-sensors-client` | github.com/harryzz/wandr-sensors-client |
| `contracts/` (was `wit/` + `proposals/`) | github.com/harryzz/wandr-wit |

Main-repo commits: `cc5c6638` (rename), `3c4c6305` (build.rs fix — **restore point before
the split**), `ec75c119` (the rewiring). `wandr-host` carries 103 commits of real history
(`git subtree split`); `wandr-wit` 130 (two `git subtree add` grafts, one per path).

**Why only ONE HAL-ish crate was extracted:** `wandr-sensors-client` is the only sensor
consumer shared by BOTH sides — the host serves guest-facing `wandr:device/sensors`, the
arbiter drives proximity/auto-brightness. `hal-display`/`hal-lights`/`hal-net` have a
single consumer each and stayed in `wandr`. The dual access is deliberate (task 77: the
arbiter is the persistent coordinator, hosts are per-app + ephemeral) and safe because
both are clients of **`sensorservice`**, which multiplexes the single-client HAL — see
[[reference_openswiftui_custom_path_wandr]] style caution: do NOT "fix" this by making
the host forward through the arbiter; task 94 already resolved the HAL conflict.

**Renamed while splitting:** `wandr-hal-sensors` → `wandr-sensors-client` (it binds
`android.frameworks.sensorservice.ISensorManager` — a *service* client, not a HAL client;
the `hal-` prefix was a task-20/85 fossil). Types `HalSensor`/`HalSample` →
`SensorDesc`/`SensorEvent` — they could NOT be `SensorInfo`/`SensorSample`, which belong
to the WIT bindings that `sensors_impl.rs` converts INTO.

**Traps the split exposed (all fixed, all would bite again):**
- `.cargo/config.toml` forced `target = aarch64-linux-android` AND hardcoded
  `/home/harry/android-ndk-r27d/...` — a fresh clone could not build at all. Now: no
  default target, Android via cargo-ndk + `$ANDROID_NDK_HOME`.
- `wandr-sensors-client/build.rs` hardcoded `../wandr-host/vendor` (monorepo-only). Now
  probes `../wandr-host/vendor` AND `../../vendor`, honours `WANDR_AOSP_VENDOR`.
- The `[patch]` for rsbinder is **unconditional**, so `crates/rsbinder` must exist on
  EVERY target, not just Android (CI desktop jobs must fetch it).
- `ffmpeg-next 8.1` is a HARD desktop dep (not feature-gated) → desktop needs system
  ffmpeg dev libs; Windows must use a **release** ffmpeg (BtbN `master-latest` is
  post-8.0 and fails: `AVCodec::pix_fmts` removed).
- CI must build `--features p3-async` — the build scripts all default `P3=1`, but it is
  NOT in `default`, so a plain `cargo build` yields a host with no WASI 0.3 surface and
  guests fail at instantiate with "resource implementation is missing".
- `git subtree split` reads the WORKING TREE, not HEAD — split before `git mv`, or split
  from a detached worktree. It also does not follow renames (history stops at the rename).

**Gotchas for daily work:**
- Fresh clone of `wandr` needs `--recurse-submodules`; Android needs
  `git -C runtime/wandr-host submodule update --init --recursive` (~2.3 GB, one time).
- `runtime/wandr-host/vendor/` holds 4 dirs that are **.gitignore'd in wandr-host** (not
  a split artifact — pre-existing): `aosp-system-{core,libbase,logging}` are headers-only
  AOSP clones at tag `android-15.0.0_r36`, and `generated-aidl` is build output of
  `tools/scripts/gen-libgui-aidl.sh`. No clone or `submodule update` restores them, so
  never wipe `vendor/` assuming git rebuilds it. `tools/scripts/build-sf-probe.sh` is now
  self-healing (clones/regenerates what's missing, `4cdd09f7`).
- `runtime/wandr-hal-display/build.rs` uses `../wandr-host/vendor` — still valid (both
  stay siblings under `runtime/`), but it means Android builds of hal-display need the
  host submodule's vendor initialized.
- **`runtime/wandr-host/cpp/build/libsf_surface.so` is untracked AND NOT locally
  rebuildable** — it is built in an AOSP Soong tree
  (`out/soong/.intermediates/external/sf_surface/...`, the a-03 box), then pushed to
  `/data/local/tmp/`. Deleting `runtime/wandr-host/` wipes it and `run-hybrid-stack.sh`
  then aborts with "✗ missing libsf_surface.so". Recover with
  `adb pull /data/local/tmp/libsf_surface.so runtime/wandr-host/cpp/build/`.
- **`adb shell "su -c 'cat <binary>'"` CORRUPTS binaries on WSL** (CRLF translation —
  85848 bytes came back as 86063). Always `adb pull`.
- The Android linker now comes from `tools/scripts/env-android.sh`
  (`CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER`, commit `db844f60`), not the crate's
  `.cargo/config.toml`. Never set `*_RUSTFLAGS` there — it would clobber the
  `aes_armv8`/`polyval_armv8` cfgs and silently drop to software AES.
- Verified 2026-07-19: host built FROM the submodule (66 MB) and deployed to the Pixel
  via `run-hybrid-stack.sh --wandr-only`; full `--no-art` stack came up clean.
- FUTURE: `tasks/117-wandr-video-consolidation.md` (drop FFmpeg → static BSD libvpx et al.
  + HW backends; no "cpal for video" exists, and no pure-Rust VP8 at all) and
  `tasks/118-redistributable-desktop-binaries.md` (do 117 FIRST — it removes the LGPL +
  soname problem; then it is just old-glibc + tarball). Both in tasks/STATUS.md.
- CI artifacts are a BUILD CHECK, not portable binaries: the desktop host links the
  runner's ffmpeg soname (`libavutil.so.58`), so it won't run on a box with a different
  ffmpeg. Build locally to run locally.
