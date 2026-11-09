from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
from pathlib import Path
import json,time
root=Path(__file__).resolve().parent
reply='''## 模拟分析结果\n\n这是 **本地模拟服务** 的固定回复，用于检查界面，不代表模型或实物判定。\n\n| 检查项 | 观察 | 建议 |\n|---|---|---|\n| VIN | 12.00 V（合成） | 对照设定限值 |\n| 状态 | PING → CHARGING | 检查时序 |\n| 告警 | timeout | 查看原始上下文 |\n\n### 排查步骤\n\n1. 保留原始数据和时间戳。\n2. 检查异常前后的状态切换。\n3. 使用实测波形复核。\n\n```text\nSIMULATION ONLY\nNo instrument command executed\n```\n'''
class Handler(BaseHTTPRequestHandler):
 def log_message(self,*args):pass
 def do_POST(self):
  size=int(self.headers.get('Content-Length','0'))
  if size>131072:self.send_error(413);return
  payload=json.loads(self.rfile.read(size))
  with (root/'mock-requests.jsonl').open('a',encoding='utf-8') as f:f.write(json.dumps({'time':time.time(),'path':self.path,'bytes':size,'messages':len(payload.get('messages',[]))})+'\n')
  data=json.dumps({'id':'local-ui-mock','object':'chat.completion','model':'LOCAL-MOCK-NOT-A-REAL-MODEL','choices':[{'index':0,'message':{'role':'assistant','content':reply},'finish_reason':'stop'}],'usage':{'prompt_tokens':10,'completion_tokens':100,'total_tokens':110}},ensure_ascii=False).encode()
  self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(data)));self.end_headers();self.wfile.write(data)
server=ThreadingHTTPServer(('127.0.0.1',0),Handler)
(root/'mock-ready.json').write_text(json.dumps({'host':'127.0.0.1','port':server.server_port,'kind':'local-fixed-ui-fixture'}),encoding='utf-8')
server.serve_forever()
