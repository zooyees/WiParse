import test from 'node:test';
import assert from 'node:assert/strict';
import {acknowledgeOperation,beginOperation,finishOperation,hasPendingOperation} from '../src/deviceInterlock.ts';

function storage(){
  const values=new Map();
  return {getItem:key=>values.get(key)??null,setItem:(key,value)=>values.set(key,value),removeItem:key=>values.delete(key)};
}

test('pending operation survives reopening and blocks another command until verified',()=>{
  const data=storage();
  beginOperation(data,'source','USB::SOURCE','first');
  assert.equal(hasPendingOperation(data,'source','usb::source'),true);
  assert.throws(()=>beginOperation(data,'source','USB::SOURCE','second'),/尚未核对/);
  acknowledgeOperation(data,'source','USB::SOURCE');
  assert.equal(hasPendingOperation(data,'source','USB::SOURCE'),false);
});

test('a late completion cannot remove a newer recovery marker',()=>{
  const data=storage();
  beginOperation(data,'load','USB::LOAD','old');
  acknowledgeOperation(data,'load','USB::LOAD');
  beginOperation(data,'load','USB::LOAD','new');
  assert.throws(()=>finishOperation(data,'load','USB::LOAD','old'),/状态已变化/);
  assert.equal(hasPendingOperation(data,'load','USB::LOAD'),true);
  finishOperation(data,'load','USB::LOAD','new');
  assert.equal(hasPendingOperation(data,'load','USB::LOAD'),false);
});

test('storage errors prevent starting an untracked operation',()=>{
  const data=storage();
  data.setItem=()=>{throw Error('storage unavailable');};
  assert.throws(()=>beginOperation(data,'source','USB::SOURCE','token'),/storage unavailable/);
  assert.throws(()=>beginOperation(data,'source','','token'),/设备地址无效/);
});

test('manual console and power controls share the same resource recovery marker',()=>{
  const data=storage();beginOperation(data,'source','USB::SOURCE','first');
  assert.equal(hasPendingOperation(data,'manual','usb::source'),true);
  assert.throws(()=>beginOperation(data,'manual','USB::SOURCE','second'),/尚未核对/);
  finishOperation(data,'source','USB::SOURCE','first');
  beginOperation(data,'manual','USB::SOURCE','second');finishOperation(data,'manual','USB::SOURCE','second');
});
test('legacy recovery markers remain blocking until the operator acknowledges them',()=>{
  const data=storage();data.setItem('wiparse.device-operation.v1:source:USB::SOURCE','old');
  assert.equal(hasPendingOperation(data,'manual','USB::SOURCE'),true);
  assert.throws(()=>beginOperation(data,'manual','USB::SOURCE','new'),/尚未核对/);
  acknowledgeOperation(data,'manual','USB::SOURCE');assert.equal(hasPendingOperation(data,'source','USB::SOURCE'),false);
});
