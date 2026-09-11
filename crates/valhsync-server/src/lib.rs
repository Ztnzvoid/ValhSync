//! Library half of `valhsync-server`, so the HTTP server can be embedded in
//! integration tests. The binary in `main.rs` is a thin CLI over these modules.

pub mod config;
pub mod detect;
pub mod gameserver;
pub mod gui;
pub mod keys;
pub mod logs;
pub mod net;
pub mod pack;
pub mod serve;
pub mod store;
pub mod wizard;
