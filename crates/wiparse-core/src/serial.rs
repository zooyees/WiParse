//! Serial port helpers.

use crate::metrics::{parse_metric_frame, MetricSample};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SerialError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Port(#[from] serialport::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortInfo {
    pub device: String,
    pub description: String,
    pub hwid: String,
}

pub fn list_ports() -> Result<Vec<PortInfo>, SerialError> {
    let mut out = Vec::new();
    for p in serialport::available_ports()? {
        let (description, hwid) = match &p.port_type {
            serialport::SerialPortType::UsbPort(info) => {
                let desc = info.product.clone().unwrap_or_else(|| "USB Serial".into());
                let vid_pid = format!("USB VID:PID={:04X}:{:04X}", info.vid, info.pid);
                (desc, vid_pid)
            }
            _ => (String::new(), String::new()),
        };
        out.push(PortInfo {
            device: p.port_name,
            description,
            hwid,
        });
    }
    out.sort_by(|a, b| a.device.cmp(&b.device));
    Ok(out)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CapturedEvent {
    Metrics(MetricSample),
    Log {
        line: String,
        qi: Option<crate::protocol::QiLineParse>,
    },
}

pub struct SerialSession {
    port: Box<dyn serialport::SerialPort>,
    buf: Vec<u8>,
}

impl SerialSession {
    pub fn open(device: &str, baud: u32) -> Result<Self, SerialError> {
        let port = serialport::new(device, baud)
            .timeout(Duration::from_millis(50))
            .open()?;
        Ok(Self {
            port,
            buf: Vec::with_capacity(4096),
        })
    }

    pub fn write_hex(&mut self, hex_data: &str) -> Result<usize, SerialError> {
        let cleaned: String = hex_data.chars().filter(|c| c.is_ascii_hexdigit()).collect();
        if cleaned.len() % 2 != 0 {
            return Err(SerialError::Message("hex length must be even".into()));
        }
        let mut bytes = Vec::with_capacity(cleaned.len() / 2);
        let mut i = 0;
        while i < cleaned.len() {
            let b = u8::from_str_radix(&cleaned[i..i + 2], 16)
                .map_err(|e| SerialError::Message(e.to_string()))?;
            bytes.push(b);
            i += 2;
        }
        self.port.write_all(&bytes)?;
        Ok(bytes.len())
    }

    /// Read available bytes and emit complete lines as events.
    pub fn poll_events(&mut self) -> Result<Vec<CapturedEvent>, SerialError> {
        let mut tmp = [0u8; 2048];
        match self.port.read(&mut tmp) {
            Ok(0) => {}
            Ok(n) => self.buf.extend_from_slice(&tmp[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => return Err(e.into()),
        }

        let mut events = Vec::new();
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let mut line_bytes = self.buf.drain(..=pos).collect::<Vec<_>>();
            if line_bytes.last() == Some(&b'\n') {
                line_bytes.pop();
            }
            if line_bytes.last() == Some(&b'\r') {
                line_bytes.pop();
            }
            let line = String::from_utf8_lossy(&line_bytes).to_string();
            if let Some(m) = parse_metric_frame(&line) {
                events.push(CapturedEvent::Metrics(m));
            } else if !line.is_empty() {
                // Qi decode is on-demand in the log hover tooltip (Auto Parse).
                events.push(CapturedEvent::Log { line, qi: None });
            }
        }
        Ok(events)
    }
}
