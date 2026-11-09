/** The highlighted workspace must reflect the host's actual active plugin. */
export function selectedPluginFromHost(state:unknown,installed:ReadonlyArray<{id:string}>):string {
  if(!state||typeof state!=='object'||Array.isArray(state))return '';
  const tool=(state as Record<string,unknown>).test_tool;
  if(!tool||typeof tool!=='object'||Array.isArray(tool))return '';
  const selected=(tool as Record<string,unknown>).selected;
  return typeof selected==='string'&&installed.some(item=>item.id===selected)?selected:'';
}
