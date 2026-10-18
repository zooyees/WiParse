/** Migrate existing demonstration filenames without changing test content or IDs. */
import fs from 'node:fs';
import path from 'node:path';
const root=fs.realpathSync(path.resolve(process.argv[2]??'recipes/fpga-firmware-test'));
const cases=fs.realpathSync(path.join(root,'cases'));
if(path.relative(root,cases).startsWith('..')) throw Error('Repository path escape');
const changes=[];
for(const name of fs.readdirSync(cases).filter(n=>!/^[_\.]/.test(n)&&n.endsWith('.json'))) {
  const source=path.join(cases,name),value=JSON.parse(fs.readFileSync(source,'utf8'));
  if(!/^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/.test(value.id)) throw Error('Invalid configuration ID');
  if(name===value.id+'.json')continue;
  const target=path.join(cases,value.id+'.json');
  if(fs.existsSync(target))throw Error(`Conflict: ${target}`);
  changes.push({source,target,id:value.id});
}
for(const item of changes) {fs.renameSync(item.source,item.target);fs.appendFileSync(path.join(root,'.filename-migration.jsonl'),JSON.stringify({at:new Date().toISOString(),operation:'rename_demo_file',from:path.basename(item.source),to:path.basename(item.target),id:item.id})+'\n');}
console.log(JSON.stringify({ok:true,renamed:changes.length,repository:cases}));
