import assert from 'node:assert/strict';
import fs from 'node:fs';
import http from 'node:http';
import {fileURLToPath} from 'node:url';
import {Client} from '@modelcontextprotocol/sdk/client/index.js';
import {StdioClientTransport} from '@modelcontextprotocol/sdk/client/stdio.js';

const version=JSON.parse(fs.readFileSync(new URL('../package.json',import.meta.url))).version;
const received=[];
const mock=http.createServer(async(req,res)=>{
  let text='';for await(const chunk of req) text+=chunk;
  const body=text?JSON.parse(text):null;
  if(body)received.push(body);
  res.setHeader('Content-Type','application/json');
  res.end(JSON.stringify({ok:true,data:req.url==='/v1/health'?{version}:{method:body?.method,params:body?.params}}));
});
await new Promise(resolve=>mock.listen(0,'127.0.0.1',resolve));
const transport=new StdioClientTransport({command:process.execPath,args:[fileURLToPath(new URL('../dist/index.js',import.meta.url))],env:{...process.env,WIPARSE_URL:`http://127.0.0.1:${mock.address().port}`},stderr:'pipe'});
const client=new Client({name:'wiparse-release-smoke',version:'1.0.0'});
try {
  await client.connect(transport);
  assert.equal(client.getServerVersion().version,version);
  const list=await client.listTools();assert.equal(list.tools.length,6);
  assert(list.tools.some(t=>t.name==='wiparse_ui'));
  for(const [op,params,method] of [
    ['show',{tab:'test_report'},'ui.show'],
    ['project.select',{id:'default'},'ui.project.select'],
    ['report.open',{path:'C:/demo/projects/default/tests/ScopeSerialDemo/runs/20261013_231510/artifacts/reports/summary.md'},'ui.report.open'],
    ['report.open',{path:'C:/demo/projects/default/tests/ScopeSerialDemo/runs/20261013_231510/artifacts/docs/checks.json'},'ui.report.open'],
  ]) {
    const reply=await client.callTool({name:'wiparse_ui',arguments:{op,params}});
    assert.equal(JSON.parse(reply.content[0].text).ok,true);
    assert.deepEqual(received.at(-1),{method,params});
  }
  const hubOps=['config.list','config.get','config.create','config.save','config.duplicate','config.archive','config.restore','config.import','config.export','config.batch','config.archives','config.validate','matrix.get','preflight.state','project.get','project.save','project.validate','project.migration.preview','project.migration.commit','result.get'];
  for(const op of hubOps){const params={plugin:'hub-workspace-demo',limit:5};const reply=await client.callTool({name:'wiparse_ui',arguments:{op:`test_tool.${op}`,params}});assert.equal(JSON.parse(reply.content[0].text).ok,true);assert.deepEqual(received.at(-1),{method:`ui.test_tool.${op}`,params});}
  console.log(JSON.stringify({ok:true,mcp_version:version,tool_count:list.tools.length,report_dispatch_checks:4,hub_dispatch_checks:hubOps.length,backend:'local mock; no hardware'},null,2));
} finally {
  await client.close();await transport.close();
  await new Promise(resolve=>mock.close(resolve));
}
