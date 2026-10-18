import fs from 'node:fs';
import path from 'node:path';
import { createHash, randomUUID } from 'node:crypto';
export const hash = value => createHash('sha256').update(value).digest('hex');
export const digest = value => hash(JSON.stringify(value));
export const utc = () => new Date().toISOString();
export const fail = (code, detail = '') => Object.assign(new Error(`${code}: ${detail}`), { code });
export function atomic(file, value, replace = fs.renameSync, pause = ms => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms)) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const tmp = `${file}.${randomUUID()}.tmp`;
  const fd = fs.openSync(tmp, 'wx');
  try { fs.writeFileSync(fd, JSON.stringify(value, null, 2)); fs.fsyncSync(fd); } finally { fs.closeSync(fd); }
  try {
    // Windows readers/antivirus may briefly deny replacement. Never unlink the target.
    for (let attempt=0;;attempt++) {
      try { replace(tmp,file); break; }
      catch(error) {
        if(attempt>=29 || !['EPERM','EACCES','EBUSY'].includes(error.code)) throw error;
        pause(20);
      }
    }
  } finally { if (fs.existsSync(tmp)) fs.unlinkSync(tmp); }
}
export function jailed(root, relative) {
  const base = fs.realpathSync(root), target = fs.realpathSync(path.resolve(root, relative));
  const rel = path.relative(base, target);
  if (rel.startsWith('..') || path.isAbsolute(rel)) throw fail('CONFIG.PATH_ESCAPE', relative);
  return target;
}
export function append(file, value) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const fd = fs.openSync(file, 'a');
  try { fs.writeSync(fd, JSON.stringify(value) + '\n'); fs.fsyncSync(fd); } finally { fs.closeSync(fd); }
}
export const bool = v => v === true || v === 'true';
