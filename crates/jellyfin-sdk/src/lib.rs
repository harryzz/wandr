#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! An async Jellyfin API client built on `reqwest`.
//!
//! This crate is intentionally structured to scale:
//! - `client` contains the HTTP layer (auth, base URL, error mapping).
//! - `api::*` contains grouped endpoint wrappers.
//! - `models::*` contains request/response DTOs.

mod client;
mod error;
pub mod openapi;
pub mod pagination;

pub mod api;
pub mod models;

pub use crate::client::{JellyfinClient, JellyfinClientBuilder, RetryConfig};
pub use crate::error::{Error, Result};
