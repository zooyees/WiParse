//! WiParse shared core (protocol, config, metrics, serial).

pub mod charge_state;
pub mod config;
pub mod db;
pub mod i18n;
pub mod instrument;
pub mod log;
pub mod metrics;
pub mod paths;
pub mod protocol;
pub mod scope;
pub mod serial;
pub mod wave;
pub mod waveform_file;
pub mod rigol_wfm;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
