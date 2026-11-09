//! Exact range extrema for immutable samples. Only edge fragments are scanned;
//! complete 256-sample blocks are merged from a shared segment tree.
use std::sync::Arc;

const BLOCK: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Extrema { low: f64, high: f64 }
impl Extrema {
    const EMPTY: Self = Self { low: f64::INFINITY, high: f64::NEG_INFINITY };
    fn merge(self, other: Self) -> Self {
        Self { low: self.low.min(other.low), high: self.high.max(other.high) }
    }
    fn add(self, value: f64) -> Self { self.merge(Self { low: value, high: value }) }
}

#[derive(Clone)]
pub(super) struct ExtremaIndex {
    points: usize,
    leaves: usize,
    tree: Arc<[Extrema]>,
}

impl ExtremaIndex {
    pub(super) fn new(samples: &[f64]) -> Result<Self, String> {
        if samples.is_empty() { return Err("不能索引空波形".into()); }
        let leaves = samples.len().div_ceil(BLOCK).checked_next_power_of_two().ok_or("波形索引过大")?;
        let size = leaves.checked_mul(2).ok_or("波形索引过大")?;
        let mut tree = vec![Extrema::EMPTY; size];
        for (index, block) in samples.chunks(BLOCK).enumerate() {
            let mut bounds = Extrema::EMPTY;
            for &value in block {
                if !value.is_finite() { return Err("波形索引包含非有限采样值".into()); }
                bounds = bounds.add(value);
            }
            tree[leaves + index] = bounds;
        }
        for index in (1..leaves).rev() { tree[index] = tree[index * 2].merge(tree[index * 2 + 1]); }
        Ok(Self { points: samples.len(), leaves, tree: tree.into() })
    }

    /// Exact half-open sample range. The original samples must be the immutable
    /// array used to build this index; Dataset keeps both in the same order.
    pub(super) fn range(&self, samples: &[f64], start: usize, end: usize) -> Option<(f64, f64)> {
        if samples.len() != self.points || start >= end || end > samples.len() { return None; }
        let first_full = start.div_ceil(BLOCK) * BLOCK;
        let edge_end = first_full.min(end);
        let full_end = end / BLOCK * BLOCK;
        let mut bounds = Extrema::EMPTY;
        for &value in &samples[start..edge_end] { bounds = bounds.add(value); }
        if edge_end < full_end {
            let mut left = self.leaves + edge_end / BLOCK;
            let mut right = self.leaves + full_end / BLOCK;
            while left < right {
                if left % 2 == 1 { bounds = bounds.merge(self.tree[left]); left += 1; }
                if right % 2 == 1 { right -= 1; bounds = bounds.merge(self.tree[right]); }
                left /= 2; right /= 2;
            }
        }
        for &value in &samples[edge_end.max(full_end)..end] { bounds = bounds.add(value); }
        Some((bounds.low, bounds.high))
    }

    #[cfg(test)]
    pub(super) fn allocated_bytes(&self) -> usize { self.tree.len() * std::mem::size_of::<Extrema>() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_ranges_match_exact_scan_at_block_edges_and_tail() {
        for length in [1, 2, 255, 256, 257, 511, 512, 513, 1025, 4099] {
            let mut values: Vec<_> = (0..length).map(|i| ((i * 71 % 199) as f64 - 99.) / 17.).collect();
            for edge in [0, 255, 256, 257, length - 1] {
                if edge < length { values[edge] = if edge % 2 == 0 { 1000. } else { -2000. }; }
            }
            let index = ExtremaIndex::new(&values).unwrap();
            for seed in 0..700 {
                let start = seed * 157 % length;
                let end = (start + 1 + seed * 29 % (length - start)).min(length);
                let exact = values[start..end].iter().fold(Extrema::EMPTY, |bounds, &value| bounds.add(value));
                assert_eq!(index.range(&values, start, end), Some((exact.low, exact.high)), "length={length}, {start}..{end}");
            }
            let cloned = index.clone();
            assert!(Arc::ptr_eq(&index.tree, &cloned.tree));
            assert!(index.range(&values, length, length).is_none());
            assert!(index.range(&values, 0, length + 1).is_none());
            assert!(index.range(&[], 0, 1).is_none());
        }
    }

    #[test]
    fn invalid_samples_are_rejected_and_large_index_is_small() {
        assert!(ExtremaIndex::new(&[]).is_err());
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] { assert!(ExtremaIndex::new(&[0., value]).is_err()); }
        let values = vec![3.5; 1_000_003];
        let index = ExtremaIndex::new(&values).unwrap();
        assert_eq!(index.range(&values, 1, values.len() - 1), Some((3.5, 3.5)));
        assert!(index.allocated_bytes() < values.len() * std::mem::size_of::<f64>() / 25);
    }
}
