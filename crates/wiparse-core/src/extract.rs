//! Local, record-oriented selectors shared by the plotter and log analysis tools.
use regex::{Regex, RegexBuilder};
use serde_json::Value;
use std::{borrow::Cow, cell::OnceCell};

/// Number of field series supported by the data-analysis plot and its AI inputs.
pub const MAX_SERIES: usize = 8;
pub const MAX_SELECTOR_BYTES: usize = 512;

pub const HELP: &str = "Use key:VIN for named numbers, json:/samples/0/v for a JSON Pointer, csv:2, tsv:2, semi:2, pipe:2 or ws:2 for a 1-based column, and re: with capture group 1 for custom text. Select one physical record at a time. Column and JSON values use their declared source units.";

pub const MAX_DISCOVERED_FIELDS: usize = 4096;
#[derive(Default)]
pub struct FieldCatalog {
    pub fields: Vec<(String, String)>,
    pub truncated: bool,
}

/// Return candidates in first-observed order, scanning until EOF or actual catalog overflow.
pub fn discover_catalog(text: &str) -> FieldCatalog {
    use std::{collections::HashSet, sync::LazyLock};
    static KEYS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"([\p{L}_][\p{L}\p{N}_.-]*)(?:[ \t]*[:=][ \t]*|[ \t]+)[+-]?(?:[0-9]|\.[0-9])")
            .unwrap()
    });
    let mut catalog = FieldCatalog::default();
    let mut seen = HashSet::new();
    let mut seen_keys = HashSet::new();
    let mut first = None;
    let mut known_columns = HashSet::new();
    let mut headers = std::collections::HashMap::new();
    for line in text.lines() {
        let line = line.trim().trim_start_matches('\u{feff}');
        if line.is_empty() {
            continue;
        }
        let header = *first.get_or_insert(line);
        if line.starts_with(['{', '[']) {
            if let Ok(value) = serde_json::from_str::<Value>(line) {
                json_fields(&value, "", &mut catalog, &mut seen);
                if catalog.truncated {
                    break;
                }
                continue;
            }
        }
        let mut named = false;
        for cap in KEYS.captures_iter(line) {
            named = true;
            let key = cap.get(1).unwrap().as_str();
            if key.len() + 4 <= MAX_SELECTOR_BYTES && seen_keys.insert(key) {
                add_field(&mut catalog, &mut seen, key, format!("key:{key}"));
                if catalog.truncated {
                    return catalog;
                }
            }
        }
        if named {
            continue;
        }
        for (sep, prefix) in [('\t', "tsv"), (',', "csv"), (';', "semi"), ('|', "pipe")] {
            if !line.contains(sep) {
                continue;
            }
            let names = headers
                .entry(sep)
                .or_insert_with(|| cells(header, sep).take(4096).collect::<Vec<_>>());
            for (i, cell) in cells(line, sep).take(4096).enumerate() {
                if known_columns.contains(&(sep, i)) || number(&cell).is_none() {
                    continue;
                }
                known_columns.insert((sep, i));
                let selector = format!("{prefix}:{}", i + 1);
                let label = names
                    .get(i)
                    .map(|s| s.as_ref())
                    .filter(|s| s.len() <= 64 && !s.is_empty() && number(s).is_none())
                    .unwrap_or(&selector);
                add_field(&mut catalog, &mut seen, label, selector.clone());
                if catalog.truncated {
                    return catalog;
                }
            }
            break;
        }
    }
    catalog
}
pub fn discover(text: &str) -> Vec<(String, String)> {
    discover_catalog(text).fields
}

fn add_field(
    catalog: &mut FieldCatalog,
    seen: &mut std::collections::HashSet<String>,
    label: &str,
    selector: String,
) {
    if selector.len() > MAX_SELECTOR_BYTES || !seen.insert(selector.clone()) {
        return;
    }
    if catalog.fields.len() == MAX_DISCOVERED_FIELDS {
        catalog.truncated = true;
        return;
    }
    catalog.fields.push((label.to_owned(), selector));
}
fn json_fields(
    value: &Value,
    path: &str,
    catalog: &mut FieldCatalog,
    seen: &mut std::collections::HashSet<String>,
) {
    if path.len() + 5 > MAX_SELECTOR_BYTES || catalog.truncated {
        return;
    }
    match value {
        Value::Object(obj) => {
            for (key, v) in obj {
                json_fields(
                    v,
                    &format!("{path}/{}", key.replace('~', "~0").replace('/', "~1")),
                    catalog,
                    seen,
                );
                if catalog.truncated {
                    break;
                }
            }
        }
        Value::Array(arr) => {
            for (i, v) in arr.iter().enumerate() {
                json_fields(v, &format!("{path}/{i}"), catalog, seen);
                if catalog.truncated {
                    break;
                }
            }
        }
        _ if value.is_number() || value.as_str().is_some_and(|s| number(s).is_some()) => {
            add_field(catalog, seen, path, format!("json:{path}"))
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
    Capture(Regex, usize),
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
        Self::compile_inner(input, literal_default, case_sensitive, false)
    }

    /// A bare clock regex may match the whole timestamp. A prefixed match can still use group 1.
    pub fn timestamp(input: &str, literal_default: bool) -> Result<Self, String> {
        Self::compile_inner(input, literal_default, true, true)
    }

    fn compile_inner(
        input: &str,
        literal_default: bool,
        case_sensitive: bool,
        whole_match: bool,
    ) -> Result<Self, String> {
        if input.is_empty() || input.len() > MAX_SELECTOR_BYTES {
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
        let group = usize::from(regex.captures_len() > 1);
        if group == 0 && !whole_match {
            return Err("正则表达式需要第一个捕获组".into());
        }
        Ok(Self::Capture(regex, group))
    }

    pub fn first<'a>(&self, record: &'a Record<'_>) -> Option<Cow<'a, str>> {
        match self {
            Self::Capture(re, group) => re
                .captures(record.text)?
                .get(*group)
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
        if let Self::Capture(re, group) = self {
            for cap in re.captures_iter(record.text) {
                if let Some(v) = cap.get(*group) {
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
    cells(text, sep).nth(wanted)
}

fn cells(text: &str, sep: char) -> impl Iterator<Item = Cow<'_, str>> {
    let mut next = Some(text.trim_start_matches('\u{feff}'));
    std::iter::from_fn(move || {
        let rest = next.take()?;
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
        next = tail;
        Some(if cell.contains("\"\"") {
            Cow::Owned(cell.replace("\"\"", "\""))
        } else {
            Cow::Borrowed(cell.trim())
        })
    })
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
    fn file_catalog_includes_dense_records_and_sparse_late_telemetry() {
        let row = (0..40)
            .map(|i| format!("k{i}=1"))
            .collect::<Vec<_>>()
            .join(" ");
        let noise = format!("{}\n", ".".repeat(1024)).repeat(70);
        let catalog = discover_catalog(&format!("{row}\n{row}\n{noise}Vcoil=12 Tcoil=37"));
        assert_eq!(catalog.fields.len(), 42);
        assert_eq!(catalog.fields[41].1, "key:Tcoil");
        assert!(!catalog.truncated);
        let csv = (0..80).map(|i| i.to_string()).collect::<Vec<_>>().join(",");
        assert_eq!(discover(&csv).last().unwrap().1, "csv:80");
        let json = format!("[{csv}]");
        assert!(discover(&json).iter().any(|(_, s)| s == "json:/79"));
    }

    #[test]
    fn catalog_flags_only_real_overflow() {
        let mut text = (0..MAX_DISCOVERED_FIELDS)
            .map(|i| format!("key{i}=1\n"))
            .collect::<String>();
        text.push_str("key0=2\n");
        assert!(!discover_catalog(&text).truncated);
        text.push_str("extra=3");
        let catalog = discover_catalog(&text);
        assert_eq!(catalog.fields.len(), MAX_DISCOVERED_FIELDS);
        assert!(catalog.truncated);
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
