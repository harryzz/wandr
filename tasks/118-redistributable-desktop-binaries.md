# Task 118 — Redistributable desktop binaries (no "install exactly this .so/.dll")

> Status: 🔲 NOT STARTED — proposal, 2026-07-19, rescoped 2026-07-20.
> Today the CI artifacts are a BUILD CHECK only: a Linux artifact wants the runner's
> `libavutil.so.58`, a macOS one carries a Homebrew bottle's `minos`, so neither runs on
> a user's machine. This task makes GitHub Releases produce files that just run.
>
> **Do task 117 first.** Most of the difficulty here is FFmpeg — its LGPL obligations and
> its per-machine soname/bottle. Task 117 replaces it with statically-linked BSD libraries
> (libvpx et al.), after which the only remaining portability problem is **glibc**, and the
> bundling machinery below shrinks to a tarball. Doing 118 first means building an
> elaborate bundling pipeline for a dependency we are about to delete.

## The one hard constraint

**GPU/driver libraries can never be bundled.** `libEGL`/`libGL`/`libva` (Linux),
`Metal`/`VideoToolbox` (macOS) and `d3d11.dll` (Windows) must come from the user's
system — they are matched to their kernel and hardware. Every solution below therefore
bundles *everything except* the driver stack. That is fine: those are stable ABIs present
on any machine that can run a GUI.

The two things that actually break portability today are **FFmpeg** and **glibc** — and
task 117 removes the first. After it lands, this task is essentially "build against an old
glibc and tar it up".

## Linux — AppImage (or tarball + `$ORIGIN`)

AppImage exists for exactly this: one executable file, bundled libs, system GL.

1. Build in an **old-glibc container** (e.g. `ubuntu:22.04`, or `cargo-zigbuild
   --target x86_64-unknown-linux-gnu.2.31`). A binary built on 24.04 will not start on
   22.04 — glibc is forward- but not backward-compatible. This is the single most common
   cause of "works on CI, not on my machine".
2. Bundle any remaining non-driver deps. **After task 117 there are none** — libvpx and
   friends are statically linked — so this step exists only while FFmpeg is still a
   dynamic dependency.
3. `linuxdeploy` + `appimagetool` produce `wandr-host-x86_64.AppImage`.

Cheaper alternative if AppImage tooling is unwanted: a tarball with `lib/` next to the
binary and `-Wl,-rpath,'$ORIGIN/lib'` — the approach most Linux games use. Same effect,
less machinery.

## Windows — zip with DLLs beside the .exe

Windows resolves DLLs from the executable's directory first, so no installer is needed:

    wandr-host-windows-x86_64.zip
      wasm-android-host.exe
      avcodec-61.dll avformat-61.dll avutil-59.dll swscale-8.dll  (LGPL build)
      ^^^ these disappear after task 117 — the zip becomes just the .exe

Also build with `-C target-feature=+crt-static` so users do not need the
Visual C++ Redistributable — another classic "install this first" failure.

## macOS — .app bundle, and Gatekeeper is the real problem

> SCOPE (2026-07-20): the `macos-x86_64` CI job was dropped — it produced an artifact that
> could not run on an older macOS anyway. x86_64 is tested by **building locally on the
> target Mac**, where brew installs a matching bottle; that removes the `minos` mismatch
> entirely for local use, because build machine == target machine.
>
> This section therefore applies ONLY IF macOS binaries are ever *published*. Note the
> remaining macOS CI job is `macos-latest` = **aarch64** (macOS 14/15), so its artifact
> carries an even higher `minos` than the retired macos-13 one did. If macOS stays
> local-build-only, this whole section — **including the Gatekeeper signing decision** —
> is moot: quarantine is applied to downloads, not to locally-built binaries.

1. Put dylibs in `Contents/Frameworks/`, rewrite install names to `@rpath` /
   `@executable_path/../Frameworks` (`install_name_tool`).
2. Build on the OLDEST runner available with `MACOSX_DEPLOYMENT_TARGET` set (already
   defaulted to 12.0 in `scripts/build-host-macos.sh`).
3. **Gatekeeper**: an unsigned, un-notarized binary is quarantined on download — users see
   "damaged and can't be opened", which reads like a broken build. Options:
   - Apple Developer ID + `codesign` + `notarytool` (needs the $99/yr account) — the only
     option that gives a clean double-click experience;
   - or ship unsigned and document `xattr -dr com.apple.quarantine wandr-host.app`.
   There is no third option. Decide before publishing macOS artifacts.

## Android

Already solved: the APK flow (`cargo apk` / `scripts/build-apk.sh`).

## Licensing consequence (moot after task 117)

Bundling FFmpeg means shipping LGPL code, so the release MUST use an **LGPL-configured**
FFmpeg (`--disable-gpl --disable-nonfree`) — NOT a distro build, which is commonly
`--enable-gpl` (verified locally) and would make the bundle GPL. LGPL then requires: the
licence text, the FFmpeg source (a URL suffices), and the ability to relink — satisfied
automatically because wandr is Apache-2.0 with public source. See NOTICE.

**Task 117 removes this obligation entirely** — libvpx/dav1d/openh264/rav1e are BSD and
statically linked, so there is nothing to relink and no notice beyond attribution. This
section applies only while FFmpeg is still linked.

## Suggested sequencing

0. **Task 117** (drop FFmpeg) — deletes the licence question and the soname problem, and
   makes every step below smaller.
1. Fix **glibc** (build in an old container) — the cheapest fix, and after 117 probably
   the ONLY remaining portability failure.
2. Linux tarball with `$ORIGIN/lib`, then AppImage if the single-file UX is wanted.
3. Windows zip + static CRT.
4. macOS last — it is gated on the signing decision, not on engineering.
5. Publish from a tag via `softprops/action-gh-release`, and keep the current per-push CI
   as the build check it already is.
