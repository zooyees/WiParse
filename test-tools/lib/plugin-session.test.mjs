import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {writeReportWithEvidence} from './plugin-session.mjs';

test('Markdown evidence index contains portable, encoded links without linking the report itself',()=>{
  const root=fs.mkdtempSync(path.join(os.tmpdir(),'wiparse-report-'));
  try {
    const artifacts=path.join(root,'artifacts');
    const reports=path.join(artifacts,'reports'),logs=path.join(artifacts,'logs');
    fs.mkdirSync(reports,{recursive:true});fs.mkdirSync(logs,{recursive:true});
    const md=path.join(reports,'summary.md'),json=path.join(logs,'数据 (1).json');
    fs.writeFileSync(json,'{}');fs.writeFileSync(md,'old');
    const plugin={outputs:[{id:'reports',include:['*.md']},{id:'logs',include:['*.json']}]};
    writeReportWithEvidence(plugin,{artifacts_dir:artifacts,run_dir:root},md,'# Test');
    const text=fs.readFileSync(md,'utf8');
    assert(text.startsWith('# Test'));assert(!text.includes('](summary.md)'));
    const url=text.match(/\]\((.+)\)/)[1];
    assert.equal(path.resolve(reports,decodeURIComponent(url)),json);
    assert(text.includes('Evidence'));assert(!url.includes(' '));
  } finally {fs.rmSync(root,{recursive:true,force:true});}
});

test('report evidence is grouped by purpose while every file remains reachable',()=>{
  const root=fs.mkdtempSync(path.join(os.tmpdir(),'wiparse-report-groups-'));
  try {
    const artifacts=path.join(root,'artifacts'),reports=path.join(artifacts,'reports');
    fs.mkdirSync(reports,{recursive:true});
    const names=['capture.isf','screenshot.png','records.json','serial.log','export.pdf'];
    const docs=path.join(artifacts,'docs'); fs.mkdirSync(docs);
    for(const name of names) fs.writeFileSync(path.join(docs,name),'sim');
    const md=path.join(reports,'summary.md');
    writeReportWithEvidence({outputs:[{id:'docs',include:['*']}]},
      {artifacts_dir:artifacts,run_dir:root},md,'# Report\n\n## Result\n\nSIMULATION');
    const text=fs.readFileSync(md,'utf8');
    assert.equal([...text.matchAll(/^### /gm)].length,4);
    assert.match(text,/### 波形与采样数据/); assert.match(text,/### 图片/);
    assert.match(text,/### 记录与日志/); assert.match(text,/### 其他文件/);
    const urls=[...text.matchAll(/\]\(([^)]+)\)/g)].map(m=>m[1]);
    assert.equal(urls.length,names.length);
    for(const url of urls) assert(fs.existsSync(path.resolve(reports,decodeURIComponent(url))));
  } finally {fs.rmSync(root,{recursive:true,force:true});}
});
