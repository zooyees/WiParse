// Development migration tool: change only explicit UI text, never business data.
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {parse} from '@babel/parser';
const app=fileURLToPath(new URL('../',import.meta.url));
const repo=fileURLToPath(new URL('../../../',import.meta.url));
const source=path.join(app,'src'),catalogPath=path.join(source,'localization.en.json');
const catalog=JSON.parse(fs.readFileSync(catalogPath,'utf8'));
const core=fs.readFileSync(path.join(repo,'crates/wiparse-core/src/i18n.rs'),'utf8');
const pairs=new Map();
for(const match of core.matchAll(/\(Lang::(Zh|En),\s*"([^"]+)"\)\s*=>\s*("(?:[^"\\]|\\.)*")/g)){
  const value=JSON.parse(match[3].replace(/\\u\{([0-9a-f]+)\}/gi,(_all,hex)=>JSON.stringify(String.fromCodePoint(parseInt(hex,16))).slice(1,-1)));
  const pair=pairs.get(match[2])??{};pair[match[1]]=value;pairs.set(match[2],pair);
}
for(const pair of pairs.values())if(pair.Zh&&pair.En)catalog[pair.Zh.replace(/\s+/g,' ').trim()] ||= pair.En;
const chinese=text=>/[\u3400-\u9fff]/.test(text);
const labels=new Map();
const attributes=new Set(['title','placeholder','alt','aria-label']);
let modified=0;
for(const name of fs.readdirSync(source).filter(name=>name.endsWith('.tsx')&&name!=='localization.tsx')){
  const filename=path.join(source,name),text=fs.readFileSync(filename,'utf8');
  // Parsing is required to distinguish UI labels from protocol tokens, enum
  // values, filenames, raw log content and event-handler business parameters.
  const tree=parse(text,{sourceType:'module',plugins:['typescript','jsx']});
  const edits=new Map();let component=false,helper=false;
  const remember=value=>{const key=value.replace(/\s+/g,' ').trim();if(!chinese(key))return;const files=labels.get(key)??new Set();files.add(name);labels.set(key,files);catalog[key] ||= '';};
  const expression=node=>{
    if(node?.type==='StringLiteral'&&chinese(node.value)){remember(node.value);helper=true;edits.set(node.start,{start:node.start,end:node.end,value:`localizeLabel(${JSON.stringify(node.value)})`});}
    else if(node?.type==='ConditionalExpression'){expression(node.consequent);expression(node.alternate);}
    else if(node?.type==='LogicalExpression')expression(node.right);
  };
  const visit=(node,parent)=>{
    if(!node||typeof node!=='object')return;
    if(node.type==='JSXText'&&chinese(node.value)){
      const label=node.value.replace(/\s+/g,' ').trim();remember(label);component=true;
      edits.set(node.start,{start:node.start,end:node.end,value:`<LocalizedLabel text={${JSON.stringify(label)}}/>`});
    }
    if(node.type==='JSXAttribute'&&attributes.has(node.name?.name)&&node.value?.type==='StringLiteral'&&chinese(node.value.value)){
      remember(node.value.value);helper=true;edits.set(node.value.start,{start:node.value.start,end:node.value.end,value:`{localizeLabel(${JSON.stringify(node.value.value)})}`});
    }
    if(node.type==='JSXExpressionContainer'&&(parent?.type!=='JSXAttribute'||attributes.has(parent.name?.name)))expression(node.expression);
    if(node.type==='ObjectProperty'&&['label','title','help'].includes(node.key?.name)&&node.value?.type==='StringLiteral')remember(node.value.value);
    if(node.type==='CallExpression'&&['number','text','choice','flag','optionalFlag','numeric'].includes(node.callee?.name)&&node.arguments[1]?.type==='StringLiteral')remember(node.arguments[1].value);
    if(node.type==='CallExpression'&&(node.callee?.object?.name==='window'&&node.callee?.property?.name==='confirm'||['setError','setNotice','setMessage','setFeedback','setFileError','setSearchStatus'].includes(node.callee?.name)))expression(node.arguments[0]);
    if(node.type==='CallExpression'&&node.callee?.name==='localizeLabel'&&node.arguments[0]?.type==='StringLiteral')remember(node.arguments[0].value);
    if(node.type==='JSXAttribute'&&node.name?.name==='text'&&parent?.name?.name==='LocalizedLabel'&&node.value?.type==='JSXExpressionContainer'&&node.value.expression?.type==='StringLiteral')remember(node.value.expression.value);
    for(const [key,value] of Object.entries(node))if(!['loc','start','end','extra','comments','tokens'].includes(key)){
      if(Array.isArray(value)){for(const child of value)if(child?.type)visit(child,node);}
      else if(value?.type)visit(value,node);
    }
  };
  visit(tree,null);
  if(!edits.size)continue;
  let result=text;for(const edit of [...edits.values()].sort((left,right)=>right.start-left.start))result=result.slice(0,edit.start)+edit.value+result.slice(edit.end);
  const imported=[component?'LocalizedLabel':null,helper?'localizeLabel':null].filter(Boolean);
  const existing=text.match(/import\s*\{([^}]+)\}\s*from ['"]\.\/localization['"];?/);
  if(existing){const names=[...new Set([...existing[1].split(',').map(value=>value.trim()).filter(Boolean),...imported])];result=result.replace(existing[0],`import {${names.join(',')}} from './localization';`);}
  else result=`import {${imported.join(',')}} from './localization';\n`+result;
  fs.writeFileSync(filename,result,'utf8');modified++;
}
const sorted=Object.fromEntries(Object.entries(catalog).sort(([left],[right])=>left.localeCompare(right,'zh')));
fs.writeFileSync(catalogPath,JSON.stringify(sorted,null,2)+'\n','utf8');
const missing=[...labels].filter(([key])=>!catalog[key]).map(([text,files])=>({text,files:[...files]}));
fs.writeFileSync(path.join(repo,'docs/plans/ui-language-inventory-2026-11-08.json'),JSON.stringify({modified_files:modified,labels:labels.size,missing},null,2)+'\n','utf8');
console.log(JSON.stringify({modified_files:modified,labels:labels.size,missing:missing.length}));
