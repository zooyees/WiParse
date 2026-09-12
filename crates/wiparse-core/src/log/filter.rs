//! Filter / search helpers shared by GUI and background scans.

use super::store::LogStore;

/// Parse filter input. ``|`` splits without trimming segments (Python parity).
pub fn parse_filter_patterns(raw: &str) -> Option<Vec<String>> {
    if raw.is_empty() {
        return None;
    }
    if raw.contains('|') {
        Some(raw.split('|').map(|s| s.to_string()).collect())
    } else {
        Some(vec![raw.to_string()])
    }
}

pub fn prepared_needles(patterns: Option<&[String]>, case_sensitive: bool) -> Vec<String> {
    let Some(patterns) = patterns else {
        return Vec::new();
    };
    patterns
        .iter()
        .filter(|p| !p.is_empty())
        .map(|p| {
            if case_sensitive {
                p.clone()
            } else {
                p.to_lowercase()
            }
        })
        .collect()
}

pub fn line_matches_prepared(line: &str, needles: &[String], case_sensitive: bool) -> bool {
    if needles.is_empty() {
        return true;
    }
    if case_sensitive {
        needles.iter().any(|p| line.contains(p.as_str()))
    } else {
        let hay = line.to_lowercase();
        needles.iter().any(|p| hay.contains(p.as_str()))
    }
}

/// Byte range of one highlighted match inside a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextHit {
    pub row: usize,
    pub start: usize,
    pub end: usize,
}

/// Substring ranges for `needles` (already case-folded when insensitive).
pub fn match_ranges_in_line(line: &str, needles: &[String], case_sensitive: bool) -> Vec<(usize, usize)> {
    let haystack = if case_sensitive {
        line.to_owned()
    } else {
        line.to_lowercase()
    };
    let mut ranges = Vec::new();
    for needle in needles {
        if needle.is_empty() {
            continue;
        }
        let prepared = if case_sensitive {
            needle.clone()
        } else {
            needle.to_lowercase()
        };
        for (start, _) in haystack.match_indices(&prepared) {
            let end = start + prepared.len();
            if line.is_char_boundary(start) && line.is_char_boundary(end) {
                ranges.push((start, end));
            }
        }
    }
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(ranges.len());
    for (start, end) in ranges {
        if let Some(last) = merged.last_mut() {
            if start <= last.1 {
                last.1 = last.1.max(end);
                continue;
            }
        }
        merged.push((start, end));
    }
    merged
}

/// Stop collecting after this many matches (avoids multi-GB index vectors on broad filters).
pub const MAX_FILTER_MATCHES: usize = 50_000;

/// Collect every match occurrence (row + byte range). `truncated` is true when the cap hit.
pub fn collect_match_hits<S: LogStore + ?Sized>(
    store: &S,
    needles: &[String],
    case_sensitive: bool,
) -> (Vec<TextHit>, bool) {
    if needles.is_empty() {
        return (Vec::new(), false);
    }
    let n = store.line_count();
    let mut out = Vec::new();
    out.reserve(n.min(MAX_FILTER_MATCHES));
    for i in 0..n {
        let Some(line) = store.line_at(i) else {
            continue;
        };
        for (start, end) in match_ranges_in_line(&line, needles, case_sensitive) {
            if out.len() >= MAX_FILTER_MATCHES {
                return (out, true);
            }
            out.push(TextHit {
                row: i,
                start,
                end,
            });
        }
    }
    (out, false)
}

pub fn unique_match_rows(hits: &[TextHit]) -> Vec<usize> {
    let mut rows = Vec::new();
    for hit in hits {
        if rows.last() != Some(&hit.row) {
            rows.push(hit.row);
        }
    }
    rows
}

/// Collect master row indices matching filter needles.
pub fn collect_match_indices<S: LogStore + ?Sized>(
    store: &S,
    needles: &[String],
    case_sensitive: bool,
) -> Vec<usize> {
    unique_match_rows(&collect_match_hits(store, needles, case_sensitive).0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::LiveLogStore;

    #[test]
    fn or_patterns_and_case_sensitivity_match_expected_rows() {
        let mut store = LiveLogStore::new();
        store.append_lines(["ASK packet", "fsk packet", "other"]);

        let patterns = parse_filter_patterns("ASK|FSK").unwrap();
        let insensitive = prepared_needles(Some(&patterns), false);
        assert_eq!(
            collect_match_indices(&store, &insensitive, false),
            vec![0, 1]
        );

        let sensitive = prepared_needles(Some(&patterns), true);
        assert_eq!(collect_match_indices(&store, &sensitive, true), vec![0]);
    }

    #[test]
    fn empty_or_segments_do_not_turn_into_match_all() {
        let patterns = parse_filter_patterns("|ASK||").unwrap();
        assert_eq!(prepared_needles(Some(&patterns), false), vec!["ask"]);
    }

    #[test]
    fn match_hits_include_every_occurrence_on_a_line() {
        let mut store = LiveLogStore::new();
        store.append_lines(["ASK then ASK", "none"]);
        let needles = vec!["ASK".to_string()];
        let (hits, truncated) = collect_match_hits(&store, &needles, true);
        assert!(!truncated);
        assert_eq!(
            hits,
            vec![
                TextHit {
                    row: 0,
                    start: 0,
                    end: 3
                },
                TextHit {
                    row: 0,
                    start: 9,
                    end: 12
                }
            ]
        );
        assert_eq!(unique_match_rows(&hits), vec![0]);
    }
}
