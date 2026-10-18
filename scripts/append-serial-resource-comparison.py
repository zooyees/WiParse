"""Write measured release values into the audit without changing product code."""
import json
from pathlib import Path

root=Path(__file__).resolve().parents[1]
old=json.loads((root/'evidence/serial-resource-2026-10-16/summary.json').read_text())
new=json.loads((root/'evidence/serial-resource-1.1.32-2026-10-16/summary.json').read_text())
baseline={r['name']:r for r in old['results']}
labels={'idle':'未监控','connected-idle':'连接但无数据','serial-500':'500 行每秒 前台','serial-2000':'2000 行每秒 前台','background-500':'500 行每秒 后台','serial-500-overflow':'500 行每秒 超过行数上限'}
lines=['## 串口 Release 资源对照实测','',
       '同机本地 TCP 合成输入，约 128 字节原始文本每行；普通场景短时采样约 8 秒，超限场景约 16 秒。CPU 为整个进程相对整机占比，工作集包含整个 GUI。基线时有本机打包工作，新版数据在编译结束后取得，数值用于观察量级与吞吐，不能据此承诺固定降幅或长期稳定性。','',
       '| 场景 | 1.1.31 CPU | 1.1.32 CPU | 新版工作集 MiB | 新版累计接收 / 发送 |',
       '| --- | --- | --- | --- | --- |']
for r in new['results']:
    b=baseline[r['name']]
    lines.append(f"| {labels[r['name']]} | {b['cpu_machine_percent']:.2f}% | {r['cpu_machine_percent']:.2f}% | {r['working_set_mib']:.1f} | {r['received_lines']} / {r['sent_lines']} |")
startup=next(r for r in new['results'] if r['name']=='idle')
if startup['api_latency_max_ms'] > 1000:
    lines += ['', f"首次未监控场景中，状态 API 最大延迟为 {startup['api_latency_max_ms']:.0f} ms，采样时私有提交明显增长；该组可能受冷启动或初始化影响，不能当作稳态响应或稳态 CPU。后续场景保留在原始数据中分别比较，未删除这一异常取样。"]
lines += ['', '累计数在各自取样时刻读取，小差值包含采样时序和在途数据；巨大差值表示积压，不能直接等同于丢失。高负载时新版处理了更多数据，CPU 对比必须同时考虑实际吞吐，不能要求在工作量增加时 CPU 数值也必然更低。TCP 输入不是物理 COM/WinUSB 满速验收；没有把短时工作集变化解释为内存泄漏结论。', '',
          '原始数据随包保存于 `docs/verification/serial-resource-1.1.31.json` 与 `serial-resource-1.1.32.json`，新版程序哈希一并记录。', '']
report=root/'docs/system-audit-1.1.32-2026-10-16.md'
text=report.read_text(encoding='utf-8')
start='<!-- serial-resource-comparison:start -->'
end='<!-- serial-resource-comparison:end -->'
block=start+'\n'+'\n'.join(lines)+'\n'+end+'\n\n'
if start in text:
    a=text.index(start); b=text.index(end,a)+len(end)
    text=text[:a]+block.rstrip()+text[b:]
else:
    text=text.replace('## 仍需现场验证的范围',block+'## 仍需现场验证的范围')
report.write_text(text,encoding='utf-8')
print(json.dumps({'ok':True,'scenarios':len(new['results']),'report':str(report)},ensure_ascii=False))
