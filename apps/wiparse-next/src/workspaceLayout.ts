/** Shared logical-pixel constraints; never shrink the tool rail below 200px. */
export const RAIL_MIN = 200;
export const RAIL_MAX = 480;
export const DIVIDER_WIDTH = 6;
export const CONTENT_MIN = 240;
export function railMaximum(container: number): number {
  return Math.max(RAIL_MIN, Math.min(RAIL_MAX, (Number.isFinite(container) ? container : 0) - DIVIDER_WIDTH - CONTENT_MIN));
}
export function railWidth(preferred: number, container: number): number {
  return Math.max(RAIL_MIN, Math.min(railMaximum(container), Number.isFinite(preferred) ? preferred : RAIL_MIN));
}
export function restoreRail(value: string | null): number {
  const n = value === null ? NaN : Number(value);
  return Number.isFinite(n) && n >= RAIL_MIN && n <= RAIL_MAX ? n : RAIL_MIN;
}
