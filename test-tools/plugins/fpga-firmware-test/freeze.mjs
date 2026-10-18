import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';

// A source inventory, separate from the host's final EXE/package acceptance.
const repo=fileURLToPath(new URL('../../../',import.meta.url));
const plugin='test-tools/plugins/fpga-firmware-test';
const output=`${plugin}/PLUGIN_FREEZE.json`;
const pluginManifest=JSON.parse(fs.readFileSync(path.join(repo,plugin,'plugin.json')));
const evidenceName=`evidence/plugin-regression-${pluginManifest.version}-2026-10-18.json`;
const sha=bytes=>createHash('sha256').update(bytes).digest('hex');
function inventory(){
 const files=[];
 const visit=relative=>{
  for(const entry of fs.readdirSync(path.join(repo,relative),{withFileTypes:true})){
   const name=`${relative}/${entry.name}`;
   if(name===output)continue;
   if(entry.isSymbolicLink())throw Error(`Symlink excluded from freeze: ${name}`);
   if(entry.isDirectory())visit(name);
   else if(entry.isFile()){const bytes=fs.readFileSync(path.join(repo,name));files.push({path:name,bytes:bytes.length,sha256:sha(bytes)});}
  }
 };
 visit(plugin);visit('recipes/fpga-firmware-test');
 return files.sort((a,b)=>a.path.localeCompare(b.path,'en'));
}
const files=inventory();
if(process.argv.includes('--write')){
 const evidence=JSON.parse(fs.readFileSync(path.join(repo,plugin,evidenceName)));
 for(const source of [evidence.source_sha256,evidence.ui_contract_followup?.source_sha256??{}])for(const[file,hash]of Object.entries(source))if(sha(fs.readFileSync(path.join(repo,plugin,file)))!==hash)throw Error(`Evidence/source mismatch: ${file}`);
 const manifest={schema_version:'wiparse.fpga-plugin-source-freeze/1',date:'2026-10-18',plugin_id:'fpga-firmware-test',plugin_version:pluginManifest.version,minimum_wiparse:pluginManifest.engines.wiparse,status:'source_frozen_pending_host_acceptance',scope:'Plugin, original 20 migration sources and project migration evidence only; host EXE acceptance remains separate',required_widgets:['rule_builder','timeline','reference_selector'],evidence:`${plugin}/${evidenceName}`,regression:{passed:evidence.passed,failed:evidence.failed,fault_matrix:evidence.fault_matrix_cases,ui_followup_passed:evidence.ui_contract_followup?.passed??0},architecture_reliability_review:evidence.architecture_review??'pending',host_acceptance:'pending',hardware_accepted:false,longest_aging_accepted:false,files};
 fs.writeFileSync(path.join(repo,output),JSON.stringify(manifest,null,2)+'\n');
}else if(process.argv.includes('--verify')){
 const expected=JSON.parse(fs.readFileSync(path.join(repo,output)));
 if(JSON.stringify(expected.files)!==JSON.stringify(files))throw Error('Frozen source inventory changed');
}else throw Error('Use --write to freeze or --verify to verify');
const cases=files.filter(f=>/^recipes\/fpga-firmware-test\/cases\/[^/]+\.json$/.test(f.path));
if(cases.length!==20)throw Error('Expected 20 business configurations');
console.log(JSON.stringify({files:files.length,business_cases:cases.length,freeze:output,sha256:sha(fs.readFileSync(path.join(repo,output)))}));
