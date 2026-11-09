"""Archive the frozen R45 preview, preserving its verified files."""
from pathlib import Path
from datetime import datetime, timezone
import hashlib,json,zipfile
root=Path(__file__).resolve().parents[3]
source=root/'dist/WiParse-Next-20261109-Preview-R45'
target=root/'dist/WiParse-Next-20261109-R45-Full.zip'
temporary=target.with_suffix('.zip.partial')
assert source.is_dir() and not target.exists() and not temporary.exists()
def digest(stream):
 h=hashlib.sha256()
 while block:=stream.read(1024*1024):h.update(block)
 return h.hexdigest()
def filehash(path):
 with path.open('rb') as stream:return digest(stream)
inventory_path=source/'files-sha256.json'
inventory=json.loads(inventory_path.read_text(encoding='utf-8-sig'))
expected={**inventory,'files-sha256.json':filehash(inventory_path)}
files=sorted(p for p in source.rglob('*') if p.is_file())
assert {p.relative_to(source).as_posix() for p in files}==set(expected)
for p in files:
 assert not p.is_symlink()
 assert filehash(p)==expected[p.relative_to(source).as_posix()],str(p)
print(f'Frozen delivery verified: {len(files)} files',flush=True)
prefix=source.name+'/'
readme="""WiParse Next R45 完整更新包（2026-11-09）

解压到独立目录，运行 WiParse-Next-20261109-Preview-R45/bin/WiParse.exe。
请保留全部相邻文件；包含 GUI、Host、CLI、Service、MCP、插件、依赖和匹配调试符号。
本压缩包封装已验证的 R45 预览目录；其中 PREVIEW.txt / RELEASE-NOTES.txt 的“不生成 ZIP”描述属于此前 EXE 预览交付阶段，本次已按用户要求生成完整 ZIP。

Windows 需要 WebView2；MCP/插件需要系统安装 Node.js 24；连接真实仪器需要相应 VISA/设备驱动。本包未附这些系统安装程序。
不包含用户密钥、工程、采集日志；AI 默认关闭。请先备份旧目录，并保留自己的配置、工程与日志，建议先在独立目录测试。
107 项前端测试及前端/发布构建通过；双通道桌面证据和原生测试继承边界详见 docs/verification 和 docs/reviews。真实仪器、AI、多 DPI、长时负载未在本轮验收。
"""
with zipfile.ZipFile(temporary,'x',zipfile.ZIP_DEFLATED,compresslevel=6,allowZip64=True) as bundle:
 bundle.writestr('安装与更新说明.txt',readme.encode('utf-8'))
 for p in files:bundle.write(p,prefix+p.relative_to(source).as_posix())
print('Compression complete; verifying archived hashes and CRC',flush=True)
with zipfile.ZipFile(temporary) as bundle:
 names=bundle.namelist()
 assert len(names)==len(set(names))==len(files)+1
 assert set(names)=={prefix+k for k in expected}|{'安装与更新说明.txt'}
 assert bundle.testzip() is None
 for rel,sha in expected.items():
  with bundle.open(prefix+rel) as stream:assert digest(stream)==sha,rel
 assert bundle.read('安装与更新说明.txt').decode('utf-8')==readme
for p in files:assert filehash(p)==expected[p.relative_to(source).as_posix()],f'Delivery changed: {p}'
temporary.rename(target)
sha=filehash(target)
target.with_suffix('.zip.sha256').write_text(f'{sha}  {target.name}\n',encoding='ascii')
result={'passed':True,'archive':str(target),'bytes':target.stat().st_size,'sha256':sha,'runtime_files':len(files),'archive_entries':len(names),'all_archived_hashes_and_crc_verified':True,'source_preview_unchanged':True,'created_at':datetime.now(timezone.utc).isoformat()}
(Path(__file__).parent/'zip-delivery.json').write_text(json.dumps(result,ensure_ascii=False,indent=2),encoding='utf-8')
print(json.dumps(result,ensure_ascii=False),flush=True)
