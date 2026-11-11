//! Local, record-oriented selectors shared by the plotter and log analysis tools.
use regex::{Regex, RegexBuilder};
use serde_json::Value;
use std::{borrow::Cow, cell::OnceCell};

pub const HELP: &str = "Use key:VIN for named numbers, json:/samples/0/v for a JSON Pointer, csv:2, tsv:2, semi:2, pipe:2 or ws:2 for a 1-based column, and re: with capture group 1 for custom text. Select one physical record at a time. Column and JSON values use their declared source units.";

/// Sample a bounded prefix. Suggestions are validated by an observed numeric cell.
pub fn discover(text: &str) -> Vec<(String, String)> {
    use std::sync::OnceLock;
    static KEYS: OnceLock<Regex> = OnceLock::new();
    let keys = KEYS.get_or_init(|| {
        Regex::new(
            r"([\p{L}_][\p{L}\p{N}_.-]{0,47})(?:[ \t]*[:=][ \t]*|[ \t]+)[+-]?(?:[0-9]|\.[0-9])",
        )
        .unwrap()
    });
    let mut fields = Vec::new();
    let mut size = 0;
    let mut first = None;
    for line in text.lines().take(64) {
        size += line.len();
        if size > 64 * 1024 || fields.len() >= 16 {
            break;
        }
        let line = line.trim().trim_start_matches('\u{feff}');
        if line.is_empty() {
            continue;
        }
        let header = *first.get_or_insert(line);
        if line.starts_with(['{', '[']) {
            if let Ok(value) = serde_json::from_str::<Value>(line) {
                json_fields(&value, "", 0, &mut fields);
                continue;
            }
        }
        let mut named = false;
        for cap in keys.captures_iter(line).take(16) {
            named = true;
            add_field(&mut fields, &cap[1], format!("key:{}", &cap[1]));
        }
        if named {
            continue;
        }
        for (sep, prefix) in [('\t', "tsv"), (',', "csv"), (';', "semi"), ('|', "pipe")] {
            if !line.contains(sep) {
                continue;
            }
            for i in 0..32 {
                let Some(cell) = column(line, sep, i) else {
                    break;
                };
                if number(&cell).is_some() {
                    let selector = format!("{prefix}:{}", i + 1);
                    let label = column(header, sep, i)
                        .filter(|s| s.len() <= 64 && !s.is_empty() && number(s).is_none());
                    add_field(
                        &mut fields,
                        label.as_deref().unwrap_or(&selector),
                        selector.clone(),
                    );
                }
            }
            break;
        }
    }
    fields
}
fn add_field(fields: &mut Vec<(String, String)>, label: &str, selector: String) {
    if fields.len() < 16 && selector.len() <= 256 && !fields.iter().any(|(_, s)| s == &selector) {
        fields.push((label.to_owned(), selector));
    }
}
fn json_fields(value: &Value, path: &str, depth: usize, fields: &mut Vec<(String, String)>) {
    if fields.len() >= 16 || depth > 8 || path.len() > 240 {
        return;
    }
    match value {
        Value::Object(o) => {
            for (key, v) in o.iter().take(64) {
                json_fields(
                    v,
                    &format!("{path}/{}", key.replace('~', "~0").replace('/', "~1")),
                    depth + 1,
                    fields,
                );
            }
        }
        Value::Array(a) => {
            for (i, v) in a.iter().take(16).enumerate() {
                json_fields(v, &format!("{path}/{i}"), depth + 1, fields);
            }
        }
        _ if value.is_number() || value.as_str().is_some_and(|s| number(s).is_some()) => {
            add_field(fields, path, format!("json:{path}"))
        }
        _ => {}
    }
}

pub struct Record<'a> {
    text: &'a str,
    json: OnceCell<Option<Value>>,
}

pub fn number(text: &str) -> Option<f64> {
    let text = text.trim();
    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
    for (prefix, radix) in [("0x", 16), ("0X", 16), ("0b", 2), ("0B", 2)] {
        if let Some(digits) = unsigned.strip_prefix(prefix) {
            let magnitude = u64::from_str_radix(digits, radix).ok()? as f64;
            return Some(if text.starts_with('-') {
                -magnitude
            } else {
                magnitude
            });
        }
    }
    text.parse::<f64>().ok().filter(|value| value.is_finite())
}

impl<'a> Record<'a> {
    pub fn new(text: &'a str) -> Self {
        Self {
            text,
            json: OnceCell::new(),
        }
    }
}

pub enum Selector {
    Capture(Regex),
    Pointer(String),
    Column(char, usize),
    Words(usize),
}

impl Selector {
    /// Existing AI patterns are regular expressions. Existing GUI filters are literal names.
    pub fn compile(
        input: &str,
        literal_default: bool,
        case_sensitive: bool,
    ) -> Result<Self, String> {
        if input.is_empty() || input.len() > 512 {
            return Err("字段规则长度须为 1–512 字节".into());
        }
        let (prefix, arg) = input.split_once(':').unwrap_or(("", input));
        if prefix == "json" {
            if !arg.starts_with('/') {
                return Err("JSON Pointer 须以 / 开头".into());
            }
            return Ok(Self::Pointer(arg.to_owned()));
        }
        if matches!(prefix, "csv" | "tsv" | "semi" | "pipe" | "ws") {
            let index = arg
                .parse::<usize>()
                .ok()
                .filter(|n| (1..=4096).contains(n))
                .ok_or("列号须为 1–4096")?
                - 1;
            return Ok(match prefix {
                "ws" => Self::Words(index),
                "csv" => Self::Column(',', index),
                "tsv" => Self::Column('\t', index),
                "semi" => Self::Column(';', index),
                _ => Self::Column('|', index),
            });
        }
        let pattern = if prefix == "re" {
            arg.to_owned()
        } else if prefix == "key" || literal_default {
            let key = if prefix == "key" { arg } else { input };
            if key.is_empty() {
                return Err("字段名为空".into());
            }
            format!(
                r#"(?m)(?:^|[^\p{{L}}\p{{N}}_])["']?{}["']?(?:[ \t]*[:=][ \t]*|[ \t]+)["']?([+-]?(?:0[xXbB][0-9A-Za-z_]*|(?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)(?:[eE][+-]?[0-9]+)?))"#,
                regex::escape(key)
            )
        } else {
            input.to_owned()
        };
        let regex = RegexBuilder::new(&pattern)
            .case_insensitive(!case_sensitive)
            .size_limit(1024 * 1024)
            .dfa_size_limit(1024 * 1024)
            .build()
            .map_err(|e| e.to_string())?;
        if regex.captures_len() < 2 {
            return Err("正则表达式需要第一个捕获组".into());
        }
        Ok(Self::Capture(regex))
    }

    pub fn first<'a>(&self, record: &'a Record<'_>) -> Option<Cow<'a, str>> {
        match self {
            Self::Capture(re) => re
                .captures(record.text)?
                .get(1)
                .map(|v| Cow::Borrowed(v.as_str())),
            Self::Pointer(path) => {
                let value = record
                    .json
                    .get_or_init(|| {
                        serde_json::from_str(record.text.trim_start_matches('\u{feff}')).ok()
                    })
                    .as_ref()?
                    .pointer(path)?;
                match value {
                    Value::String(s) => Some(Cow::Borrowed(s.trim())),
                    Value::Number(n) => Some(Cow::Owned(n.to_string())),
                    _ => None,
                }
            }
            Self::Words(n) => record.text.split_whitespace().nth(*n).map(Cow::Borrowed),
            Self::Column(sep, n) => column(record.text, *sep, *n),
        }
    }

    pub fn visit(&self, record: &Record<'_>, mut consume: impl FnMut(&str)) {
        if let Self::Capture(re) = self {
            for cap in re.captures_iter(record.text) {
                if let Some(v) = cap.get(1) {
                    consume(v.as_str());
                }
            }
        } else if let Some(v) = self.first(record) {
            consume(&v);
        }
    }
}

/// Scan one delimited record, preserving empty columns and doubled quotes.
fn column(text: &str, sep: char, wanted: usize) -> Option<Cow<'_, str>> {
    let mut rest = text.trim_start_matches('\u{feff}');
    for index in 0..=wanted {
        let (cell, tail) = if let Some(quoted) = rest.strip_prefix('"') {
            let mut pos = 0;
            loop {
                pos += quoted[pos..].find('"')?;
                let tail = &quoted[pos + 1..];
                if tail.starts_with('"') {
                    pos += 2;
                    continue;
                }
                if !tail.is_empty() && !tail.starts_with(sep) {
                    return None;
                }
                let value = &quoted[..pos];
                break (value, tail.strip_prefix(sep));
            }
        } else {
            match rest.split_once(sep) {
                Some((a, b)) => (a, Some(b)),
                None => (rest, None),
            }
        };
        if index == wanted {
            return Some(if cell.contains("\"\"") {
                Cow::Owned(cell.replace("\"\"", "\""))
            } else {
                Cow::Borrowed(cell.trim())
            });
        }
        rest = tail?;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn named_nested_and_delimited_values_are_unambiguous() {
        for (rule, line, expected) in [
            ("key:VIN", "NOTVIN=55 VIN: -1.25e2V", "-1.25e2"),
            ("json:/a~1b/1/v", r#"{"a/b":[null,{"v":"0.125"}]}"#, "0.125"),
            ("csv:3", r#""a,""quoted""",,7"#, "7"),
            ("tsv:2", "t\t2.4", "2.4"),
            ("semi:2", "1;3", "3"),
            ("pipe:2", "1|4", "4"),
            ("ws:3", "a  b   50", "50"),
            ("re:val\\[([0-9]+)\\]", "val[81]", "81"),
        ] {
            let selector = Selector::compile(rule, false, true).unwrap();
            assert_eq!(
                selector.first(&Record::new(line)).as_deref(),
                Some(expected),
                "{rule}"
            );
        }
        assert_eq!(number("-0xFF"), Some(-255.));
        assert_eq!(number("0b1010"), Some(10.));
        assert!(number("0xZ").is_none());
        let raw = Record::new("ADC=0x1F");
        assert_eq!(
            number(
                &Selector::compile("ADC", true, true)
                    .unwrap()
                    .first(&raw)
                    .unwrap()
            ),
            Some(31.)
        );
        assert!(Selector::compile("csv:0", false, true).is_err());
        assert!(Selector::compile("re:nocapture", false, true).is_err());
        assert!(Selector::compile("csv:2", false, true)
            .unwrap()
            .first(&Record::new("\"unterminated,2"))
            .is_none());
    }
    #[test]
    fn detected_rules_reproduce_observed_values() {
        let rules = discover("time,VIN,mode\n0,5.2,ok\n1,,wait");
        assert_eq!(
            rules,
            vec![
                ("time".into(), "csv:1".into()),
                ("VIN".into(), "csv:2".into())
            ]
        );
        assert_eq!(discover(r#"{"CH1":{"v":5},"CH2":{"v":8}}"#).len(), 2);
    }
}
