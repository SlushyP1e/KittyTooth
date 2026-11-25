//! KittyTooth core: everything except the desktop shell.
//!
//! The crate has no Tauri dependency so the full pipeline (sensor → processing → OSC) can be
//! unit and integration tested on any CI machine with the mock sensor.

pub mod beat;
pub mod config;
pub mod engine;
pub mod error;
pub mod hrm;
pub mod http;
pub mod osc;
pub mod overlay;
pub mod processing;
pub mod sensor;
pub mod tray_icon;

pub use error::{KtError, KtResult, UserError};
