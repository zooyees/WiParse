"""Local-only synthetic UART stream, with bounded lifetime and explicit stop."""
import socket,time,json
from pathlib import Path
root=Path(__file__).parent
listener=socket.socket();listener.bind(('127.0.0.1',0));listener.listen(1);listener.settimeout(1)
port=listener.getsockname()[1]
(root/'serial-simulator.json').write_text(json.dumps({'port':port,'mode':'synthetic_loopback_only'}),encoding='utf-8')
print(f'Local synthetic UART listening on 127.0.0.1:{port}',flush=True)
started=time.monotonic();counter=0;clients=0
while time.monotonic()-started<3600 and not (root/'serial-simulator.stop').exists():
 try:client,_=listener.accept()
 except socket.timeout:continue
 clients+=1
 with client:
  client.settimeout(1)
  while time.monotonic()-started<3600 and not (root/'serial-simulator.stop').exists():
   burst=10
   payload=''.join(f'QA46 row={counter+i:08d} synthetic temperature=25.0 status=OK\r\n' for i in range(burst));counter+=burst
   try:client.sendall(payload.encode())
   except OSError:break
   time.sleep(.05)
listener.close()
(root/'serial-simulator-result.json').write_text(json.dumps({'lines_sent':counter,'connections':clients,'elapsed_seconds':time.monotonic()-started}),encoding='utf-8')
print('Synthetic UART closed',flush=True)
