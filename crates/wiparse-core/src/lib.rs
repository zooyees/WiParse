//! WiParse shared core (protocol, config, metrics, serial).

pub mod brief;
pub mod bus_decode;
pub mod bridge;
pub mod charge_state;
pub mod config;
pub mod db;
pub mod evidence;
pub mod i18n;
pub mod instrument;
pub mod log;
pub mod marketplace;
pub mod metrics;
pub mod native_lib;
pub mod paths;
pub mod probe;
pub mod protocol;
pub mod scope;
pub mod serial;
pub mod testrun;
pub mod update;
pub mod usb;
pub mod wave;
pub mod wave_display;
pub mod waveform_file;
pub mod rigol_wfm;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
