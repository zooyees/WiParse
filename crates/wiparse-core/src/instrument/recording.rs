//! Bounded acquisition storage and stable CSV export.

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::{self, Write};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sample {
    pub timestamp: DateTime<Local>,
    pub resource: String,
    pub channel: String,
    pub value: Option<f64>,
    pub unit: String,
    pub status: String,
}

impl Sample {
    pub fn value(
        resource: impl Into<String>,
        channel: impl Into<String>,
        value: f64,
        unit: impl Into<String>,
    ) -> Self {
        Self {
            timestamp: Local::now(),
            resource: resource.into(),
            channel: channel.into(),
            value: Some(value),
            unit: unit.into(),
            status: "ok".into(),
        }
    }

    pub fn error(resource: impl Into<String>, status: impl Into<String>) -> Self {
        Self {
            timestamp: Local::now(),
            resource: resource.into(),
            channel: String::new(),
            value: None,
            unit: String::new(),
            status: status.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AcquisitionBuffer {
    max_points: usize,
    samples: VecDeque<Sample>,
}

impl AcquisitionBuffer {
    pub fn new(max_points: usize) -> Self {
        Self {
            max_points: max_points.max(1),
            samples: VecDeque::new(),
        }
    }

    pub fn push(&mut self, sample: Sample) {
        while self.samples.len() >= self.max_points {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Sample> {
        self.samples.iter()
    }
}

pub fn export_csv(
    path: impl AsRef<Path>,
    samples: impl IntoIterator<Item = Sample>,
) -> io::Result<()> {
    let file = std::fs::File::create(path)?;
    write_csv(file, samples)
}

pub fn write_csv(
    mut writer: impl Write,
    samples: impl IntoIterator<Item = Sample>,
) -> io::Result<()> {
    writeln!(writer, "timestamp,resource,channel,value,unit,status")?;
    for sample in samples {
        let value = sample.value.map(|v| v.to_string()).unwrap_or_default();
        writeln!(
            writer,
            "{},{},{},{},{},{}",
            csv(&sample.timestamp.to_rfc3339()),
            csv(&sample.resource),
            csv(&sample.channel),
            value,
            csv(&sample.unit),
            csv(&sample.status),
        )?;
    }
    Ok(())
}

fn csv(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_is_bounded() {
        let mut buffer = AcquisitionBuffer::new(2);
        for value in 0..3 {
            buffer.push(Sample::value("R", "V", value as f64, "V"));
        }
        assert_eq!(buffer.len(), 2);
        assert_eq!(buffer.iter().next().unwrap().value, Some(1.0));
    }

    #[test]
    fn csv_quotes_fields() {
        let mut output = Vec::new();
        write_csv(&mut output, [Sample::error("USB,1", "bad \"reply\"")]).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("\"USB,1\""));
        assert!(text.contains("\"bad \"\"reply\"\"\""));
    }
}
