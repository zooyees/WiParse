export type ResonancePoint = [number, number];

/** Points already produced by the existing resonance curve. Reject anything else. */
export function resonancePoints(value: unknown): ResonancePoint[] {
  if (!Array.isArray(value) || value.length < 2 || value.length > 802) return [];
  const points: ResonancePoint[] = [];
  for (const item of value) {
    if (!Array.isArray(item) || item.length !== 2) return [];
    const [frequency, current] = item;
    if (typeof frequency !== 'number' || typeof current !== 'number' || !Number.isFinite(frequency) || !Number.isFinite(current) || current < 0) return [];
    if (points.length && frequency < points[points.length - 1][0]) return [];
    points.push([frequency, current]);
  }
  return points;
}
