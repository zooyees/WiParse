"""Validate current workbench release evidence and actual UI previews."""
import io, json, sys, zipfile
from PIL import Image
with zipfile.ZipFile(sys.argv[1]) as bundle:
    prefix = next(n[:-len('release-manifest.json')] for n in bundle.namelist() if n.endswith('/release-manifest.json'))
    read = lambda name: bundle.read(prefix + name)
    manifest = json.loads(read('release-manifest.json'))
    assert manifest['version'] == '1.1.36'
    evidence = json.loads(read('docs/verification/release-1.1.36.json'))
    assert evidence['software_version'] == '1.1.36'
    assert evidence['unified_workbench_layout_regressions'] == 'passed'
    assert evidence['rust_tests_passed'] >= 397
    assert len(evidence['instrument_runtime_pages']) == 8
    images = [n for n in bundle.namelist() if '/docs/ui-release-1.1.36/' in n and n.endswith('.png')]
    assert len(images) == 26, len(images)
    for name in images:
        with Image.open(io.BytesIO(bundle.read(name))) as image:
            assert image.width >= 1000 and image.height >= 700
            image.verify()
    print(json.dumps({'ok': True, 'version': '1.1.36', 'previews': len(images)}))
