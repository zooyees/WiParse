import catalog from './localization.en.json' with {type:'json'};
export type Language='zh'|'en';
const english:Record<string,string>=catalog;
const listeners=new Set<()=>void>();
let active:Language='zh';
try{if(typeof window!=='undefined'&&window.localStorage.getItem('wiparse.ui.language')==='en')active='en';}catch{/* The interface works without persistent browser storage. */}
export const getLanguage=()=>active;
export function subscribeLanguage(listener:()=>void){listeners.add(listener);return()=>{listeners.delete(listener);};}
export function applyLanguage(value:string){
  const next:Language=/^en(?:[-_]|$)/i.test(value)?'en':'zh';
  if(typeof document!=='undefined')document.documentElement.lang=next==='zh'?'zh-CN':'en';
  try{if(typeof window!=='undefined')window.localStorage.setItem('wiparse.ui.language',next);}catch{}
  if(next===active)return;active=next;for(const listener of [...listeners])listener();
}
/** Interface labels only: never apply this to raw logs, model replies or data. */
export function localizeLabel(text:string,language:Language=active):string {
  if(language==='zh')return text;
  const key=text.replace(/\s+/g,' ').trim();return english[key]||text;
}
