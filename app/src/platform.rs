//! Everything in the app that a real operating system has to answer for.
//!
//! The program is built for Windows, Linux and macOS from one code base, and only a handful of
//! things actually differ between them: where the state file lives, how a folder is opened for the
//! person using it, which command answers a ping and what its output says, how the default gateway
//! is found, and whether starting at sign-in is possible at all. Those are collected here so the
//! rest of the app — history, chart, statistics, i18n — never has to ask which system it runs on.
//!
//! Each module holds one operating-system implementation behind `cfg`, plus a fallback for anything
//! else. A missing feature says so by returning `None` or `false`; nothing here pretends to have
//! done something it could not.

pub mod autostart;
pub mod folder;
pub mod gateway;
pub mod probe;
pub mod statedir;
