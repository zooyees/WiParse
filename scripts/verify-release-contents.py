"""Verify the MCP build and demo data are complete and relocatable in the ZIP."""
import json
import re
import sys
import zipfile
from pathlib import Path, PurePosixPath
from urllib.parse import unquote

archive = Path(sys.argv[1])
root = Path(__file__).resolve().parents[1]
with zipfile.ZipFile(archive) as bundle:
    prefix = next(n[:-len('release-manifest.json')] for n in bundle.namelist() if n.endswith('/release-manifest.json'))
    names = {n[len(prefix):] for n in bundle.namelist() if n.startswith(prefix)}
    read = lambda n: bundle.read(prefix+n)
    manifest = json.loads(read('release-manifest.json'))
    package = json.loads(read('mcp/wiparse/package.json'))
    assert package['version'] == manifest['version'] == manifest['mcp_version']
    for name in ['index.js','http.js','index.d.ts','http.d.ts']:
        rel = 'mcp/wiparse/dist/'+name
        assert read(rel) == (root/rel).read_bytes(), 'Stale compiled MCP: '+name
    assert 'mcp/wiparse/node_modules/@modelcontextprotocol/sdk/package.json' in names
    assert 'mcp/wiparse/node_modules/zod/package.json' in names
    assert 'mcp/wiparse/cursor.mcp.generated.json' not in names
    run = manifest['demo']['run']
    doc = json.loads(read(run+'/run.json'))
    assert doc['simulation_only'] and doc['product_verdict'] == 'INCONCLUSIVE'
    for item in doc['artifacts']['items']:
        assert run+'/'+item['path'] in names
    report = run+'/'+doc['summary_md']
    text = read(report).decode('utf-8-sig')
    links = re.findall(r'\]\((<[^>]*>|(?:[^()]|\([^()]*\))*)\)',text)
    for link in links:
        target = PurePosixPath(report).parent / unquote(link.strip('<>'))
        parts = []
        for part in target.parts:
            if part == '..': parts.pop()
            elif part != '.': parts.append(part)
        assert '/'.join(parts) in names, 'Broken report link: '+link
    assert len(links) == 14
    assert not any(n.endswith('service-token.json') or n.endswith('config.json.bak') for n in names)
    assert json.loads(read('docs/verification/release-1.1.30.json'))['software_version'] == manifest['version']
    print(json.dumps({'ok':True,'version':manifest['version'],'mcp':'compiled source matches; offline dependencies present','demo_links':len(links),'demo_cycles':3,'industrial_approved':False},indent=2))
