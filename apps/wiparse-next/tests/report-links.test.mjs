import test from 'node:test';
import assert from 'node:assert/strict';
import {evidenceTarget,headingAnchor,reportHeadingIds} from '../src/reportLinks.ts';

test('report links route portable evidence and legacy local paths to the correct viewer',()=>{
  assert.equal(evidenceTarget('../波形/通道%201.isf#CH1'),'wave');
  assert.equal(evidenceTarget('./截图/a%23b.png'),'image');
  assert.equal(evidenceTarget('C:\\工程\\报告\\结果.md'),'document');
  assert.equal(evidenceTarget('#测试结果'),'anchor');
  assert.equal(evidenceTarget('evidence.pdf'),'external');
  assert.equal(headingAnchor('2. 测试结果 Summary'),'2-测试结果-summary');
});
test('report links never treat executable schemes or shares as evidence',()=>{
  for(const source of ['javascript:alert(1)','%6aavascript:alert(1)','file:///C:/secret.txt','//host/share.md','\\\\host\\share.md','run.exe','a%ZZ.md','x\n.md']) {
    assert.equal(evidenceTarget(source),'blocked',source);
  }
});

test('all report headings have stable unique anchors, including formatted titles',()=>{
  const heading=children=>({type:'heading',children});
  const tree={type:'root',children:[heading([{type:'text',value:'结果'}]),heading([{type:'strong',children:[{type:'text',value:'结果'}]}]),heading([{type:'text',value:'结果-1'}]),heading([{type:'text',value:'结果'}]),heading([{type:'text',value:'!!!'}])]};
  reportHeadingIds()(tree);
  assert.deepEqual(tree.children.map(node=>node.data.hProperties.id),['结果','结果-1','结果-1-1','结果-2','section']);
});
