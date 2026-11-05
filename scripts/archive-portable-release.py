"""Freeze, ZIP, verify and extract a complete WiParse delivery without overwrites."""
import argparse
import hashlib
import json
import posixpath
import re
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def digest(path):
    sha = hashlib.sha256()
    with path.open('rb') as stream:
        while block := stream.read(1024 * 1024):
            sha.update(block)
    return sha.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    parser.add_argument('--extract-to', type=Path, required=True)
    args = parser.parse_args()
    folder = args.directory.resolve()
    archive = folder.parent / (folder.name + '.zip')
    destination = args.extract_to.resolve()
    assert folder.is_relative_to(ROOT / 'dist')
    assert destination.is_relative_to(ROOT / 'tmp') and not destination.exists()
    assert not archive.exists(), 'Existing releases are preserved'
    manifest_path = folder / 'release-manifest.json'
    manifest = json.loads(manifest_path.read_text(encoding='utf-8'))
    for item in manifest['files']:
        assert digest(folder / item['path']) == item['sha256'], item['path']
    temporary = archive.with_suffix('.zip.tmp')
    assert not temporary.exists()
    with zipfile.ZipFile(temporary, 'x', zipfile.ZIP_DEFLATED, compresslevel=6, allowZip64=True) as bundle:
        for path in sorted(folder.rglob('*')):
            if path.is_file():
                bundle.write(path, str(path.relative_to(folder.parent)).replace('\\', '/'))
    temporary.replace(archive)
    with zipfile.ZipFile(archive) as bundle:
        assert bundle.testzip() is None, 'ZIP CRC error'
        names = bundle.namelist()
        assert len(names) == len(set(names))
        prefix = folder.name + '/'
        assert all(n.startswith(prefix) and not any(p in ('..', '') for p in n.split('/')) for n in names)
        read = lambda rel: bundle.read(prefix + rel)
        assert json.loads(read('release-manifest.json')) == manifest
        for item in manifest['files']:
            assert hashlib.sha256(read(item['path'])).hexdigest() == item['sha256'], item['path']
        assert len(names) == len(manifest['files']) + 1
        for rel in [n.removeprefix(prefix) for n in names]:
            assert not rel.endswith(('.generated.json', 'service-token.json', '.env', '.npmrc'))
            assert not rel.startswith(('projects/', 'logs/', 'evidence/'))
        cfg = json.loads(read('config.json'))
        assert cfg['ai']['enabled'] is False and not cfg['ai']['protected_api_key']
        settings = cfg['apps']['test_tool']
        for key in ['plugins_dir', 'cli_path', 'node_path', 'data_root']:
            assert not re.match(r'^(?:[A-Za-z]:|[/\\])', settings[key]), key
        assert settings['node_path'] == 'runtime/node/node.exe'
        assert read('runtime/node/LICENSE')
        assert read('bin/WiParse.exe')[:2] == b'MZ'
        assert hashlib.sha256(read('bin/WiParse.exe')).hexdigest() == manifest['gui_verified_sha256']
        plugins = [n for n in names if re.fullmatch(re.escape(prefix) + r'test-tools/plugins/[^/]+/plugin.json', n)]
        assert len(plugins) == len(manifest['plugins']) == 9
        cases = [n for n in names if re.fullmatch(re.escape(prefix) + r'recipes/fpga-firmware-test/cases/[^/]+\.json', n)]
        assert len(cases) == 20
        for entry in cases:
            case = json.loads(bundle.read(entry))
            target = posixpath.normpath(posixpath.join(posixpath.dirname(entry), case['firmware']['path']))
            assert target in names and hashlib.sha256(bundle.read(target)).hexdigest() == case['firmware']['sha256']
        destination.mkdir(parents=True)
        bundle.extractall(destination)
    sha = digest(archive)
    archive.with_suffix('.zip.sha256').write_text(f'{sha}  {archive.name}\n', encoding='ascii')
    result = {'ok': True, 'archive': str(archive), 'sha256': sha, 'bytes': archive.stat().st_size,
              'file_count': len(names), 'plugins': len(plugins), 'fpga_cases': len(cases),
              'extracted_to': str(destination / folder.name), 'crc_and_all_file_hashes_verified': True}
    (ROOT / 'tmp/portable-archive-verification.json').write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding='utf-8')
    print(json.dumps(result, ensure_ascii=False))


if __name__ == '__main__':
    main()
