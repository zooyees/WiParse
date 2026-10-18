import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';

export class HubError extends Error {
  constructor(code, detail = '', data = {}) { super(`${code}${detail ? ': ' + detail : ''}`); this.code = code; this.data = data; }
}
export const fail = (code, detail, data) => { throw new HubError(code, detail, data); };
export const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
export const canonical = value => JSON.stringify(value, (_key, val) => val && typeof val === 'object' && !Array.isArray(val) ? Object.fromEntries(Object.keys(val).sort().map(k => [k, val[k]])) : val);
export function id(value) {
  if (typeof value !== 'string' || !/^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/.test(value) || /[.]$/.test(value) || /^(con|prn|aux|nul|com[0-9]|lpt[0-9])(?:\.|$)/i.test(value)) fail('CONFIG.ID_CONFLICT', 'Invalid identifier');
  return value;
}
export function inside(file, root) { const rel = path.relative(root, file); return !rel || (!rel.startsWith('..' + path.sep) && rel !== '..' && !path.isAbsolute(rel)); }
// Inspect every existing component; realpath alone would silently follow a junction.
export function jail(file, roots) {
  const absolute = path.resolve(file);
  let cursor = absolute;
  while (true) {
    try { if (fs.lstatSync(cursor).isSymbolicLink()) fail('CONFIG.PATH_OUTSIDE_ROOT', 'Links and junctions are not allowed'); }
    catch (e) { if (e.code !== 'ENOENT') throw e; }
    const parent = path.dirname(cursor); if (parent === cursor) break; cursor = parent;
  }
  const canonicalize = p => {
    let existing = path.resolve(p); const tail = [];
    while (!fs.existsSync(existing)) { tail.unshift(path.basename(existing)); const parent = path.dirname(existing); if (parent === existing) break; existing = parent; }
    return path.join(fs.realpathSync.native(existing), ...tail);
  };
  const actual = canonicalize(absolute);
  if (!roots.some(root => inside(actual, canonicalize(root)))) fail('CONFIG.PATH_OUTSIDE_ROOT', 'Path is outside the approved roots');
  return absolute;
}
export function readJson(file, max = 1048576) {
  const stat = fs.statSync(file); if (!stat.isFile() || stat.size > max) fail('CONFIG.REPOSITORY_LIMIT', 'File exceeds the size limit');
  return JSON.parse(fs.readFileSync(file, 'utf8').replace(/^\uFEFF/, ''));
}
export function flushDir(dir) {
  let fd; try { fd = fs.openSync(dir, 'r'); fs.fsyncSync(fd); }
  catch (e) { if (process.platform !== 'win32' || !['EPERM','EACCES','EINVAL','EISDIR'].includes(e.code)) throw e; }
  finally { if (fd !== undefined) fs.closeSync(fd); }
}
// Node/libuv rename uses replacement semantics on Windows; never unlink the target first.
export function atomicWrite(file, bytes) {
  fs.mkdirSync(path.dirname(file), {recursive:true});
  const tmp = path.join(path.dirname(file), `.${path.basename(file)}.${crypto.randomUUID()}.tmp`);
  const fd = fs.openSync(tmp, 'wx', 0o600);
  try { fs.writeFileSync(fd, bytes); fs.fsyncSync(fd); } finally { fs.closeSync(fd); }
  try { fs.renameSync(tmp, file); flushDir(path.dirname(file)); }
  catch (e) { try { fs.unlinkSync(tmp); } catch {} throw e; }
}
export function appendSync(file, text) {
  fs.mkdirSync(path.dirname(file), {recursive:true}); const fd = fs.openSync(file, 'a', 0o600);
  try { fs.writeFileSync(fd, text); fs.fsyncSync(fd); } finally { fs.closeSync(fd); }
}
export function redact(value, key = '') {
  if (/secret|password|token|(?:^|_)key(?:$|_)/i.test(key)) return '[REDACTED]';
  if (Array.isArray(value)) return value.map(v => redact(v, key));
  if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value).map(([k,v]) => [k,redact(v,k)]));
  if (typeof value === 'string' && (/^[A-Za-z]:[\\/]/.test(value) || value.startsWith('/') || value.startsWith('\\\\'))) return '[LOCAL_PATH]';
  return value;
}
export function page(items, params = {}) {
  const offset = Math.floor(Math.max(0, Math.min(500, Number(params.offset) || 0)));
  const limit = Math.floor(Math.max(1, Math.min(500, Number(params.limit) || 100)));
  const query = String(params.query || '').toLowerCase(); const tags = params.tags || []; const statuses = params.statuses || [];
  if(!Array.isArray(tags)||!Array.isArray(statuses)||tags.length>32||statuses.length>8)fail('CONFIG.INVALID_FILTER');
  let rows = items.filter(item => (!query || `${item.name} ${item.id} ${(item.tags || []).join(' ')}`.toLowerCase().includes(query)) && (!statuses.length || statuses.includes(item.status)) && (!tags.length || tags.some(t => item.tags?.includes(t))));
  const sort = ['name','updated_at','order','id','status'].includes(params.sort) ? params.sort : 'order';
  rows.sort((a,b) => (typeof a[sort] === 'number' ? a[sort]-b[sort] : String(a[sort] ?? '').localeCompare(String(b[sort] ?? ''))) * (params.direction === 'desc' ? -1 : 1) || a.id.localeCompare(b.id));
  return {total:items.length, filtered:rows.length, offset, limit, items: rows.slice(offset,offset+limit)};
}
