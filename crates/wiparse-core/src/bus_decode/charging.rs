//! Shared charging-protocol primitives. Threshold arguments are always volts.
use super::digital::{sample_volts, LogicLevels, LogicWave};
use super::{try_push_frame, BusByteError, BusByteSpan, BusDecodeResult, BusFrame, BusFrameError};
use crate::instrument::WaveformTrace;

pub(crate) fn signal(trace: &WaveformTrace, threshold: Option<f64>) -> LogicWave<'_> {
    // Charging signals (especially CC BMC) are not 3.3 V CMOS. Use the measured
    // swing, retaining hysteresis and a minimum swing to reject ground noise.
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for &v in trace.y.iter().filter(|v| v.is_finite()) {
        let v = sample_volts(v, &trace.y_unit);
        lo = lo.min(v);
        hi = hi.max(v);
    }
    let valid = lo.is_finite() && hi - lo >= 0.2;
    let mid =
        threshold
            .filter(|v| v.is_finite())
            .unwrap_or(if valid { (lo + hi) / 2.0 } else { 0.6 });
    let half = if valid {
        ((hi - lo) * 0.1).clamp(0.02, 0.25)
    } else {
        0.1
    };
    LogicWave::with_levels(
        trace,
        LogicLevels {
            vil: mid - half,
            vih: mid + half,
        },
    )
}

pub(crate) fn crc8(bytes: &[u8], polynomial: u8) -> u8 {
    let mut crc = 0u8;
    for &byte in bytes {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ polynomial
            } else {
                crc << 1
            };
        }
    }
    crc
}

pub(crate) fn append(
    result: &mut BusDecodeResult,
    used: &mut usize,
    frame: BusFrame,
    mut spans: Vec<BusByteSpan>,
) -> bool {
    if !try_push_frame(&mut result.frames, used, frame) {
        result.truncated = true;
        return false;
    }
    result.byte_spans.append(&mut spans);
    true
}

pub(crate) fn finish(result: &mut BusDecodeResult, protocol: &str) {
    let errors = result.frames.iter().filter(|f| f.has_error()).count();
    result.info = format!(
        "{protocol}: {} frame(s), {errors} error(s){}",
        result.frames.len(),
        if result.truncated {
            "; decode limit reached"
        } else {
            ""
        }
    );
    if result.frames.is_empty() && result.error.is_none() {
        result.error = Some(format!(
            "No {protocol} frames — check channel, threshold and sample rate"
        ));
    }
}

pub(crate) fn byte_error(error: BusFrameError) -> BusByteError {
    match error {
        BusFrameError::None => BusByteError::None,
        BusFrameError::Checksum => BusByteError::Checksum,
        BusFrameError::Parity => BusByteError::Parity,
        BusFrameError::Framing => BusByteError::Framing,
    }
}
