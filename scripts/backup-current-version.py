"""Preserve current source, local settings, evidence and one runnable preview without a ZIP."""
import hashlib
import json
import shutil
import subprocess
from datetime import datetime, timezone
from pathlib import Path

root = Path(__file__).resolve().parents[1]
version = '1.2.2'
backup = root / 'backups' / f'WiParse-{version}-{datetime.now():%Y%m%d-%H%M%S}'
backup.mkdir(parents=True, exist_ok=False)
files = set()
for name in ['Cargo.toml','Cargo.lock','README.md','.gitignore','config.default.json','config.json','config.json.bak','WiParse.exe','WiParse-CLI.exe','WiParse-Service.exe']:
    if (root / name).is_file(): files.add(Path(name))
skip = {'node_modules','__pycache__','.git','target'}
for folder in ['crates','docs','scripts','test-tools','mcp','services','recipes','projects','evidence/release-1.2.2-2026-10-18','dist/wiparse-1.2.2-preview']:
    base = root / folder
    if base.exists():
        for p in base.rglob('*'):
            if p.is_file() and not any(part in skip for part in p.relative_to(base).parts):
                if folder.startswith('dist/') and p.relative_to(base).parts[0] == 'tmp': continue
                files.add(p.relative_to(root))
for p in (root / 'dist').glob('*'):
    if p.is_file(): files.add(p.relative_to(root))
entries=[]
for relative in sorted(files):
    source=root/relative; target=backup/'snapshot'/relative
    target.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(source,target)
    actual=hashlib.sha256(target.read_bytes()).hexdigest()
    assert actual==hashlib.sha256(source.read_bytes()).hexdigest(),str(relative)
    entries.append({'path':relative.as_posix(),'bytes':target.stat().st_size,'sha256':actual})
for output,args in [('working-tree.patch',['git','diff','--binary','HEAD']),('staged.patch',['git','diff','--cached','--binary'])]:
    with (backup/output).open('wb') as f:subprocess.run(args,cwd=root,stdout=f,check=True)
record={'version':version,'created_utc':datetime.now(timezone.utc).isoformat(),'head':subprocess.check_output(['git','rev-parse','HEAD'],cwd=root,text=True).strip(),'files':entries,'zip_created':False,'notes':'Private local backup. Node dependencies and historical expanded builds are excluded; lockfiles and the current runnable preview are retained. Includes local configuration; do not upload this directory.'}
(backup/'backup-manifest.json').write_text(json.dumps(record,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
print(json.dumps({'ok':True,'backup':str(backup),'files':len(entries),'bytes':sum(e['bytes'] for e in entries),'zip_created':False}))
