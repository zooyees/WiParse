"""Check that a flat instrument UI release carries current previews and evidence."""
import io, json, sys, zipfile
from PIL import Image
from pathlib import Path
archive=Path(sys.argv[1])
with zipfile.ZipFile(archive) as z:
    prefix=next(n[:-len('release-manifest.json')] for n in z.namelist() if n.endswith('/release-manifest.json'))
    read=lambda name:z.read(prefix+name)
    manifest=json.loads(read('release-manifest.json'))
    assert manifest['version']=='1.1.34'
    evidence=json.loads(read('docs/verification/release-1.1.34.json'))
    assert evidence['software_version']==manifest['version']
    assert evidence['instrument_workspace_regressions']=='passed'
    assert evidence['rust_tests_passed']>=389
    assert len(evidence['instrument_runtime_pages'])==8
    assert sum(evidence['ui_render_combinations'].values())==716
    images=0
    for theme in ('dark','light'):
        for page in range(9):
            for tab in (range(3) if page<8 else range(1)):
                name=f'docs/ui-instrument-workspace-1.1.34/instrument-{page}-tab-{tab}-mode-0-{theme}.png'
                with Image.open(io.BytesIO(read(name))) as image:
                    assert image.size==(1100,720)
                    image.verify()
                images+=1
    assert '独立滚动' in read('docs/ui-instrument-workspace-1.1.34.md').decode('utf-8')
    print(json.dumps({'ok':True,'version':manifest['version'],'instrument_workspace_previews':images,'ui_render_combinations':716,'industrial_approved':False}))
