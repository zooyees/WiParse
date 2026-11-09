import {fail} from './common.mjs';

/** Preserve redacted fields from the exact, revision-checked stored document. */
export function restoreVisibleDocument(visible,stored,depth=0,root=stored) {
  if(depth>48)fail('CONFIG.INVALID_REQUEST','Document nesting limit exceeded');
  const origin=typeof visible==='string'?visible.match(/^\[(?:REDACTED|LOCAL_PATH)@([^\r\n]+)\]$/):null;
  if(origin){
    let path;try{path=JSON.parse(decodeURIComponent(origin[1]));}catch{fail('CONFIG.REDACTED_VALUE','Invalid preserved-value reference');}
    if(!Array.isArray(path)||path.length>48||path.some(key=>typeof key!=='string'||key.length>1024||['__proto__','constructor','prototype'].includes(key)))fail('CONFIG.REDACTED_VALUE','Invalid preserved-value reference');
    let original=root;
    for(const key of path){if(original===null||typeof original!=='object'||!Object.hasOwn(original,key))fail('CONFIG.REDACTED_VALUE','Preserved value no longer exists');original=original[key];}
    if(original===undefined)fail('CONFIG.REDACTED_VALUE','Preserved value no longer exists');
    return structuredClone(original);
  }
  if(visible==='[REDACTED]'||visible==='[LOCAL_PATH]') {
    if(stored===undefined||stored==='[REDACTED]'||stored==='[LOCAL_PATH]')fail('CONFIG.REDACTED_VALUE','Cannot create a field from a placeholder');
    return structuredClone(stored);
  }
  if(Array.isArray(visible))return visible.map((value,index)=>{
    const identity=value&&typeof value==='object'&&!Array.isArray(value)&&typeof value.id==='string'?value.id:null;
    const previous=identity&&Array.isArray(stored)?stored.find(item=>item?.id===identity):Array.isArray(stored)?stored[index]:undefined;
    return restoreVisibleDocument(value,previous,depth+1,root);
  });
  if(visible&&typeof visible==='object')return Object.fromEntries(Object.entries(visible).map(([key,value])=>{
    if(['__proto__','constructor','prototype'].includes(key))fail('CONFIG.INVALID_REQUEST','Reserved property');
    return [key,restoreVisibleDocument(value,stored&&Object.hasOwn(stored,key)?stored[key]:undefined,depth+1,root)];
  }));
  return visible;
}
