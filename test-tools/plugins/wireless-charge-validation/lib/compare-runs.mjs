import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

function readRun(directory) {
  const manifest = JSON.parse(fs.readFileSync(path.join(directory, 'manifest.json'), 'utf8'));
  const bytes = fs.readFileSync(path.join(directory, 'checks.json'));
  const expected = manifest.files.find(file => file.path === 'checks.json')?.sha256;
  if (createHash('sha256').update(bytes).digest('hex') !== expected) throw Error('CHECKS.HASH_MISMATCH');
  const run = JSON.parse(bytes);
  if (!run.run_id || run.run_id !== manifest.run_id || run.attempt_id !== manifest.attempt_id)
    throw Error('CHECKS.ATTEMPT_MISMATCH');
  return run;
}

function index(run) {
  const checks = new Map();
  for (const scenario of run.cases) {
    for (const check of scenario.checks) {
      const key = JSON.stringify([scenario.case_id, check.check_id]);
      if (checks.has(key)) throw Error('CHECKS.DUPLICATE');
      checks.set(key, check);
    }
  }
  return checks;
}

export function compareRuns(beforeDir, afterDir) {
  const before = readRun(beforeDir), after = readRun(afterDir);
  const left = index(before), right = index(after);
  const configurationKeys = new Set([...Object.keys(before.configuration_hashes), ...Object.keys(after.configuration_hashes)]);
  const changed = [...configurationKeys].filter(k => before.configuration_hashes[k] !== after.configuration_hashes[k]);
  const rows = [];
  for (const key of new Set([...left.keys(), ...right.keys()])) {
    const [case_id, check_id] = JSON.parse(key);
    const a = left.get(key), b = right.get(key);
    const comparable = Boolean(a && b && before.mode === after.mode && !changed.length &&
      a.scope === b.scope && a.limits_hash === b.limits_hash && a.state_window === b.state_window);
    const x = a?.measurement, y = b?.measurement;
    const scalar = comparable && x && y && x.metric_id === y.metric_id && x.unit === y.unit &&
      x.channel === y.channel && JSON.stringify(x.window) === JSON.stringify(y.window) &&
      [a.product_verdict,b.product_verdict].every(v=>v==='PASS'||v==='FAIL') &&
      Number.isFinite(x.value) && Number.isFinite(y.value);
    rows.push({case_id, check_id, comparable, before:a?.product_verdict ?? null,
      after:b?.product_verdict ?? null, delta:scalar ? y.value-x.value : null, unit:scalar ? x.unit : null});
  }
  return {before_attempt:before.attempt_id,after_attempt:after.attempt_id,changed_configuration:changed,
    mode_before:before.mode,mode_after:after.mode,industrial_approved:false,rows};
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  if (process.argv.length !== 4) throw Error('Usage: node compare-runs.mjs BEFORE_REPORT_DIR AFTER_REPORT_DIR');
  process.stdout.write(JSON.stringify(compareRuns(process.argv[2], process.argv[3]),null,2)+'\n');
}
