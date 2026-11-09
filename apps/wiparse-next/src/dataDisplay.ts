export type DataKind = 'voltage' | 'current' | 'temperature';

const REF_SPAN: Record<DataKind, number> = {voltage: 10_000, current: 5_000, temperature: 50};
const BAND_SPACING = 1.2;
const ORDER: DataKind[] = ['voltage', 'current', 'temperature'];

/** Same mapping as the old data page: `raw * (scale / ref_span) + offset`. */
export function displayY(raw: number, scale: number, offset: number, kind: DataKind): number {
  return raw * (scale / REF_SPAN[kind]) + offset;
}

export function kindUnit(kind: DataKind): string {
  if (kind === 'current') return 'mA';
  if (kind === 'temperature') return '°C';
  return 'mV';
}

/** Stack active series by kind, voltage then current then temperature, 1.2 apart. */
export function autoLayout(series: {kind: DataKind; active: boolean}[]): {scale: number; offset: number}[] {
  const groups: number[][] = [[], [], []];
  series.forEach((item, index) => {
    if (item.active) groups[ORDER.indexOf(item.kind)].push(index);
  });
  const layout = series.map(() => ({scale: 1, offset: 0}));
  let band = 0;
  for (const group of groups) {
    if (!group.length) continue;
    group.forEach((index, place) => { layout[index] = {scale: 1, offset: band + place * BAND_SPACING}; });
    band += group.length * BAND_SPACING + BAND_SPACING;
  }
  return layout;
}

export function displayExtent(series: {minimum: number | null; maximum: number | null; kind: DataKind; scale: number; offset: number}[]): {min: number; max: number} | null {
  let minimum = Infinity;
  let maximum = -Infinity;
  for (const seriesItem of series) {
    if (seriesItem.minimum === null || seriesItem.maximum === null) continue;
    const low = displayY(seriesItem.minimum, seriesItem.scale, seriesItem.offset, seriesItem.kind);
    const high = displayY(seriesItem.maximum, seriesItem.scale, seriesItem.offset, seriesItem.kind);
    minimum = Math.min(minimum, low, high);
    maximum = Math.max(maximum, low, high);
  }
  if (!Number.isFinite(minimum)) return null;
  if (Math.abs(maximum - minimum) < 1e-12) {
    maximum = minimum + 1;
    minimum -= 0.5;
  }
  return {min: minimum, max: maximum};
}

export function fitY(extent: {min: number; max: number}): {min: number; max: number} {
  const pad = Math.max(Math.abs(extent.max - extent.min) * 0.08, 1e-12);
  return {min: extent.min - pad, max: extent.max + pad};
}

/** Vertical drag uses the same sign as the old axis strip: pointer down lowers the offset. */
export function dragOffset(offset: number, dy: number, height: number, span: number): number {
  if (!(height > 0) || !Number.isFinite(dy) || !Number.isFinite(span) || !Number.isFinite(offset)) return offset;
  return offset - (dy / height) * span;
}

/** Line-mode wheel notches use the old 120 px notch. Pixel mode keeps the reported distance. */
export function wheelPixels(deltaY: number, mode: number): number {
  if (!Number.isFinite(deltaY) || deltaY === 0) return 0;
  return mode === 0 ? deltaY : deltaY * 120;
}

/** Shift moves the offset by 0.08 of the span per 120 px. Otherwise the scale follows the old exponential. */
export function wheelAdjust(current: {offset: number; scale: number}, deltaY: number, span: number, shift: boolean): {offset?: number; scale?: number} | null {
  if (!Number.isFinite(deltaY) || deltaY === 0) return null;
  if (shift) {
    if (!Number.isFinite(span) || !(Math.abs(span) > 0) || !Number.isFinite(current.offset)) return null;
    return {offset: current.offset + -(deltaY / 120) * span * 0.08};
  }
  if (!Number.isFinite(current.scale)) return null;
  return {scale: Math.min(1e12, Math.max(1e-12, current.scale * Math.exp(deltaY * 0.002)))};
}

/** Ctrl-drag uses the old exponential scale, clamped to the original limits. */
export function dragScale(scale: number, dy: number): number {
  if (!Number.isFinite(dy) || !Number.isFinite(scale)) return scale;
  return Math.min(1e12, Math.max(1e-12, scale * Math.exp(-dy * 0.008)));
}

export function nearestHandle(points: {x: number; y: number; active: boolean}[], pointerX: number, pointerY: number, radius = 14): number | null {
  let best: number | null = null;
  let bestDistance = radius * radius;
  points.forEach((point, index) => {
    if (!point.active) return;
    const dx = pointerX - point.x;
    const dy = pointerY - point.y;
    const distance = dx * dx + dy * dy;
    if (distance < bestDistance) {
      bestDistance = distance;
      best = index;
    }
  });
  return best;
}

/** Shrink or grow the current window around its center. `factor` below 1 zooms in. */
export function zoomY(view: {min: number; max: number}, factor: number): {min: number; max: number} | null {
  const middle = 0.5 * (view.min + view.max);
  const half = 0.5 * (view.max - view.min) * factor;
  if (!Number.isFinite(middle) || !Number.isFinite(half) || half <= 1e-12) return null;
  return {min: middle - half, max: middle + half};
}

/** Inverse of `displayY`. A vanished scale returns the plot value, matching the old axis. */
export function nativeY(plotY: number, scale: number, offset: number, kind: DataKind): number {
  const factor = scale / REF_SPAN[kind];
  if (!Number.isFinite(plotY) || !Number.isFinite(offset) || Math.abs(factor) < Number.EPSILON) return plotY;
  return (plotY - offset) / factor;
}

function exponential(value: number, digits: number): string {
  const exponent = Math.floor(Math.log10(Math.abs(value)));
  return `${(value / 10 ** exponent).toFixed(digits)}e${exponent}`;
}

/** Short engineering text used by the old per-division header. */
export function formatEngShort(value: number): string {
  if (!Number.isFinite(value)) return '—';
  const abs = Math.abs(value);
  if (abs >= 1000 || (abs > 0 && abs < 0.01)) return exponential(value, 3);
  if (Math.abs(value - Math.round(value)) < 1e-9) return String(Math.round(value));
  return value.toFixed(3);
}

/** Tick label from the old native axis. Units are added only on the end ticks. */
export function formatAxisTick(value: number, unit: string, withUnit: boolean): string {
  if (!Number.isFinite(value)) return '—';
  const abs = Math.abs(value);
  const text = abs >= 1000 || (abs > 0 && abs < 0.01) ? exponential(value, 2) : abs >= 100 ? value.toFixed(0) : abs >= 10 ? value.toFixed(1) : value.toFixed(2);
  return withUnit && unit ? `${text}${unit}` : text;
}

/** Round ticks covering the visible native range, same step rule as the old axis. */
export function niceAxisTicks(low: number, high: number, target: number): number[] {
  const count = Math.max(2, target);
  const span = Math.abs(high - low);
  if (!Number.isFinite(span) || span < 1e-30) return [low];
  const raw = span / (count - 1);
  const exponent = Math.floor(Math.log10(raw));
  const base = 10 ** exponent;
  const fraction = raw / base;
  const nice = fraction <= 1 ? 1 : fraction <= 2 ? 2 : fraction <= 5 ? 5 : 10;
  const step = nice * base;
  let value = Math.floor(low / step) * step;
  const end = high + step * 0.5;
  const ticks: number[] = [];
  for (let guard = 0; value <= end && guard < 64; guard += 1) {
    if (value >= low - step * 1e-6 && value <= high + step * 1e-6) ticks.push(Object.is(value, -0) ? 0 : value);
    value += step;
  }
  return ticks.length ? ticks : [low, high];
}

export type ReadoutSample = {kind: 'value'; value: number} | {kind: 'range'; low: number; high: number};

/** Decimal readout from the old data cursor, without scientific notation. */
export function formatPlain(value: number): string {
  if (!Number.isFinite(value)) return '—';
  const abs = Math.abs(value);
  if (Math.abs(value - Math.round(value)) < 1e-9 && abs < 1e12) return String(Math.round(value));
  if (abs >= 1000) return value.toFixed(1);
  if (abs >= 1) return value.toFixed(3);
  return abs >= 0.001 ? value.toFixed(4) : value.toFixed(6);
}

function bucketSample(bucket: {low: number; high: number}): ReadoutSample {
  return bucket.low === bucket.high ? {kind: 'value', value: bucket.low} : {kind: 'range', low: Math.min(bucket.low, bucket.high), high: Math.max(bucket.low, bucket.high)};
}

/** Cursor sample. Exact plotted points interpolate; a merged bucket stays a min–max range. */
export function sampleAt(buckets: {frame: number; low: number; high: number}[], frame: number): ReadoutSample | null {
  if (!buckets.length || !Number.isFinite(frame)) return null;
  if (frame <= buckets[0].frame) return bucketSample(buckets[0]);
  const last = buckets[buckets.length - 1];
  if (frame >= last.frame) return bucketSample(last);
  let lower = 0;
  let upper = buckets.length - 1;
  while (upper - lower > 1) {
    const middle = Math.floor((lower + upper) / 2);
    if (buckets[middle].frame <= frame) lower = middle;
    else upper = middle;
  }
  const left = buckets[lower];
  const right = buckets[upper];
  if (left.low !== left.high || right.low !== right.high || Math.abs(right.frame - left.frame) < 1e-15) return bucketSample(left);
  const ratio = (frame - left.frame) / (right.frame - left.frame);
  return {kind: 'value', value: left.low + ratio * (right.low - left.low)};
}

/** Frame under a plot pointer. The same mapping the curve uses from the first to the last visible frame. */
export function frameAt(pointer: number, left: number, width: number, start: number, end: number): number | null {
  if (!Number.isFinite(pointer) || !(width > 0) || !(end > start)) return null;
  if (pointer < left || pointer > left + width) return null;
  return start + ((pointer - left) / width) * Math.max(1, end - start - 1);
}

/** Horizontal position of a frame inside the plot. */
export function cursorX(frame: number, left: number, width: number, start: number, end: number): number | null {
  if (!Number.isFinite(frame) || !(width > 0) || !(end > start)) return null;
  const x = left + width * (frame - start) / Math.max(1, end - start - 1);
  return x >= left - 0.5 && x <= left + width + 0.5 ? x : null;
}

/** Closest plotted series at one frame. A merged bucket counts as its vertical span. */
export function nearestSeries(series: {buckets: {frame: number; low: number; high: number}[]; scale: number; offset: number; kind: DataKind; active: boolean}[], frame: number, plotY: number): number | null {
  if (!Number.isFinite(frame) || !Number.isFinite(plotY)) return null;
  let best: number | null = null;
  let bestDistance = Infinity;
  series.forEach((item, index) => {
    if (!item.active) return;
    const sample = sampleAt(item.buckets, frame);
    if (!sample) return;
    const distance = sample.kind === 'value'
      ? Math.abs(displayY(sample.value, item.scale, item.offset, item.kind) - plotY)
      : distanceToSpan(plotY, displayY(sample.low, item.scale, item.offset, item.kind), displayY(sample.high, item.scale, item.offset, item.kind));
    if (distance < bestDistance) {
      bestDistance = distance;
      best = index;
    }
  });
  return best;
}

function distanceToSpan(value: number, first: number, second: number): number {
  const low = Math.min(first, second);
  const high = Math.max(first, second);
  if (value < low) return low - value;
  return value > high ? value - high : 0;
}

/** Eight divisions across the visible native range. */
export function perDivision(low: number, high: number): number | null {
  if (!Number.isFinite(low) || !Number.isFinite(high)) return null;
  return Math.abs(high - low) / 8;
}

/** A threshold uses the first active series of the same kind. */
export function thresholdY(native: number, series: {kind: DataKind; scale: number; offset: number; active: boolean}[], kind: DataKind): number | null {
  if (!Number.isFinite(native)) return null;
  const match = series.find(item => item.kind === kind && item.active);
  return match ? displayY(native, match.scale, match.offset, kind) : null;
}
