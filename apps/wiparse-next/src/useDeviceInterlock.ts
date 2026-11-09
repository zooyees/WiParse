import {useEffect,useState} from 'react';
import {acknowledgeOperation,beginOperation,finishOperation,hasPendingOperation} from './deviceInterlock';

/** Survives component unmount and application restart, but only in this WebView profile. */
export function useDeviceInterlock(kind:string,resource:string){
  const [uncertain,setUncertain]=useState(()=>{
    try{return hasPendingOperation(window.localStorage,kind,resource);}catch{return true;}
  });
  const changed=()=>window.dispatchEvent(new Event('wiparse-device-operation'));
  useEffect(()=>{const update=()=>{try{setUncertain(hasPendingOperation(window.localStorage,kind,resource));}catch{setUncertain(true);}};window.addEventListener('wiparse-device-operation',update);return()=>window.removeEventListener('wiparse-device-operation',update);},[kind,resource]);
  const begin=():string=>{
    const token=crypto.randomUUID();
    beginOperation(window.localStorage,kind,resource,token);changed();
    return token;
  };
  const finish=(token:string):boolean=>{
    try{finishOperation(window.localStorage,kind,resource,token);changed();setUncertain(false);return true;}
    catch{setUncertain(true);return false;}
  };
  const acknowledge=():boolean=>{
    try{acknowledgeOperation(window.localStorage,kind,resource);changed();setUncertain(false);return true;}
    catch{setUncertain(true);return false;}
  };
  return {uncertain,setUncertain,begin,finish,acknowledge};
}
