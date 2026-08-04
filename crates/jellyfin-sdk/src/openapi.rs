//! Build-time metadata extracted from `docs/jellyfin-openapi-stable.json`.

#[allow(missing_docs)]
mod meta {
    include!(concat!(env!("OUT_DIR"), "/openapi_meta.rs"));
}

pub use meta::*;
