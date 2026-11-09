import {object} from './model.ts';
/** Mirror the existing trigger contract without rewriting document metadata. */
export function scopeRuleItems(raw:string):unknown[]|null {
  try{const value:unknown=JSON.parse(raw.trim()||'[]');return Array.isArray(value)?value:Array.isArray(object(value).items)?object(value).items as unknown[]:null;}catch{return null;}
}
