import {memo,useSyncExternalStore} from 'react';
import {getLanguage,subscribeLanguage,localizeLabel} from './language';
export {localizeLabel} from './language';
export const LocalizedLabel=memo(function LocalizedLabel({text}:{text:string}) {
  const language=useSyncExternalStore(subscribeLanguage,getLanguage,getLanguage);
  const label=localizeLabel(text,language);
  return language==='en'?` ${label} `:label;
});
export function useLanguage(){return useSyncExternalStore(subscribeLanguage,getLanguage,getLanguage);}
