//! Log storage — live ring buffer and disk-backed file store with chunked cache.

pub mod file;
pub mod filter;
pub mod live;
pub mod store;

pub use file::{build_file_store_worker, FileBuildEvent, FileLogStore};
pub use filter::{
    collect_match_hits, collect_match_indices, line_matches_prepared, match_ranges_in_line,
    parse_filter_patterns, prepared_needles, unique_match_rows, TextHit, MAX_FILTER_MATCHES,
};
pub use live::{LiveLogStore, MAX_LIVE_LINES};
pub use store::LogStore;
