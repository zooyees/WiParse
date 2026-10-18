import fs from 'node:fs';
import path from 'node:path';
import vm from 'node:vm';
import {fileURLToPath} from 'node:url';
const repo=path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const run=path.resolve(process.argv[2]);
const meta=JSON.parse(fs.readFileSync(path.join(run,'demo-input.json'),'utf8'));
const guiVersion=fs.readFileSync(path.join(repo,'Cargo.toml'),'utf8').match(/^version\s*=\s*"([^"]+)"/m)[1];
const artifacts=path.join(run,'artifacts');
const summary=path.join(artifacts,'reports',`ScopeSerialDemo_summary_${meta.stamp}.md`);
if (meta.simulation_only !== true || !/^\d{8}_\d{6}$/.test(meta.stamp)) throw new Error('Expected a synthetic demo run');
for (const record of meta.records) record.guiVersion=`${guiVersion} (模拟)`;
fs.writeFileSync(path.join(run,'demo-input.json'),JSON.stringify(meta,null,2)+'\n');
// Rebuild this generated report rather than append duplicate sessions on reruns.
fs.rmSync(summary,{force:true});
const source=fs.readFileSync(path.join(repo,'test-tools/plugins/scope-serial-monitor/loop.mjs'),'utf8');
// Execute the existing report writer without starting the hardware capture loop.
const writers=source.slice(source.indexOf('function mdCell('),source.indexOf('function writePdfReport('));
const sandbox={fs,path,cfg:{sop:{id:'WI-SCOPE-SERIAL-DEMO',rev:'SIM-1',title:'示波器/串口监控 · 模拟效果检查'},station:{id:'SIM-STATION-001',name:'离线演示工位（非实测）',product:'无线充电器模拟样机'}},
 reportDir:path.dirname(summary),docsDir:path.join(artifacts,'docs'),summaryMd:summary,port:'SIM-COM7（虚拟）',baud:2000000,
 prefix:'ScopeSerialDemo',sessionStamp:meta.stamp,compiledTriggers:meta.records.map(r=>({label:r.trigger})),stampNice:()=>`${meta.stamp}（模拟）`};
vm.createContext(sandbox);
vm.runInContext(writers,sandbox);
sandbox.appendSessionBanner({guiVersion,scopeModel:'SIMULATED OSCILLOSCOPE',scopeId:'SIM-SCOPE-001'});
for(const rec of meta.records) sandbox.appendSummaryRecord(rec);
const lines=meta.records.flatMap(r=>r.context);
const log=path.join(artifacts,'docs','serial-monitor.log');
fs.writeFileSync(log,lines.join('\n')+'\n');
const data=path.join(artifacts,'docs','capture-records.json');
fs.writeFileSync(data,JSON.stringify(meta,null,2)+'\n');
const rel=p=>path.relative(path.dirname(summary),p).split(path.sep).map(encodeURIComponent).join('/');
fs.appendFileSync(summary,`\n---\n\n## 模拟场景说明与原始数据\n\n**本报告全部内容为离线合成数据。未连接串口或示波器，不代表产品实测或合格结论。**\n\n三轮分别演示 PING、正常功率传输和 VIN 欠压后的 PWM 关闭。频率和动作时序为预设值，未执行实测指标判定。PDF 未生成。\n\n- [完整串口日志](${rel(log)})\n- [采集记录与模拟参数](${rel(data)})\n`+meta.records.map(r=>`- 第 ${r.cycle} 轮 [原始 CSV 数据](${rel(r.csvPath)})`).join('\n')+'\n');
const items=['reports','waves','shots','docs'].flatMap(output=>fs.readdirSync(path.join(artifacts,output)).map(name=>({output,path:path.join(artifacts,output,name),view:output==='reports'?'report':output==='waves'?'wave':output==='docs'?'log':'external'})));
fs.writeFileSync(path.join(run,'run.json'),JSON.stringify({plugin:'scope-serial-monitor',plugin_id:'scope-serial-monitor',project:'default',test_id:'ScopeSerialDemo',stamp:meta.stamp,ok:true,simulation_only:true,product_verdict:'INCONCLUSIVE',summary_md:summary,artifacts:{items}},null,2));
const text=fs.readFileSync(summary,'utf8');
const links=[...text.matchAll(/\]\((<[^>]*>|(?:[^()]|\([^()]*\))*)\)/g)].map(m=>m[1].replace(/^<|>$/g,''));
for(const link of links) if(!fs.existsSync(path.resolve(path.dirname(summary),decodeURIComponent(link)))) throw new Error(`Missing link: ${link}`);
console.log(JSON.stringify({summary_md:summary,run_dir:run,cycles:meta.records.length,verified_links:links.length,simulation_only:true},null,2));
