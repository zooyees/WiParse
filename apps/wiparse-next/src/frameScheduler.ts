/** One pending frame per plot owner; cleanup prevents stale canvas work. */
export function frameScheduler(draw:()=>void,request:(callback:()=>void)=>number,cancel:(id:number)=>void) {
  let pending:number|null=null,disposed=false;
  return {
    schedule(){if(disposed||pending!==null)return;pending=request(()=>{pending=null;if(!disposed)draw();});},
    dispose(){disposed=true;if(pending!==null)cancel(pending);pending=null;}
  };
}
