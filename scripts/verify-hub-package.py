"""Verify shipped Hub/plugin code, offline dependencies and portable simulation evidence."""
import argparse
import hashlib
import json
import posixpath
import re
import sys
import zipfile
from pathlib import Path
from urllib.parse import unquote

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('archive', type=Path)
parser.add_argument('--source-manifest', type=Path, help='Frozen path/sha256 manifest; never compare an old ZIP to mutable source')
args = parser.parse_args()
archive = args.archive
with zipfile.ZipFile(archive) as bundle:
    prefix = next(n[:-len('release-manifest.json')] for n in bundle.namelist() if n.endswith('/release-manifest.json'))
    names = {n[len(prefix):] for n in bundle.namelist() if n.startswith(prefix)}
    read = lambda n: bundle.read(prefix+n)
    assert len(bundle.namelist()) == len(set(bundle.namelist())), 'Duplicate ZIP entries'
    manifest = json.loads(read('release-manifest.json'))
    for item in manifest['files']+manifest['symbols']:
        assert hashlib.sha256(read(item['path'])).hexdigest().upper() == item['sha256'].upper(), item['path']
    settings=json.loads(read('config.json'))['apps']['test_tool']
    for key in ['plugins_dir','cli_path','data_root']:
        assert not re.match(r'^(?:[A-Za-z]:|[/\\])',settings[key]), key
    assert isinstance(manifest['version'], str) and manifest['version']
    for name in ['ajv','ajv-formats']:
        assert 'test-tools/node_modules/'+name+'/package.json' in names
    files = [n for n in names if n.startswith('test-tools/plugins/fpga-firmware-test/') and not n.endswith('/')]
    embedded_freeze = 'docs/verification/source-freeze.json'
    if embedded_freeze in names or manifest['version'] == '1.2.1':
        frozen_bytes = read(embedded_freeze)
        assert hashlib.sha256(frozen_bytes).hexdigest().upper() == manifest['source_freeze_sha256'].upper()
        frozen = json.loads(frozen_bytes)
        for item in frozen['files']:
            assert hashlib.sha256(read(item['path'])).hexdigest().lower() == item['sha256'].lower(), item['path']
        if args.source_manifest:
            assert frozen == json.loads(args.source_manifest.read_text(encoding='utf-8-sig')), 'Freeze differs from accepted source'
    elif args.source_manifest:
        frozen = json.loads(args.source_manifest.read_text(encoding='utf-8-sig'))
        for item in frozen['files']:
            assert hashlib.sha256(read(item['path'])).hexdigest().lower() == item['sha256'].lower(), item['path']
    plugin = json.loads(read('test-tools/plugins/fpga-firmware-test/plugin.json'))
    assert plugin['ui']['max_items'] == 500
    cases = [n for n in names if n.startswith('recipes/fpga-firmware-test/cases/') and n.endswith('.json')]
    assert 0 < len(cases) <= 500
    for name in cases:
        case = json.loads(read(name))
        firmware = case['firmware']['path']
        assert not re.match(r'^(?:[A-Za-z]:|[/\\])',firmware), firmware
        target = posixpath.normpath(posixpath.join(posixpath.dirname(name),firmware))
        assert target in names
        assert hashlib.sha256(read(target)).hexdigest() == case['firmware']['sha256']
    run = manifest['fpga_demo']['run']
    record = json.loads(read(run+'/run.json'))
    assert record['simulation_only'] and record['industrial_approved'] is False
    for item in record['artifacts']['items']:
        assert run+'/'+item['path'] in names
    report = run+'/'+record['summary_md']
    text = read(report).decode('utf-8-sig')
    assert '模拟' in text
    links = re.findall(r'\]\(([^)]+)\)',text)
    assert links, 'Demo report contains no evidence links'
    for link in links:
        assert not re.match(r'^(?:[A-Za-z]:|[/\\]|https?:)',link)
        assert posixpath.normpath(posixpath.join(posixpath.dirname(report),unquote(link))) in names,link
    matrix = json.loads(read('projects/default/tests/FpgaFirmwareTest/matrix.json'))
    assert len(matrix['items']) == len(cases)
    assert sum(matrix['summary'].get(s,0) for s in ['pass','fail','error','aborted','skipped','inconclusive','pending','running']) == len(cases), matrix['summary']
    assert not any('/.editor/' in n or n.endswith('fpga-preflight.json') or n.endswith('.station-run.lock') for n in names)
    print(json.dumps({'ok':True,'version':manifest['version'],'plugin_version':plugin['version'],'source_files':len(files),'simulation_cases':len(cases),'evidence_links':len(links),'portable_paths':True,'industrial_approved':False,'frozen_sources_checked':embedded_freeze in names or bool(args.source_manifest)},indent=2))
