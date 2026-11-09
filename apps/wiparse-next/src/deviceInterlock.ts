/** Local recovery marker, not a cross-process device lock. */
export type MarkerStorage = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;

function key(_kind: string, resource: string): string {
  if (!resource.trim()) throw Error('设备地址无效');
  return `wiparse.device-operation.v1:instrument:${resource.trim().toUpperCase()}`;
}

function legacyKeys(resource:string):string[]{
  return ['source','load','manual'].map(kind=>`wiparse.device-operation.v1:${kind}:${resource.trim().toUpperCase()}`);
}

export function hasPendingOperation(storage: MarkerStorage, kind: string, resource: string): boolean {
  return [key(kind,resource),...legacyKeys(resource)].some(name=>storage.getItem(name)!==null);
}

export function beginOperation(storage: MarkerStorage, kind: string, resource: string, token: string): void {
  const name=key(kind, resource);
  if (!token || hasPendingOperation(storage,kind,resource)) throw Error('设备上一次操作尚未核对');
  storage.setItem(name,token);
  if (storage.getItem(name)!==token) throw Error('无法保存设备操作状态');
}

/** Do not clear a newer marker in response to an older operation. */
export function finishOperation(storage: MarkerStorage, kind: string, resource: string, token: string): void {
  const name=key(kind, resource);
  if (storage.getItem(name)!==token) throw Error('设备操作状态已变化，请核对设备');
  storage.removeItem(name);
  if (storage.getItem(name)!==null) throw Error('无法清除设备操作状态');
}

export function acknowledgeOperation(storage: MarkerStorage, kind: string, resource: string): void {
  for(const name of [key(kind,resource),...legacyKeys(resource)])storage.removeItem(name);
  if (hasPendingOperation(storage,kind,resource)) throw Error('无法清除设备操作状态');
}
