import {object} from './model.ts';
export type Schema=Record<string,unknown>;

export function resolveSchema(input:unknown,documents:Record<string,unknown>,root:unknown,depth=0):Schema {
  if(depth>16)return {};
  const schema=object(input);let result={...schema};
  if(typeof schema.$ref==='string'){
    const [key,fragment]=schema.$ref.split('#');let target=key?documents[key]:root;
    for(const segment of (fragment??'').split('/').slice(1))target=object(target)[segment.replaceAll('~1','/').replaceAll('~0','~')];
    result={...resolveSchema(target,documents,root,depth+1),...schema};delete result.$ref;
  }
  for(const branch of Array.isArray(result.allOf)?result.allOf:[]){const next=resolveSchema(branch,documents,root,depth+1);result={...result,...next,properties:{...object(result.properties),...object(next.properties)},required:[...new Set([...(Array.isArray(result.required)?result.required:[]),...(Array.isArray(next.required)?next.required:[])])]};}
  return result;
}

export function schemaDefault(input:unknown,documents:Record<string,unknown>,root:unknown,depth=0):unknown {
  if(depth>16)return null;
  const schema=resolveSchema(input,documents,root);
  if(schema.const!==undefined)return structuredClone(schema.const);
  if(schema.default!==undefined)return structuredClone(schema.default);
  if(Array.isArray(schema.enum))return schema.enum[0]??null;
  const branches=Array.isArray(schema.oneOf)?schema.oneOf:Array.isArray(schema.anyOf)?schema.anyOf:[];
  if(branches.length)return schemaDefault(branches[0],documents,root,depth+1);
  const type=Array.isArray(schema.type)?schema.type.find(value=>value!=='null'):schema.type;
  if(type==='object'||schema.properties)return Object.fromEntries(Object.entries(object(schema.properties)).filter(([key,value])=>(Array.isArray(schema.required)&&schema.required.includes(key))||object(value).default!==undefined||object(value).const!==undefined).map(([key,value])=>[key,schemaDefault(value,documents,root,depth+1)]));
  if(type==='null')return null;
  if(type==='array')return [];
  if(type==='boolean')return false;
  if(type==='number'||type==='integer')return typeof schema.minimum==='number'?Math.max(0,schema.minimum):0;
  return '';
}

export function variantIndex(branches:unknown[],value:unknown,documents:Record<string,unknown>,root:unknown):number {
  const current=object(value);
  const schemas=branches.map(branch=>resolveSchema(branch,documents,root));
  const index=schemas.findIndex(schema=>schema.const!==undefined?schema.const===value:Object.entries(object(schema.properties)).some(([key,field])=>object(field).const!==undefined&&object(field).const===current[key]));
  if(index>=0)return index;
  const actual=value===null?'null':Array.isArray(value)?'array':typeof value;
  const typed=schemas.findIndex(schema=>{const types=Array.isArray(schema.type)?schema.type:[schema.type];return types.includes(actual)||(actual==='number'&&Number.isInteger(value)&&types.includes('integer'));});
  return Math.max(0,typed);
}
