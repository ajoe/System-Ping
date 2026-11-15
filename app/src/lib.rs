//! `system_ping` is the library behind the window: everything in `src/` except `main.rs`.
//!
//! The binary (`main.rs`) re-exports this crate, so an integration test under `tests/` can
//! drive collection headless — engine, history and CSV export, without eframe or a window.

pub mod app;
pub mod exporter;
pub mod history;
pub mod ping;
pub mod runtime;
pub mod screenshot;
pub mod store;
pub mod theme;
pub mod ui;
