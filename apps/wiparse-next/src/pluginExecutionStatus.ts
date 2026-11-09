/** A global host message is usable only inside its matching active workspace. */
export function pluginExecutionStatus(state:Record<string,unknown>,project:string,plugin:string,acceptedHere:boolean):string{
 if(state.project!==project||state.selected!==plugin)return '等待操作';
 if(state.running===true)return state.running_plugin===plugin?'任务运行中':'其他测试运行中';
 const status=typeof state.status==='string'?state.status:'';
 return acceptedHere&&status.endsWith(` · ${plugin}`)?status:'当前无运行任务';
}
