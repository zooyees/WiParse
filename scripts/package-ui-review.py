"""Build a GUI update from the verified FT4222 interim bundle."""
import hashlib
import json
import shutil
import zipfile
from datetime import datetime, timezone
from pathlib import Path

root = Path(__file__).resolve().parents[1]
base = root / 'dist/WiParse-1.2.2-Current-20261106'
out = root / 'dist/WiParse-1.2.2-Spacing-20261106'
archive = out.parent / (out.name + '.zip')
assert base.is_dir() and not out.exists() and not archive.exists()
shutil.copytree(base, out)
shutil.copy2(root / 'target/release/wiparse-gui.exe', out / 'bin/WiParse.exe')
shutil.copy2(root / 'target/release/wiparse_gui.pdb', out / 'symbols/wiparse_gui.pdb')
report = out / 'docs/reviews/wiparse-ui-ue-review-2026-11-06.md'
report.parent.mkdir(parents=True, exist_ok=True)
shutil.copy2(root / 'docs/reviews/wiparse-ui-ue-review-2026-11-06.md', report)
(out / '界面更新说明.md').write_text('本次收紧窗口外边距、工具面板之间的间距和串口页面卡片内边距。无线插件保持上一版本；实机功能仍需验收。\n', encoding='utf-8')
def digest(file):
    h = hashlib.sha256()
    with file.open('rb') as f:
        while data := f.read(1024 * 1024): h.update(data)
    return h.hexdigest()
manifest_path = out / 'release-manifest.json'
manifest = json.loads(manifest_path.read_text(encoding='utf-8'))
manifest.update(created_at=datetime.now(timezone.utc).isoformat(), description='收紧工具面板和窗口内容边距', gui_verified_sha256=digest(out / 'bin/WiParse.exe'), prior_gui_tests='GUI regression and serial/wave headless layout tests passed; real device review incomplete.')
files = sorted(p for p in out.rglob('*') if p.is_file() and p != manifest_path)
manifest['files'] = [{'path':p.relative_to(out).as_posix(),'size':p.stat().st_size,'sha256':digest(p)} for p in files]
manifest_path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2), encoding='utf-8')
with zipfile.ZipFile(archive, 'x', zipfile.ZIP_DEFLATED, compresslevel=3, allowZip64=True) as z:
    for p in [*files,manifest_path]: z.write(p,(Path(out.name)/p.relative_to(out)).as_posix())
with zipfile.ZipFile(archive) as z:
    assert z.testzip() is None
    for item in manifest['files']:
        h=hashlib.sha256()
        with z.open(out.name+'/'+item['path']) as f:
            while data:=f.read(1024*1024): h.update(data)
        assert h.hexdigest()==item['sha256'],item['path']
sha=digest(archive)
archive.with_suffix('.zip.sha256').write_text(f'{sha}  {archive.name}\n',encoding='ascii')
print(json.dumps({'archive':str(archive),'directory':str(out),'files':len(files),'sha256':sha},ensure_ascii=False))
