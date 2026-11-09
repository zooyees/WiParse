import test from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import {createClient,localApiHeaders} from '../lib/wiparse-sdk.mjs';

test('local bridge credentials stay with the exact loopback authority',()=>{
  const env={WIPARSE_LOCAL_API_TOKEN:'aabbccdd-1234-4567-8901-aabbccddeeff'};
  assert.ok(localApiHeaders('http://127.0.0.1:7878',env).Authorization);
  for(const target of ['https://example.invalid','http://127.0.0.1:7879','http://127.0.0.1:7878@evil.invalid','file:///C:/a'])assert.deepEqual(localApiHeaders(target,env),{});
});
test('an uncertain HTTP command failure is never replayed through the CLI',async t=>{
  let requests=0;
  const server=http.createServer((request,response)=>{requests++;response.writeHead(500,{'Content-Type':'application/json'});response.end(JSON.stringify({ok:false,error:'result unknown'}));});
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));t.after(()=>server.close());
  const client=createClient({url:`http://127.0.0.1:${server.address().port}`,cliPath:'this-cli-must-not-be-launched'});
  await assert.rejects(client.api.invoke('instrument.command',{device_id:1,command:'ScopeStop'}),/HTTP 500/);
  assert.equal(requests,1);
});
