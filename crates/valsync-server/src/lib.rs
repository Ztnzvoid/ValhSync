//! Library half of `valsync-server`, so the HTTP server can be embedded in
//! integration tests. The binary in `main.rs` is a thin CLI over these modules.

pub mod config;
pub mod detect;
pub mod keys;
pub mod net;
pub mod pack;
pub mod serve;
pub mod store;
