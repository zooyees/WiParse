"""Verify portable ZIP hashes and CodeView/PDB identities."""
import hashlib, json, struct, sys, zipfile
from pathlib import Path

archive = Path(sys.argv[1])
with zipfile.ZipFile(archive) as z:
    assert z.testzip() is None, 'ZIP CRC failure'
    names = z.namelist()
    prefix = next(n[:-len('release-manifest.json')] for n in names if n.endswith('/release-manifest.json'))
    manifest = json.loads(z.read(prefix + 'release-manifest.json'))
    assert manifest['version'] == (sys.argv[2] if len(sys.argv) > 2 else '1.1.30')
    for item in manifest['files'] + manifest['symbols']:
        value = z.read(prefix + item['path'])
        assert hashlib.sha256(value).hexdigest().upper() == item['sha256'].upper(), item['path']
    pairs = [('WiParse.exe', 'wiparse_gui.pdb'), ('WiParse-CLI.exe', 'wiparse.pdb'), ('WiParse-Service.exe', 'wiparse_service.pdb')]
    for exe, pdb in pairs:
        binary = z.read(prefix + 'bin/' + exe)
        symbols = z.read(prefix + 'symbols/' + pdb)
        assert symbols.startswith(b'Microsoft C/C++ MSF 7.00'), pdb
        block_size, _, _, directory_size, _, block_map = struct.unpack_from('<6I', symbols, 32)
        count = (directory_size + block_size - 1)//block_size
        blocks = struct.unpack_from('<' + 'I'*count, symbols, block_map*block_size)
        directory = b''.join(symbols[b*block_size:(b+1)*block_size] for b in blocks)[:directory_size]
        streams = struct.unpack_from('<I', directory)[0]
        sizes = struct.unpack_from('<' + 'I'*streams, directory, 4)
        pos = 4 + 4*streams
        info = None
        for index, size in enumerate(sizes):
            num = 0 if size == 0xffffffff else (size + block_size - 1)//block_size
            stream_blocks = struct.unpack_from('<' + 'I'*num, directory, pos)
            pos += 4*num
            if index == 1:
                info = b''.join(symbols[b*block_size:(b+1)*block_size] for b in stream_blocks)[:size]
        age = struct.unpack_from('<I', info, 8)[0]
        guid = info[12:28]
        expected = b'RSDS' + guid + struct.pack('<I', age)
        assert expected in binary, 'EXE/PDB identity mismatch: ' + exe
    assert prefix + 'docs/ui-layout-audit-2026-10-11.md' in names
    digest = hashlib.sha256(archive.read_bytes()).hexdigest().upper()
    assert archive.with_suffix('.zip.sha256').read_text().split()[0] == digest
    print(json.dumps({'ok': True, 'version': manifest['version'], 'zip_crc': 'passed', 'manifest_hashes': 'passed', 'exe_pdb_guids': '3 matching pairs', 'sha256': digest, 'bytes': archive.stat().st_size}, indent=2))
