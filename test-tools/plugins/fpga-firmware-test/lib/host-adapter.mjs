import { fail } from './common.mjs';
export const REQUIRED_CAPABILITIES = ['resource_lease','serial_handoff_raw_cursor','timed_capture','safe_cleanup','storage_check'];
// Injected by the host. No guessed invoke payloads or fallback to shared terminal logs.
export function hostAdapter(ctx) {
  const a=ctx.fpgaAdapter;
  if(!a || a.version!=='wiparse.fpga-host/1') throw fail('HOST.ADAPTER_REQUIRED','wiparse.fpga-host/1');
  if(ctx.args?.execution_mode==='hardware' && typeof a.cancelAndConfirm!=='function') throw fail('HOST.CANCELLATION_REQUIRED','cancelAndConfirm must confirm hardware is idle after timeout/stop');
  for(const capability of REQUIRED_CAPABILITIES) if(!a.capabilities?.includes(capability)) throw fail('HOST.CAPABILITY_REQUIRED',capability);
  for(const method of ['devices','preflight','storage','acquire','release','sourceOutput','voltage','probeReset','collectLog','capture','waveformSource','cleanup']) if(typeof a[method]!=='function') throw fail('HOST.METHOD_REQUIRED',method);
  return a;
}
