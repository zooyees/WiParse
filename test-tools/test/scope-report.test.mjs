import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import vm from 'node:vm';

test('scope report nests captures under the session and opens evidence through portable links', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'wiparse-scope-report-'));
  try {
    const reports = path.join(root, 'reports'), docs = path.join(root, 'docs');
    fs.mkdirSync(reports); fs.mkdirSync(docs);
    const summary = path.join(reports, 'summary.md');
    const source = fs.readFileSync(new URL('../plugins/scope-serial-monitor/loop.mjs', import.meta.url), 'utf8');
    const writers = source.slice(source.indexOf('function mdCell('), source.indexOf('function writePdfReport('));
    const sandbox = {fs, path, summaryMd: summary, reportDir: reports, docsDir: docs,
      cfg: {sop: {title: '模拟测试', id: 'SIM', rev: '1'}, station: {id: 'SIM', name: '模拟工位', product: 'SIM'}},
      port: 'SIM', baud: 115200, prefix: 'scope', sessionStamp: '20261016_120000',
      compiledTriggers: [{label: 'PING'}], stampNice: () => '2026-10-16 12:00:00'};
    vm.createContext(sandbox); vm.runInContext(writers, sandbox);
    sandbox.appendSessionBanner({guiVersion: '1.1.30', scopeModel: 'SIM', scopeId: 'SIM'});
    for (const cycle of [1, 2]) {
      const stamp = `20261016_12000${cycle}`;
      const wave = path.join(root, `波形 (${cycle}).isf`), shot = path.join(root, `图片 (${cycle}).png`);
      fs.writeFileSync(wave, 'sim'); fs.writeFileSync(shot, 'sim');
      fs.writeFileSync(path.join(docs, `scope_PING_${stamp}.json`), '{}');
      fs.writeFileSync(path.join(docs, 'index.jsonl'), '{}\n');
      sandbox.appendSummaryRecord({cycle, stamp, trigger: 'PING', timeText: stamp,
        line: 'PING', context: ['PING'], isfPath: wave, screenshot: shot, pdfPath: null});
    }
    const md = fs.readFileSync(summary, 'utf8');
    assert.match(md, /^# 模拟测试/m);
    assert.match(md, /^## 会话 SESSION-/m);
    assert.equal([...md.matchAll(/^### 第 /gm)].length, 2);
    assert.equal([...md.matchAll(/^#### Log 上下文/gm)].length, 2);
    assert(!md.includes('!['), 'screenshots open by link without filling the report body');
    const urls = [...md.matchAll(/\]\(((?:[^()]|\([^()]*\))*)\)/g)].map(m => m[1]);
    assert.equal(urls.length, 8, 'each capture links waveform, screenshot, record and index');
    for (const url of urls) assert(fs.existsSync(path.resolve(reports, decodeURIComponent(url))));
  } finally { fs.rmSync(root, {recursive: true, force: true}); }
});
