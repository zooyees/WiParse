"""Package the current runtime and verified wireless plugin into a fresh bundle."""
import hashlib
import json
import shutil
import zipfile
from datetime import datetime, timezone
from pathlib import Path

root = Path(__file__).resolve().parents[1]
base = root / 'dist/WiParse-1.2.2-UI-Redesign-20261106'
out = root / 'dist/WiParse-1.2.2-Current-20261106'
archive = out.parent / (out.name + '.zip')
assert base.is_dir() and not out.exists() and not archive.exists()
shutil.copytree(base, out)
for source, target in [('wiparse-gui.exe','bin/WiParse.exe'),('wiparse.exe','bin/WiParse-CLI.exe'),('wiparse-service.exe','bin/WiParse-Service.exe'),('wiparse_gui.pdb','symbols/wiparse_gui.pdb'),('wiparse.pdb','symbols/wiparse.pdb'),('wiparse_service.pdb','symbols/wiparse_service.pdb')]:
    shutil.copy2(root / 'target/release' / source, out / target)
for rel in ['test-tools/plugins/wireless-whitebox', 'recipes/wireless-whitebox']:
    dest = out / rel
    if dest.exists(): shutil.rmtree(dest)
    shutil.copytree(root / rel, dest)
for rel in ['test-tools/lib/host/fpga-host-adapter.mjs','test-tools/lib/hub/run-binding.mjs']:
    shutil.copy2(root / rel, out / rel)
note = '''# 当前版本更新说明\n\n包含当前源码重新编译的 GUI、CLI、Service 及无线白盒插件。无线插件的 MT5788/FT4222 访问和恒流负载模式仅经模拟联调，尚未完成实物验收；完整 20 项无线自动测试尚未完成。\n\n覆盖更新时保留已有 config.json、工程、日志和用户配方；更新前备份原文件。\n'''
(out / '当前版本说明.md').write_text(note, encoding='utf-8')
def digest(p):
    h=hashlib.sha256()
    with p.open('rb') as f:
        while data:=f.read(1024*1024): h.update(data)
    return h.hexdigest()
manifest_path=out/'release-manifest.json'
manifest=json.loads(manifest_path.read_text(encoding='utf-8'))
manifest.update(created_at=datetime.now(timezone.utc).isoformat(), description='当前源码构建；无线恒流模式为模拟验证阶段', gui_verified_sha256=digest(out/'bin/WiParse.exe'), wireless_whitebox_hardware_ready=False)
plugin_manifests=sorted((json.loads(p.read_text(encoding='utf-8-sig')) for p in (out/'test-tools/plugins').glob('*/plugin.json')),key=lambda p:p['id'])
manifest['plugins']=[{'id':p['id'],'version':p['version']} for p in plugin_manifests]
files=sorted(p for p in out.rglob('*') if p.is_file() and p!=manifest_path)
manifest['files']=[{'path':p.relative_to(out).as_posix(),'size':p.stat().st_size,'sha256':digest(p)} for p in files]
manifest_path.write_text(json.dumps(manifest,ensure_ascii=False,indent=2),encoding='utf-8')
with zipfile.ZipFile(archive,'x',zipfile.ZIP_DEFLATED,compresslevel=3,allowZip64=True) as z:
    for p in [*files,manifest_path]: z.write(p,(Path(out.name)/p.relative_to(out)).as_posix())
with zipfile.ZipFile(archive) as z:
    assert z.testzip() is None
    for item in manifest['files']:
        h=hashlib.sha256()
        with z.open(out.name+'/'+item['path']) as f:
            while data:=f.read(1024*1024): h.update(data)
        assert h.hexdigest()==item['sha256'],item['path']
for rel in ['test-tools/plugins/wireless-whitebox', 'recipes/wireless-whitebox']:
    for source in (root / rel).rglob('*'):
        if source.is_file():
            packaged=out / source.relative_to(root)
            assert packaged.is_file() and digest(source)==digest(packaged), f'Source changed during packaging: {source}'
for rel in ['test-tools/lib/host/fpga-host-adapter.mjs','test-tools/lib/hub/run-binding.mjs']:
    assert digest(root/rel)==digest(out/rel), f'Source changed during packaging: {rel}'
sha=digest(archive)
archive.with_suffix('.zip.sha256').write_text(f'{sha}  {archive.name}\n',encoding='ascii')
print(json.dumps({'archive':str(archive),'directory':str(out),'files':len(files),'sha256':sha},ensure_ascii=False))
