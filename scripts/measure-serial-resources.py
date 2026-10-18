"""Isolated Windows release-GUI baseline. Synthetic localhost TCP, no hardware."""
import ctypes as C
from ctypes import wintypes as W
import hashlib, json, os, pathlib, socket, statistics, subprocess, threading, time, urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[1]
OUT = pathlib.Path(os.environ.get('WIPARSE_BENCH_OUT', str(ROOT / 'evidence/serial-resource-1.1.32-2026-10-16')))
EXE = pathlib.Path(os.environ.get('WIPARSE_BENCH_EXE', str(ROOT/'target/release/wiparse-gui.exe')))
OUT.mkdir(parents=True, exist_ok=True)
K = C.WinDLL('kernel32', use_last_error=True)
P = C.WinDLL('psapi', use_last_error=True)
class Mem(C.Structure):
    _fields_ = [('cb', W.DWORD), ('faults', W.DWORD)] + [(x, C.c_size_t) for x in
        ('peak_ws', 'ws', 'peak_paged', 'paged', 'peak_nonpaged', 'nonpaged', 'private', 'peak_private')]
class IO(C.Structure):
    _fields_ = [(x, C.c_ulonglong) for x in ('reads','writes','other','read_bytes','write_bytes','other_bytes')]
K.GetProcessTimes.argtypes = [W.HANDLE] + [C.POINTER(C.c_ulonglong)] * 4
K.GetProcessIoCounters.argtypes = [W.HANDLE, C.POINTER(IO)]
P.GetProcessMemoryInfo.argtypes = [W.HANDLE, C.POINTER(Mem), W.DWORD]
def sample(proc):
    times = [C.c_ulonglong() for _ in range(4)]
    assert K.GetProcessTimes(int(proc._handle), *[C.byref(x) for x in times])
    m = Mem(); m.cb = C.sizeof(m)
    assert P.GetProcessMemoryInfo(int(proc._handle), C.byref(m), m.cb)
    io = IO(); assert K.GetProcessIoCounters(int(proc._handle), C.byref(io))
    return dict(cpu=(times[2].value+times[3].value)/1e7, ws=m.ws/1048576,
                private=m.private/1048576, write_bytes=io.write_bytes)
def free_port():
    with socket.socket() as s:
        s.bind(('127.0.0.1',0)); return s.getsockname()[1]
def run(name, rate, page, monitor=True, seconds=8):
    folder = OUT/name; folder.mkdir(exist_ok=True)
    cfg = json.loads((ROOT/'config.default.json').read_text(encoding='utf-8-sig'))
    cfg['log_monitor']['save_dir'] = str(folder/'logs')
    cfg['apps']['test_tool']['data_root'] = str(folder)
    cfg['apps']['test_tool']['plugins_dir'] = str(ROOT/'test-tools/plugins')
    config = folder/'config.json'; config.write_text(json.dumps(cfg),encoding='utf-8')
    port = free_port(); base=f'http://127.0.0.1:{port}'
    env = dict(os.environ, WIPARSE_APP_ROOT=str(folder), WIPARSE_CONFIG=str(config),
               WCM_CONFIG=str(config), WIPARSE_API_BIND=f'127.0.0.1:{port}')
    stop=threading.Event(); sent=[0]; connections=[]; server=socket.socket(); server.bind(('127.0.0.1',0)); server.listen()
    server.settimeout(.2); uart_port=server.getsockname()[1]
    def producer():
        while not stop.is_set():
            try: conn,_=server.accept(); break
            except socket.timeout: continue
            except OSError: return
        else: return
        with conn:
            connections.append(conn); conn.setsockopt(socket.SOL_SOCKET,socket.SO_SNDBUF,2*1024*1024)
            conn.settimeout(None); started=time.perf_counter()
            while not stop.is_set():
                target=int((time.perf_counter()-started)*rate)
                n=min(100,max(0,target-sent[0]))
                if n:
                    data=b''.join((f'BENCH {sent[0]+i:09d} '.encode()+b'x'*110+b'\n') for i in range(n))
                    try: conn.sendall(data)
                    except (OSError, socket.timeout): break
                    sent[0]+=n
                else: stop.wait(.002)
    worker=threading.Thread(target=producer,daemon=True); worker.start()
    si=subprocess.STARTUPINFO(); si.dwFlags|=subprocess.STARTF_USESHOWWINDOW; si.wShowWindow=0
    stdout=(folder/'stdout.log').open('wb'); stderr=(folder/'stderr.log').open('wb')
    proc=subprocess.Popen([str(EXE)],cwd=ROOT,env=env,
                          startupinfo=si,stdout=stdout,stderr=stderr)
    def request(path, payload=None):
        data=None if payload is None else json.dumps(payload).encode()
        req=urllib.request.Request(base+path,data=data,headers={'Content-Type':'application/json'})
        with urllib.request.urlopen(req,timeout=12) as response: return json.load(response)
    def invoke(method,**params):
        reply=request('/v1/invoke',dict(method=method,params=params))
        assert reply['ok'],reply
        return reply['data']
    try:
        for _ in range(150):
            try: request('/v1/health'); break
            except Exception:
                if proc.poll() is not None: raise RuntimeError('GUI exited')
                time.sleep(.1)
        expected_version = json.loads((ROOT/'mcp/wiparse/package.json').read_text(encoding='utf-8'))['version']
        actual_version = request('/v1/health')['data']['version']
        assert actual_version == expected_version, (actual_version, expected_version)
        invoke('ui.show',tab=page)
        if monitor: invoke('serial.monitor.start',port=f'tcp://127.0.0.1:{uart_port}',baud=2000000)
        time.sleep(1)
        before=sample(proc); start=time.perf_counter(); samples=[]; latency=[]
        for _ in range(seconds):
            time.sleep(1); samples.append(sample(proc))
            t=time.perf_counter(); invoke('serial.monitor.status'); latency.append((time.perf_counter()-t)*1000)
        wall=time.perf_counter()-start; after=sample(proc)
        info=invoke('log.lines.get',limit=0)
        result=dict(name=name,software_version=actual_version,rate_lines_s=rate,page=page,monitor=monitor,wall_s=wall,
            cpu_one_core_percent=(after['cpu']-before['cpu'])/wall*100,
            cpu_machine_percent=(after['cpu']-before['cpu'])/wall*100/(os.cpu_count() or 1),
            logical_cpus=os.cpu_count(),working_set_mib=after['ws'],private_mib=after['private'],
            private_growth_mib=after['private']-before['private'],
            process_write_bytes=after['write_bytes']-before['write_bytes'],
            api_latency_median_ms=statistics.median(latency),api_latency_max_ms=max(latency),
            sent_lines=sent[0],received_lines=info['total'],display_window=info['window'],
            note='Short synthetic TCP run; API latency is not input-to-paint latency; CPU includes GUI rendering.')
        print(json.dumps(result),flush=True)
        (folder/'result.json').write_text(json.dumps(result,indent=2),encoding='utf-8')
        return result
    finally:
        stop.set(); server.close()
        for conn in connections:
            try: conn.shutdown(socket.SHUT_RDWR)
            except OSError: pass
        if proc.poll() is None:
            proc.terminate(); proc.wait(timeout=10)
        stdout.close(); stderr.close(); worker.join(timeout=1)

if __name__ == '__main__':
    results=[run('idle',0,'serial',False),run('connected-idle',0,'serial'),
             run('serial-500',500,'serial'),run('serial-2000',2000,'serial'),
             run('background-500',500,'calculator')]
    results.append(run('serial-500-overflow',500,'serial',seconds=16))
    version = json.loads((ROOT/'mcp/wiparse/package.json').read_text(encoding='utf-8'))['version']
    exe_hash=hashlib.sha256(EXE.read_bytes()).hexdigest().upper()
    (OUT/'summary.json').write_text(json.dumps(dict(version=version,exe_sha256=exe_hash,results=results),indent=2),encoding='utf-8')
