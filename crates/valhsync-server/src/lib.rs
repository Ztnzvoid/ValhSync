//! Library half of `valhsync-server`, so the HTTP server can be embedded in
//! integration tests. The binary in `main.rs` is a thin CLI over these modules.

pub mod config;
pub mod detect;
pub mod gameserver;
pub mod gui;
pub mod install;
pub mod keys;
pub mod logs;
pub mod names;
pub mod net;
pub mod pack;
pub mod players;
pub mod presence;
pub mod serve;
pub mod store;
pub mod update;
pub mod webhook;
pub mod wizard;
pub mod worldbackup;
