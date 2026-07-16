//! Rich Qi tooltip content (Python `Qi22Parser._decode_packet` shell parity for egui).

use super::decode::QiDecode;
use super::defs::{bpp_fsk_pattern, qi22_fsk_pattern};

/// Semantic role for colored hover rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QiTipRole {
    TitleAsk,
    TitleFsk,
    Meta,
    Code,
    Hex,
    Ok,
    Err,
    Warn,
    Field,
    Muted,
    Separator,
}

#[derive(Debug, Clone)]
pub struct QiTipLine {
    pub role: QiTipRole,
    pub text: String,
}

/// Theme-independent content matching Python English tooltip structure.
pub fn format_qi_tooltip(d: &QiDecode) -> Vec<QiTipLine> {
    let mut lines = Vec::with_capacity(8 + d.fields.len());
    let is_ask = d.direction.eq_ignore_ascii_case("ASK");

    if is_ask {
        lines.push(QiTipLine {
            role: QiTipRole::TitleAsk,
            text: "ASK (PRx → PTx)".into(),
        });
    } else {
        lines.push(QiTipLine {
            role: QiTipRole::TitleFsk,
            text: "FSK (PTx → PRx)".into(),
        });
    }
    lines.push(QiTipLine {
        role: QiTipRole::Separator,
        text: String::new(),
    });

    lines.push(QiTipLine {
        role: QiTipRole::Meta,
        text: format!("Header  0x{:02X}  [{}]  {}", d.header, d.name, d.desc),
    });
    lines.push(QiTipLine {
        role: QiTipRole::Code,
        text: format!("Payload ({} B)", d.payload.len()),
    });
    if !d.payload_hex.is_empty() {
        lines.push(QiTipLine {
            role: QiTipRole::Hex,
            text: truncate_hex(&d.payload_hex, 24),
        });
    } else {
        lines.push(QiTipLine {
            role: QiTipRole::Muted,
            text: "(empty)".into(),
        });
    }

    if let Some(cs) = d.checksum {
        let (role, mark) = match d.checksum_ok {
            Some(true) => (QiTipRole::Ok, "OK"),
            Some(false) => (QiTipRole::Err, "ERR"),
            None => (QiTipRole::Muted, "?"),
        };
        lines.push(QiTipLine {
            role,
            text: format!("XOR checksum  0x{cs:02X}  ({mark})"),
        });
    }

    // Dual BPP / Qi 2.2.1 labels for bare FSK (Python `_fsk_bare_pattern`).
    if !is_ask && d.payload.is_empty() {
        if let Some((name, desc)) = bpp_fsk_pattern(d.header) {
            lines.push(QiTipLine {
                role: QiTipRole::Ok,
                text: format!("BPP {name} — {desc}"),
            });
        }
        if let Some((name, desc)) = qi22_fsk_pattern(d.header) {
            let bpp = bpp_fsk_pattern(d.header);
            let same = bpp.map(|(n, d)| n == name && d == desc).unwrap_or(false);
            if !same {
                lines.push(QiTipLine {
                    role: QiTipRole::Warn,
                    text: format!("Qi 2.2.1 {name} — {desc}"),
                });
            }
        }
    }

    lines.push(QiTipLine {
        role: QiTipRole::Separator,
        text: String::new(),
    });
    lines.push(QiTipLine {
        role: QiTipRole::Meta,
        text: "Field decode".into(),
    });

    if d.fields.is_empty() {
        lines.push(QiTipLine {
            role: QiTipRole::Muted,
            text: "• (no fields)".into(),
        });
    } else {
        for f in &d.fields {
            let value = match &f.value {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Bool(b) => b.to_string(),
                other => other.to_string(),
            };
            let mut text = format!("• {}: {}", f.name, value);
            if let Some(u) = &f.unit {
                if !u.is_empty() {
                    text.push(' ');
                    text.push_str(u);
                }
            }
            if let Some(raw) = &f.raw {
                text.push_str(&format!("  ({raw})"));
            }
            let role = field_role(&f.name, &value);
            lines.push(QiTipLine { role, text });
        }
    }

    lines
}

fn truncate_hex(hex: &str, max_bytes: usize) -> String {
    let parts: Vec<&str> = hex.split_whitespace().collect();
    if parts.len() <= max_bytes {
        return hex.to_string();
    }
    format!("{} …", parts[..max_bytes].join(" "))
}

fn field_role(name: &str, value: &str) -> QiTipRole {
    let n = name.to_ascii_lowercase();
    if n.contains("error") || n.contains("err") || value.contains("Fault") {
        return QiTipRole::Err;
    }
    if n.contains("warn") || value.contains("Reserved") || value.contains("unknown") {
        return QiTipRole::Warn;
    }
    if n.contains("ok") || value == "OK" || value.contains("Complete") {
        return QiTipRole::Ok;
    }
    QiTipRole::Field
}

/// Flat plain-text tip (fallback / clipboard).
pub fn format_qi_tooltip_plain(d: &QiDecode) -> String {
    format_qi_tooltip(d)
        .into_iter()
        .filter(|l| l.role != QiTipRole::Separator)
        .map(|l| l.text)
        .collect::<Vec<_>>()
        .join("\n")
}
