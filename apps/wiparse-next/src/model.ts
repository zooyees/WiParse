export type RecordValue = Record<string, unknown>;
export type MatrixRow = { tags?:string[];order?:number;updatedAt?:string;id: string; name: string; status: string; phase: string; elapsed: number | null; message: string; stale: boolean; attempt: number | null; validation: string[]; revision: number | null; sha256: string; enabled: boolean | null };
export const object = (value: unknown): RecordValue => value && typeof value === 'object' && !Array.isArray(value) ? value as RecordValue : {};
const list = (value: unknown): unknown[] => Array.isArray(value) ? value : [];
export const string = (value: unknown, fallback = '') => typeof value === 'string' ? value : fallback;
export function matchesStatus(row: MatrixRow, filter: string): boolean {
  if (filter==='all') return true;
  if (filter==='invalid') return row.status==='invalid';
  if (filter==='stale') return row.stale && row.status!=='invalid';
  if (row.stale) return false;
  if (filter==='fail') return ['fail','error'].includes(row.status);
  if (filter==='pending') return ['pending','tbt'].includes(row.status);
  return row.status===filter;
}
export function phaseLabel(phase: string): string {
  return ({finished:'已结束',running:'执行中',pending:'待执行',preflight:'预检中',error:'异常',aborted:'已中止'} as Record<string,string>)[phase] ?? (phase || '—');
}

function validationMessage(value: unknown): string {
  const error=object(value), params=object(error.params), path=string(error.path,'/');
  const keyword=string(error.keyword), field=string(params.missingProperty || params.additionalProperty);
  const description=keyword==='additionalProperties' ? `不支持的字段${field ? `：${field}` : ''}`
    : keyword==='required' ? `缺少必填字段${field ? `：${field}` : ''}`
    : keyword==='const' ? '字段值与当前版本要求不一致'
    : keyword==='if' ? '关联字段不符合要求'
    : string(error.message,'配置无效');
  return `${path}：${description}`;
}

/** Bound the amount of DOM work independently of the number of test cases. */
export function visibleRange(total: number, scrollTop: number, viewportHeight: number, rowHeight = 36, overscan = 4) {
  const start = Math.max(0, Math.min(total, Math.floor(Math.max(0, scrollTop) / rowHeight) - overscan));
  const end = Math.min(total, Math.ceil((Math.max(0, scrollTop) + Math.max(0, viewportHeight)) / rowHeight) + overscan);
  return { start, end: Math.max(start, end), before: start * rowHeight, after: (total - Math.max(start, end)) * rowHeight };
}

export function buildRows(config: unknown, matrix: unknown): MatrixRow[] {
  const cfg = object(config), result = object(matrix);
  const cases = list(cfg.items ?? cfg.configs ?? cfg.rows);
  const runRows = list(result.items ?? object(result.matrix).items);
  const byId = new Map(runRows.map((entry) => { const row = object(entry); return [string(row.id), row] as const; }));
  // The matrix is a historical record, not the configuration library. An
  // empty or unreadable library must never resurrect old test items as current.
  const source = cases;
  return source.flatMap((entry) => {
    const row = object(entry), id = string(row.id ?? row.config_id);
    if (!id) return [];
    const run = byId.get(id) ?? {};
    const rawStatus = row.status==='invalid' ? 'invalid' : matrix === null || result.available === false ? 'unknown' : string(run.status,'pending').toLowerCase();
    const elapsed = typeof run.elapsed_s === 'number' && Number.isFinite(run.elapsed_s) ? run.elapsed_s : null;
    const configuredHash=string(row.sha256), testedHash=string(run.config_sha256);
    const stale=run.stale===true || result.error==='MATRIX.STALE_RUN' || result.error==='MATRIX.INTERRUPTED_RUN' || (!!configuredHash && !!testedHash && configuredHash!==testedHash);
    const validation=list(row.errors).slice(0,5).map(validationMessage);
    const attempt=typeof run.attempt==='number' && Number.isInteger(run.attempt) && run.attempt>=0 ? run.attempt : null;
    const revision=typeof row.revision==='number'&&Number.isSafeInteger(row.revision)&&row.revision>0 ? row.revision : null;
    return [{ tags:list(row.tags).filter((tag):tag is string=>typeof tag==='string'),order:typeof row.order==='number'?row.order:0,updatedAt:string(row.updated_at),id, name: string(row.name ?? row.title, id), status: rawStatus, phase: string(run.phase), elapsed, message: string(run.message), stale, attempt, validation, revision, sha256:configuredHash, enabled:typeof row.enabled==='boolean'?row.enabled:null }];
  });
}

export function statusLabel(status: string, stale: boolean): string {
  if (status === 'invalid') return '配置无效';
  if (stale) return '结果已过期';
  return ({unknown:'状态未知',invalid:'配置无效',pending:'待测试',tbt:'待测试',running:'测试中',testing:'测试中',pass:'历史通过',fail:'历史失败',error:'错误',aborted:'已中止',skipped:'已跳过',inconclusive:'无法判定'} as Record<string,string>)[status] ?? status;
}

export function formatTime(value: number | null): string { return value === null ? '—' : `${value.toFixed(1)} s`; }
