type Job={owner:object;run:()=>Promise<string>;resolve:(value:string)=>void;reject:(reason:unknown)=>void};

/** Shared bounded native-image concurrency, with cancellation by report owner. */
export class ImageRequestQueue{
  private waiting:Job[]=[];
  private active=0;
  private limit:number;
  constructor(limit=3){if(!Number.isSafeInteger(limit)||limit<1)throw Error('图片并发限制无效');this.limit=limit;}
  enqueue(owner:object,run:()=>Promise<string>):Promise<string>{
    return new Promise((resolve,reject)=>{this.waiting.push({owner,run,resolve,reject});this.pump();});
  }
  cancel(owner:object){
    const removed=this.waiting.filter(job=>job.owner===owner);
    this.waiting=this.waiting.filter(job=>job.owner!==owner);
    for(const job of removed)job.reject(Error('图片加载已取消'));
  }
  private pump(){
    while(this.active<this.limit&&this.waiting.length){
      const job=this.waiting.shift()!;this.active++;
      void Promise.resolve().then(job.run).then(job.resolve,job.reject).finally(()=>{this.active--;this.pump();});
    }
  }
}

const sharedQueue=new ImageRequestQueue();
export class ImageLoader{
  private cache=new Map<string,Promise<string>>();
  private generation=0;
  private session:number;
  private read:(session:number,source:string)=>Promise<string>;
  private queue:ImageRequestQueue;
  constructor(session:number,read:(session:number,source:string)=>Promise<string>,queue=sharedQueue){this.session=session;this.read=read;this.queue=queue;}
  load(source:string):Promise<string>{
    const cached=this.cache.get(source);if(cached)return cached;
    if(this.cache.size>=32)return Promise.reject(Error('图片数量超过预览上限'));
    const generation=this.generation;
    const task=this.queue.enqueue(this,async()=>{
      if(generation!==this.generation)throw Error('图片加载已取消');
      const data=await this.read(this.session,source);
      if(!data.startsWith('data:image/'))throw Error('图片数据格式无效');
      return data;
    });
    this.cache.set(source,task);return task;
  }
  // Keep the owner reusable for React StrictMode effect cleanup/setup replay.
  cancelPending(){this.generation++;this.queue.cancel(this);this.cache.clear();}
}
