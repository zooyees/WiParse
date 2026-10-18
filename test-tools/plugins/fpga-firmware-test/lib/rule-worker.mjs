import {parentPort,workerData} from 'node:worker_threads';
import {evaluate} from './rules.mjs';
try {parentPort.postMessage({ok:true,value:evaluate(workerData.rules,workerData.evidence)});}catch(error){parentPort.postMessage({ok:false,error:error.message});}
