export const LIVE_CATCHUP = 2000;

/** Frame rules identify an analysis session. Presentation scale/units do not. */
export function liveRulesKey(header: string, filters: readonly string[]): string {
  return JSON.stringify([header.trim(), filters.map(value=>value.trim())]);
}

/** Same first read as the original serial-live panel: skip a huge backlog, and never read evicted rows. */
export function liveStartRow(cursor: number | null, total: number, dropped: number): number {
  if (!Number.isSafeInteger(total) || !Number.isSafeInteger(dropped) || total < 0 || dropped < 0 || dropped > total) return 0;
  let row = cursor ?? 0;
  if (!Number.isSafeInteger(row) || row < 0) row = 0;
  if (row < dropped) row = dropped;
  if ((cursor === null || cursor === 0) && total > LIVE_CATCHUP) row = Math.max(row, total - LIVE_CATCHUP);
  return Math.min(row, total);
}
