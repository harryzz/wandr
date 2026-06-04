// wart-hal-display build.rs — codegen the FULL android.gui.ISurfaceComposer
// (task 78). rsbinder-aidl 0.9.0 parses + resolves the whole interface after a
// trivial float-literal fix (`0f` → `0`) in a few imported parcelables; the
// native-backed types resolve to the `gui_aidl_types_rs` shim. We copy the AIDL
// closure into OUT_DIR and apply the float fix THERE so no vendored file is
// mutated (unlike the host's in-place ISurfaceComposer trim). Android only.

use std::path::{Path, PathBuf};

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "android" {
        return;
    }

    println!("cargo:rustc-link-lib=binder_ndk");

    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let vendor = PathBuf::from("../wart-host/vendor");
    // Full (un-trimmed) ISurfaceComposer + most android.gui types live in the
    // extract copy; IWindowInfosPublisher/WindowInfo/FocusRequest etc. live in
    // the sibling libs/gui dir.
    let extract_src = vendor.join("generated-aidl/.aidl-src/extract/libs/gui/aidl");
    let extra_src = vendor.join("aosp-frameworks-native/libs/gui");

    let extract_dst = out.join("aidl-extract");
    let extra_dst = out.join("aidl-extra");
    copy_aidl_tree(&extract_src, &extract_dst);
    copy_aidl_tree(&extra_src, &extra_dst);

    // The parser fix: strip the `f` suffix on float-literal defaults
    // (`0f`/`-1f`/`1.0f`) — only in default-value contexts (followed by ; , ) or
    // whitespace) so identifiers like utf8 are untouched.
    let float_lit = regex::Regex::new(r"([0-9])f([;,)\s])").unwrap();
    fix_floats(&extract_dst, &float_lit);
    fix_floats(&extra_dst, &float_lit);

    let source = extract_dst.join("android/gui/ISurfaceComposer.aidl");
    rsbinder_aidl::Builder::new()
        .source(source)
        .include_dir(extract_dst.clone())
        .include_dir(extra_dst.clone())
        .set_async_support(true)
        .output(out.join("sf_bindings.rs"))
        .generate()
        .expect("rsbinder-aidl ISurfaceComposer codegen failed");
}

/// Recursively copy every `*.aidl` under `src` into `dst`, preserving the
/// relative tree (so package paths resolve).
fn copy_aidl_tree(src: &Path, dst: &Path) {
    if !src.exists() {
        panic!("wart-hal-display: AIDL source missing: {}", src.display());
    }
    for entry in walk(src) {
        if entry.extension().and_then(|e| e.to_str()) != Some("aidl") {
            continue;
        }
        let rel = entry.strip_prefix(src).unwrap();
        let target = dst.join(rel);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::copy(&entry, &target).unwrap();
    }
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}

fn fix_floats(dir: &Path, re: &regex::Regex) {
    for f in walk(dir) {
        if f.extension().and_then(|e| e.to_str()) != Some("aidl") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        if re.is_match(&text) {
            let fixed = re.replace_all(&text, "$1$2");
            std::fs::write(&f, fixed.as_ref()).unwrap();
        }
    }
}
