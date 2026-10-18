import fs from 'node:fs';
import path from 'node:path';
import Ajv2020 from 'ajv/dist/2020.js';
import addFormats from 'ajv-formats';
import {jail, readJson, fail, sha, canonical} from './common.mjs';

const keywords = new Set('$schema $id $ref $defs $comment title description type properties required default enum const oneOf anyOf allOf not if then else dependentRequired additionalProperties items minItems maxItems uniqueItems minLength maxLength pattern format minimum maximum exclusiveMinimum exclusiveMaximum multipleOf readOnly writeOnly examples deprecated'.split(' '));
export function loadSchema(file, pluginRoot) {
  const documents = {}, loading = new Map(), originals = [];
  function visit(file) {
    file = jail(file,[pluginRoot]);
    if (loading.has(file)) return loading.get(file);
    if (loading.size >= 64) fail('SCHEMA.UNSUPPORTED_KEYWORD','Too many referenced schemas');
    const key = `urn:wiparse:schema:${sha(path.relative(pluginRoot,file)).slice(0,32)}`;
    loading.set(file,key);
    const schema = readJson(file); originals.push([path.relative(pluginRoot,file),schema]);
    function walk(node, depth = 0) {
      if (depth > 48) fail('SCHEMA.UNSUPPORTED_KEYWORD','Schema nesting limit exceeded');
      if (typeof node === 'boolean') return node;
      if (!node || Array.isArray(node) || typeof node !== 'object') fail('PLUGIN.UI_SCHEMA_INVALID','Schema must be an object or boolean');
      const out = {};
      for (const [k,v] of Object.entries(node)) {
        if (!keywords.has(k) && !k.startsWith('x-')) fail('SCHEMA.UNSUPPORTED_KEYWORD',k);
        if (k === '$id') continue;
        if (k === '$ref') {
          if (typeof v !== 'string' || /^[a-z]+:/i.test(v) || v.startsWith('/') || v.startsWith('\\')) fail('CONFIG.PATH_OUTSIDE_ROOT','Only local schema references are supported');
          const [relative,fragment] = v.split('#');
          const target = relative ? visit(path.resolve(path.dirname(file),relative)) : key;
          out[k] = target + (fragment !== undefined ? '#' + fragment : '');
        } else if (['properties','$defs'].includes(k)) out[k] = Object.fromEntries(Object.entries(v).map(([name,s]) => [name,walk(s,depth+1)]));
        else if (['oneOf','anyOf','allOf'].includes(k)) out[k] = v.map(s => walk(s,depth+1));
        else if (['if','then','else','not','items'].includes(k) || (k === 'additionalProperties' && typeof v === 'object')) out[k] = walk(v,depth+1);
        else out[k] = v;
      }
      return out;
    }
    const clean = walk(schema); if (typeof clean === 'object') clean.$id = key;
    documents[key] = clean; return key;
  }
  const rootId = visit(file);
  if(Buffer.byteLength(JSON.stringify(documents))>4*1048576)fail('PLUGIN.UI_SCHEMA_INVALID','Schema resource budget exceeded');
  const ajv = new Ajv2020({strictSchema:true, strictTypes:false, strictRequired:false, allErrors:true, validateFormats:true, ownProperties:true, unicodeRegExp:true});
  addFormats(ajv);
  const custom=new Set();for(const doc of Object.values(documents)){const visit=value=>{if(value&&typeof value==='object')for(const[k,v]of Object.entries(value)){if(k.startsWith('x-'))custom.add(k);visit(v);}};visit(doc);}for(const keyword of custom)ajv.addKeyword({keyword,valid:true});
  for (const format of ['path','file-path','directory','serial-port','instrument']) ajv.addFormat(format, {type:'string', validate: () => true});
  try { for (const [key,doc] of Object.entries(documents)) ajv.addSchema(doc,key); }
  catch { fail('PLUGIN.UI_SCHEMA_INVALID','Cannot compile schema'); }
  let validate; try { validate = ajv.getSchema(rootId); } catch { fail('PLUGIN.UI_SCHEMA_INVALID','Invalid or unresolved local schema'); }
  return {schema:documents[rootId], documents, fingerprint:sha(canonical(originals)),
    errors(value) { validate(value); return (validate.errors || []).slice(0,100).map(e => ({path:e.instancePath || '/', keyword:e.keyword, message:e.message, params:e.params})); }};
}
export function loadUi(file, pluginRoot) {
  if (!file) return {};
  const ui = readJson(jail(file,[pluginRoot]));
  const inspect = (value, key = '', depth = 0) => {
    if (depth > 32) fail('PLUGIN.UI_SCHEMA_INVALID','UI schema nesting limit');
    if (/^(script|html|javascript|expression|onClick|onChange)$/i.test(key)) fail('PLUGIN.UI_SCHEMA_INVALID','Executable UI schema content is not permitted');
    if (typeof value === 'string' && /^(https?:|javascript:|data:)/i.test(value)) fail('PLUGIN.UI_SCHEMA_INVALID','Remote UI resources are not permitted');
    if (value && typeof value === 'object') for (const [k,v] of Object.entries(value)) inspect(v,k,depth+1);
  }; inspect(ui);
  const widgets = new Set('file path file_picker path_picker tags tag_input discriminated_union timeline rule_builder reference_selector serial_port serial_selector serial_port_selector instrument_selector read_only_summary json_read_only'.split(' '));
  if (ui.required_widgets !== undefined && (!Array.isArray(ui.required_widgets) || ui.required_widgets.some(w => typeof w !== 'string' || !widgets.has(w))))
    fail('PLUGIN.UI_WIDGET_UNSUPPORTED','Unsupported required UI widget');
  if (ui.fields !== undefined && (!ui.fields || Array.isArray(ui.fields) || typeof ui.fields !== 'object')) fail('PLUGIN.UI_SCHEMA_INVALID','fields must be a path map');
  for (const [key,field] of Object.entries(ui.fields || {})) {
    if (!field || Array.isArray(field) || typeof field !== 'object') fail('PLUGIN.UI_SCHEMA_INVALID',`Invalid UI field: ${key}`);
    if (field.width !== undefined && !['short','normal','long'].includes(field.width)) fail('PLUGIN.UI_SCHEMA_INVALID',`Invalid field width: ${key}`);
    if (field.order !== undefined && !Number.isFinite(field.order)) fail('PLUGIN.UI_SCHEMA_INVALID',`Invalid field order: ${key}`);
    for (const flag of ['advanced','hidden']) if (field[flag] !== undefined && typeof field[flag] !== 'boolean') fail('PLUGIN.UI_SCHEMA_INVALID',`Invalid field flag: ${key}`);
  }
  return ui;
}
export function defaults(schema, documents, depth=0) {
  if (!schema || depth > 20) return null;
  if (schema.default !== undefined) return structuredClone(schema.default);
  if (schema.const !== undefined) return structuredClone(schema.const);
  if (schema.$ref) {
    const [key,fragment] = schema.$ref.split('#'); let target=documents[key];
    if (fragment) for (const p of fragment.split('/').slice(1)) target=target?.[p.replaceAll('~1','/').replaceAll('~0','~')];
    return defaults(target,documents,depth+1);
  }
  if (schema.enum?.length) return structuredClone(schema.enum[0]);
  if(schema.oneOf?.length||schema.anyOf?.length) return defaults((schema.oneOf??schema.anyOf)[0],documents,depth+1);
  const type=Array.isArray(schema.type) ? schema.type.find(t=>t!=='null') : schema.type;
  if (type === 'object' || schema.properties) return Object.fromEntries(Object.entries(schema.properties || {}).filter(([name,s]) => (schema.required || []).includes(name) || s.default !== undefined || s.const !== undefined).map(([name,s])=>[name,defaults(s,documents,depth+1)]));
  if (type === 'array') return Array.from({length:Math.min(schema.minItems || 0,500)},()=>defaults(schema.items,documents,depth+1));
  if (type === 'boolean') return false;
  if (type === 'number' || type === 'integer') return schema.minimum ?? (schema.exclusiveMinimum != null ? schema.exclusiveMinimum+1 : 0);
  return '';
}
export function redactSchema(value,schema,documents,depth=0){
  if(depth>32)return '[REDACTED]';
  if(schema?.writeOnly===true)return '[REDACTED]';
  if(schema?.$ref){const[key,fragment]=schema.$ref.split('#');let target=documents[key];if(fragment)for(const p of fragment.split('/').slice(1))target=target?.[p.replaceAll('~1','/').replaceAll('~0','~')];return redactSchema(value,target,documents,depth+1);}
  if(Array.isArray(value))return value.map(v=>redactSchema(v,schema?.items,documents,depth+1));
  if(value&&typeof value==='object')return Object.fromEntries(Object.entries(value).map(([k,v])=>[k,redactSchema(v,schema?.properties?.[k],documents,depth+1)]));
  return value;
}
