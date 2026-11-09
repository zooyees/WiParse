"""Package the validated wireless plugin against the previous portable runtime."""
import hashlib
import json
import shutil
import zipfile
from datetime import datetime, timezone
from pathlib import Path

root = Path(__file__).resolve().parents[1]
base = root / 'dist/WiParse-1.2.2-WirelessWhitebox-20261105'
out = root / 'dist/WiParse-1.2.2-WirelessWhitebox-FT4222-GUI-20261105'
archive = out.parent / (out.name + '.zip')
assert base.is_dir() and not out.exists() and not archive.exists()
shutil.copytree(base, out)
for source, target in [('wiparse-gui.exe','bin/WiParse.exe'),('wiparse.exe','bin/WiParse-CLI.exe'),('wiparse-service.exe','bin/WiParse-Service.exe'),('wiparse_gui.pdb','symbols/wiparse_gui.pdb'),('wiparse.pdb','symbols/wiparse.pdb'),('wiparse_service.pdb','symbols/wiparse_service.pdb')]:
    shutil.copy2(root / 'target/release' / source, out / target)
for rel in ['test-tools/plugins/wireless-whitebox', 'recipes/wireless-whitebox']:
    destination = out / rel
    if destination.exists():
        shutil.rmtree(destination)
    shutil.copytree(root / rel, destination)
for rel in ['test-tools/lib/host/fpga-host-adapter.mjs', 'test-tools/lib/hub/run-binding.mjs']:
    shutil.copy2(root / rel, out / rel)
notes = '''# 无线白盒测试阶段版（FT4222）\n\n本包含当前重新编译的 WiParse 程序、无线白盒插件和 Testing Hub 适配层。\n\n已实现：新 TX 日志确认 RX 启动后访问 FT4222；MT5788 身份及 RX 状态读取；按配置设置 RX 输出电压并检查电子负载电压；可选示波器证据；离线 TX 日志分析。设备和限值在软件内配置。\n\n尚未完成：完整 20 项自动测试、电源及电子负载自动控制、实物联调验收。结果不能作为完整产品合格结论。控制动作可能改变 RX 输出，使用前需确认连接和限值；异常恢复锁应在人工检查设备状态后处理。FT4222 需要系统安装相应驱动和 LibFT4222。\n'''
(out / '无线白盒阶段版说明.md').write_text(notes, encoding='utf-8')
manifest_path = out / 'release-manifest.json'
manifest = json.loads(manifest_path.read_text(encoding='utf-8'))
manifest.update(created_at=datetime.now(timezone.utc).isoformat(), description='无线白盒 FT4222 阶段版；未完成全部测试和实物验收', wireless_whitebox_hardware_ready=False)
def digest(file):
    h = hashlib.sha256()
    with file.open('rb') as f:
        while data := f.read(1024 * 1024):
            h.update(data)
    return h.hexdigest()
manifest['gui_verified_sha256'] = digest(out / 'bin/WiParse.exe')
manifest['prior_gui_tests'] = 'Current release build; GUI changes have not been visually revalidated for this interim package.'
files = sorted(p for p in out.rglob('*') if p.is_file() and p != manifest_path)
manifest['files'] = [{'path': p.relative_to(out).as_posix(), 'size': p.stat().st_size, 'sha256': digest(p)} for p in files]
manifest_path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2), encoding='utf-8')
with zipfile.ZipFile(archive, 'x', zipfile.ZIP_DEFLATED, compresslevel=3, allowZip64=True) as z:
    for p in [*files, manifest_path]:
        z.write(p, (Path(out.name) / p.relative_to(out)).as_posix())
with zipfile.ZipFile(archive) as z:
    assert z.testzip() is None
    for item in manifest['files']:
        h = hashlib.sha256()
        with z.open(out.name + '/' + item['path']) as f:
            while data := f.read(1024 * 1024): h.update(data)
        assert h.hexdigest() == item['sha256'], item['path']
checksum = digest(archive)
archive.with_suffix('.zip.sha256').write_text(f'{checksum}  {archive.name}\n', encoding='ascii')
print(json.dumps({'archive':str(archive),'directory':str(out),'files':len(files),'sha256':checksum,'bytes':archive.stat().st_size},ensure_ascii=False))
