//! Shared frame-filter parsing used by both WiParse interfaces.
/// First `filter` + whitespace + number in `frame`.
/// Token boundary: filter must not be mid-identifier (prev char not alphanumeric/_).
pub fn extract_first_filter_value(frame: &str, filter: &str) -> Option<f64> {
    if filter.is_empty() { return None; }
    let mut search_from = 0;
    while let Some(rel) = frame[search_from..].find(filter) {
        let start = search_from + rel;
        let abs = start + filter.len();
        if !is_token_boundary_before(frame, start) {
            search_from = abs;
            continue;
        }
        let rest = &frame[abs..];
        let trimmed = rest.trim_start_matches(|c: char| c == ' ' || c == '\t');
        if trimmed.len() == rest.len() {
            search_from = abs;
            continue;
        }
        if let Some((value, _)) = parse_leading_number(trimmed) {
            return Some(value);
        }
        search_from = abs;
        if search_from >= frame.len() {
            break;
        }
    }
    None
}

fn is_token_boundary_before(text: &str, idx: usize) -> bool {
    if idx == 0 {
        return true;
    }
    let Some(prev) = text[..idx].chars().next_back() else {
        return true;
    };
    !(prev.is_ascii_alphanumeric() || prev == '_')
}

fn parse_leading_number(s: &str) -> Option<(f64, usize)> {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut i = 0;
    if bytes[0] == b'+' || bytes[0] == b'-' {
        i = 1;
    }
    let start_digits = i;
    let mut saw_digit = false;
    let mut saw_dot = false;
    while i < bytes.len() {
        match bytes[i] {
            b'0'..=b'9' => {
                saw_digit = true;
                i += 1;
            }
            b'.' if !saw_dot => {
                saw_dot = true;
                i += 1;
            }
            b'e' | b'E' if saw_digit => {
                i += 1;
                if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
                    i += 1;
                }
                let exp_start = i;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                if i == exp_start {
                    return None;
                }
                break;
            }
            _ => break,
        }
    }
    if !saw_digit || i == start_digits {
        return None;
    }
    let num = std::str::from_utf8(&bytes[..i]).ok()?;
    let value: f64 = num.parse().ok()?;
    if !value.is_finite() {
        return None;
    }
    Some((value, i))
}

