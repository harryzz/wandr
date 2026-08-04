use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=docs/jellyfin-openapi-stable.json");

    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let openapi_path = manifest_dir
        .join("docs")
        .join("jellyfin-openapi-stable.json");

    let json = fs::read_to_string(openapi_path).expect("read docs/jellyfin-openapi-stable.json");
    let root: serde_json::Value = serde_json::from_str(&json).expect("parse OpenAPI JSON");

    let openapi = root
        .get("openapi")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    let info = root.get("info").unwrap_or(&serde_json::Value::Null);
    let title = info
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let version = info
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let jellyfin_version = info
        .get("x-jellyfin-version")
        .and_then(|v| v.as_str())
        .unwrap_or(version);

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let out_path = out_dir.join("openapi_meta.rs");

    let contents = format!(
        "pub const OPENAPI_SPEC: &str = {openapi:?};\n\
         pub const OPENAPI_TITLE: &str = {title:?};\n\
         pub const OPENAPI_VERSION: &str = {version:?};\n\
         pub const JELLYFIN_SERVER_VERSION: &str = {jellyfin_version:?};\n"
    );

    fs::write(out_path, contents).expect("write OUT_DIR/openapi_meta.rs");
}
