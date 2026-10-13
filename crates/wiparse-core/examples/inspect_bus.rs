//! Offline diagnostic: cargo run -p wiparse-core --example inspect_bus -- scp capture.isf
use wiparse_core::{bus_decode::*, waveform_file::load_waveform_file_all};
fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 || !["scp", "usbpd", "ufcs"].contains(&args[1].as_str()) {
        eprintln!("Usage: inspect_bus <scp|usbpd|ufcs> <capture>");
        std::process::exit(2);
    }
    let traces = load_waveform_file_all(&args[2]).expect("load capture");
    for trace in traces {
        println!("{}: {} samples, {:?}..{:?}, unit {}", trace.channel, trace.x.len(), trace.x.first(), trace.x.last(), trace.y_unit);
        let low = trace.y.iter().copied().fold(f64::INFINITY, f64::min);
        let high = trace.y.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        println!("range={low}..{high}");
        let result = match args[1].as_str() {
            "scp" => decode_scp(&trace, None, &ScpConfig::default()),
            "usbpd" => decode_usb_pd(&trace, None),
            "ufcs" => decode_ufcs(&trace, None, &UfcsConfig::default()),
            _ => panic!("expected scp, usbpd or ufcs"),
        };
        println!("{} {:?}", result.info, result.error);
        for frame in &result.frames { println!("{:.9}..{:.9}: {} {:02X?} {:?}", frame.t_start, frame.t_end, frame.summary, frame.bytes, frame.error); }
    }
}
