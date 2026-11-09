import test from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import {apiInvoke,isLocalBridge,localHeaders} from '../dist/http.js';

const token='00000000-0000-4000-8000-000000000001';
test('instance credentials stay confined to the fixed local bridge',()=>{
  assert.deepEqual(localHeaders('http://127.0.0.1:7878',{WIPARSE_LOCAL_API_TOKEN:token}),{Authorization:`Bearer ${token}`});
  for(const url of ['https://127.0.0.1:7878','http://127.0.0.1:7879','http://localhost:7878','http://user@127.0.0.1:7878','http://127.0.0.1:7878/other','https://example.invalid']){
    assert.equal(isLocalBridge(url),false,url);assert.deepEqual(localHeaders(url,{WIPARSE_LOCAL_API_TOKEN:token}),{},url);
  }
  assert.deepEqual(localHeaders('http://127.0.0.1:7878',{WIPARSE_LOCAL_API_TOKEN:'invalid\ncredential'}),{});
});

test('uncertain HTTP failures never replay a device operation',async()=>{
  let calls=0;const server=http.createServer(async(req,res)=>{for await(const _chunk of req){}calls++;res.writeHead(500,{'content-type':'application/json'});res.end('{"ok":false}');});
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
  try{await assert.rejects(apiInvoke('instrument.command',{command:'ScopeSingle'},`http://127.0.0.1:${server.address().port}`),/HTTP 500.*unknown/);assert.equal(calls,1);}
  finally{server.closeAllConnections();await new Promise(resolve=>server.close(resolve));}
});

test('device calls never follow redirects or forward credentials to another server',async()=>{
  let redirected=0;const target=http.createServer((_req,res)=>{redirected++;res.end('{"ok":true}');});
  await new Promise(resolve=>target.listen(0,'127.0.0.1',resolve));
  const server=http.createServer((_req,res)=>{res.writeHead(307,{location:`http://127.0.0.1:${target.address().port}/v1/invoke`});res.end();});
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
  try{await assert.rejects(apiInvoke('instrument.command',{},`http://127.0.0.1:${server.address().port}`));assert.equal(redirected,0);}
  finally{server.closeAllConnections();target.closeAllConnections();await Promise.all([new Promise(resolve=>server.close(resolve)),new Promise(resolve=>target.close(resolve))]);}
});
