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
