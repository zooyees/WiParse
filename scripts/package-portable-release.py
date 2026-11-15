"""Assemble a relocatable Windows delivery from verified local release artifacts."""
import argparse
import hashlib
import json
import shutil
import struct
import subprocess
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest() if hasattr(hashlib, 'file_digest') else hashlib.sha256(stream.read()).hexdigest()


def write(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    text = value if isinstance(value, str) else json.dumps(value, ensure_ascii=False, indent=2)
    if path.suffix.lower() == '.cmd':
        path.write_bytes(text.replace('\r\n', '\n').replace('\n', '\r\n').encode('utf-8'))
    else:
        path.write_text(text, encoding='utf-8')


def copy_tree(source, target):
    shutil.copytree(source, target, ignore=shutil.ignore_patterns(
        '.git', '.env', '.env.*', '.npmrc', '__pycache__', 'logs',
        'run.stop', 'run.lock', '_loop_status.json', '_loop_hint.txt', '*.generated.json', '*.bak'))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--node-dir', type=Path, required=True)
    parser.add_argument('--vc-redist', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--current', action='store_true', help='Acknowledge that target/release binaries are supplied separately and are not verified by this script')
    args = parser.parse_args()
    out = args.output.resolve()
    assert out.is_relative_to(ROOT / 'dist') and not out.exists(), 'Use a fresh workspace dist directory'
    cargo = (ROOT / 'Cargo.toml').read_text(encoding='utf-8')
    version = next(line.split('"', 2)[1] for line in cargo.splitlines() if line.startswith('version = "'))
    gui_path = ROOT / 'target/release/wiparse-gui.exe'
    gui_sha = sha(gui_path)
    if not args.current:
        parser.error('pass --current to acknowledge that this script does not verify the supplied release binaries')
    accepted = {'version': version, 'exe_sha256': gui_sha, 'full_gui': 'not verified by this packaging script'}
    assert (args.node_dir / 'LICENSE').is_file() and (args.node_dir / 'node.exe').is_file()
    out.mkdir(parents=True)
    (out / 'bin').mkdir()
    (out / 'symbols').mkdir()
    for source, target in [('wiparse-gui.exe', 'WiParse.exe'), ('wiparse.exe', 'WiParse-CLI.exe'), ('wiparse-service.exe', 'WiParse-Service.exe')]:
        shutil.copy2(ROOT / 'target/release' / source, out / 'bin' / target)
    for name in ['wiparse_gui.pdb', 'wiparse.pdb', 'wiparse_service.pdb']:
        assert (ROOT / 'target/release' / name).is_file(), name
        shutil.copy2(ROOT / 'target/release' / name, out / 'symbols' / name)
    copy_tree(ROOT / 'test-tools', out / 'test-tools')
    copy_tree(ROOT / 'mcp/wiparse', out / 'mcp/wiparse')
    copy_tree(ROOT / 'recipes', out / 'recipes')
    copy_tree(ROOT / 'Icon', out / 'Icon')
    copy_tree(args.node_dir, out / 'runtime/node')
    (out / 'runtime/vcredist').mkdir()
    shutil.copy2(args.vc_redist, out / 'runtime/vcredist/VC_redist.x64.exe')
    # Ship available x64 D2XX, preserving bytes and its original filename.
    (out / 'vendor/ftdi').mkdir(parents=True)
    shutil.copy2(ROOT / 'vendor/ftdi/ftd2xx64.dll', out / 'vendor/ftdi/ftd2xx64.dll')
    shutil.copy2(ROOT / 'vendor/ftdi/README.md', out / 'vendor/ftdi/README.md')
    default = json.loads((ROOT / 'config.default.json').read_text(encoding='utf-8-sig'))
    default['ui']['language'] = 'zh'
    default['apps']['test_tool'].update(plugins_dir='test-tools/plugins', cli_path='bin/WiParse-CLI.exe', node_path='runtime/node/node.exe', data_root='.')
    default['apps']['test_tool']['marketplace']['enabled'] = False
    write(out / 'config.default.json', default)
    write(out / 'config.json', {'ai': {'enabled': False, 'protected_api_key': ''}, 'ui': {'language': 'zh', 'theme': 'dark'},
        'apps': {'test_tool': {'plugins_dir': 'test-tools/plugins', 'cli_path': 'bin/WiParse-CLI.exe', 'node_path': 'runtime/node/node.exe',
                             'data_root': '.', 'marketplace': {'enabled': False, 'install_dir': 'marketplace'}}}})
    write(out / 'mcp/wiparse/cursor.mcp.example.json', {'mcpServers': {'wiparse': {
        'command': '<WiParse解压目录>/runtime/node/node.exe', 'args': ['<WiParse解压目录>/mcp/wiparse/dist/index.js'],
        'env': {'WIPARSE_URL': 'http://127.0.0.1:7878'}}}})
    documents = ['DEPLOY_MCP.md', 'DEPLOY_API.md', 'CLI_REFERENCE.md', 'WORKSTATION_CLOSED_LOOP.md', 'AI_INTERFACE.md', 'ROUTER_SKILLS.md',
        'TESTING_HUB_WORKSPACE.md', 'HUB_UI_FIELDS_CONTRACT.md', 'HUB_PROJECT_CONFIG_CONTRACT.md', 'AUTOMATION-SERVICE.md', 'CHANGELOG.md']
    for name in documents:
        target = out / 'docs' / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / 'docs' / name, target)
    # Keep links in ROUTER_SKILLS.md and the candidate audit resolvable. These
    # are historical records, not verification of the binaries being copied.
    for name in ['verification-r65.md', 'verification-r79-attachment-invalidation.md',
                 'verification-r81-cursor-version.md', 'verification-r82-delivery-audit.md',
                 'source-manifest-r82.json']:
        rel = Path('verification/router-skill-20261113') / name
        (out / 'docs' / rel).parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / 'docs' / rel, out / 'docs' / rel)
    copy_tree(ROOT / 'docs/specs/wireless-test', out / 'docs/specs/wireless-test')
    copy_tree(ROOT / 'docs/examples', out / 'docs/examples')
    notes = f'# WiParse {accepted["version"]} 便携目录\n\n'
    notes += '此目录的构建与验证范围以 release-manifest.json 和随附验证记录为准；旧验证记录不证明本次二进制已验收。\n'
    write(out / 'docs/release-notes.md', notes)
    for name in ['generate-fpga-hub-demo.mjs', 'sync-ftdi-dlls.ps1']:
        target = out / 'scripts' / name
        target.parent.mkdir(exist_ok=True)
        shutil.copy2(ROOT / 'scripts' / name, target)
    write(out / 'Start-WiParse.cmd', '@echo off\nsetlocal\npushd "%~dp0"\nset "PATH=%~dp0runtime\\node;%PATH%"\nstart "" /D "%~dp0" "%~dp0bin\\WiParse.exe"\npopd\n')
    write(out / 'README-更新与使用.md', f'''# WiParse {accepted['version']} Windows x64 便携目录

包含 GUI、CLI、Service、已编译 MCP、本地插件、离线 npm 依赖、Node.js 运行时和许可证、VC++ x64 离线安装程序、示例配方、文档及调试符号。具体版本和文件以 release-manifest.json 为准；模拟示例不代表真实设备验收。无线充电插件 live 执行仍受阻，详见 docs/ROUTER_SKILLS.md。

## 新目录使用

1. 将整个文件夹解压到可写目录；不要在压缩包中直接运行，也不要只复制exe。
2. 双击 Start-WiParse.cmd；也可直接运行 bin/WiParse.exe。
3. 若提示缺少 VCRUNTIME140.dll，手动运行 runtime/vcredist/VC_redist.x64.exe 安装/修复后重试。压缩包不自动安装任何系统组件。
4. 插件和MCP使用内置 runtime/node/node.exe，不要求安装Node或执行npm install。
5. AI服务地址、模型与密钥在另一台电脑上配置；本包不携带打包电脑的密钥。串口AI操作设备前勾选“允许控制”并重新发送。

## 更新另一台电脑已有安装

先退出旧版GUI、CLI、Service及相关MCP进程，备份原安装目录。推荐先把本包解压到新目录检查。

原位更新时只覆盖程序和配套资源：bin、runtime、test-tools、mcp、Icon、vendor、scripts、docs、symbols以及启动文件。若旧版exe在安装根目录，请以后从本包Start-WiParse.cmd/bin/WiParse.exe启动，避免仍运行旧根目录exe。

保留目标电脑原有config.json、projects、logs/log、evidence、数据库及个人数据；自定义recipes也须保留，不要用本包示例配方整目录覆盖。config.default.json可更新，已有config.json不要直接覆盖。检查集成测试设置中的相对路径：插件test-tools/plugins，CLI bin/WiParse-CLI.exe，Node runtime/node/node.exe，数据根目录按原工程实际位置保留。

不要从另一台电脑复制加密AI密钥；既有目标机配置可继续保留，必要时在该机重新配置。

## MCP

先启动GUI，再运行 mcp/wiparse/setup-mcp.cmd，在目标电脑注册Cursor，之后完全退出并重开Cursor。脚本优先使用包内Node，并按目标机器解压位置生成注册路径；移动目录后需重新注册。包内不包含打包机器的生成注册文件。

## 系统与仪器依赖

目标系统为Windows x64。VISA（NI-VISA/TekVISA等）、USB串口/调试探针驱动按目标电脑实际仪器安装；更新不会替换这些系统驱动。FTDI附带现有x64 D2XX；FT4222的SPI/I2C/GPIO还需厂商LibFT4222，当前仓库未提供该库，不能以本包替代完整FT4222/VISA驱动安装。

## 校验与诊断

release-manifest.json列出全部文件SHA256、程序/插件/运行时版本及外部依赖。symbols中的PDB与本包EXE配对；系统报错时请保留这些文件。示例为工程/模拟演示，不代表真实硬件或工业验收通过。
''')
    plugins = sorted((json.loads(p.read_text(encoding='utf-8-sig')) for p in (out / 'test-tools/plugins').glob('*/plugin.json')), key=lambda p: p['id'])
    mcp_version = json.loads((out / 'mcp/wiparse/package.json').read_text(encoding='utf-8'))['version']
    assert plugins, 'No local plugins found'
    for dep in ['test-tools/node_modules/ajv/package.json', 'test-tools/node_modules/ajv-formats/package.json',
                'mcp/wiparse/node_modules/@modelcontextprotocol/sdk/package.json', 'mcp/wiparse/node_modules/zod/package.json',
                'mcp/wiparse/dist/index.js', 'mcp/wiparse/dist/http.js']:
        assert (out / dep).is_file(), dep
    manifest = {'kind': 'full_portable', 'version': accepted['version'], 'platform': 'windows-x64',
        'created_at': datetime.now(timezone.utc).isoformat(), 'mcp_version': mcp_version,
        'plugins': [{'id': p['id'], 'version': p['version']} for p in plugins],
        'node_version': subprocess.check_output([str(args.node_dir / 'node.exe'), '--version'], text=True).strip(),
        'vc_redist_sha256': sha(args.vc_redist), 'gui_sha256': accepted['exe_sha256'],
        'prior_gui_tests': accepted['full_gui'], 'credential_scan': 'not performed by this script',
        'external_dependencies': ['Windows x64', 'Instrument VISA and USB drivers for the actual hardware', 'LibFT4222 for optional FT4222 SPI/I2C/GPIO'],
        'files': [{'path': str(p.relative_to(out)).replace('\\', '/'), 'size': p.stat().st_size, 'sha256': sha(p)}
                  for p in sorted(out.rglob('*')) if p.is_file()]}
    write(out / 'release-manifest.json', manifest)
    print(json.dumps({'directory': str(out), 'files': len(manifest['files']), 'plugins': manifest['plugins'], 'gui_sha256': manifest['gui_sha256']}, ensure_ascii=False))


if __name__ == '__main__':
    main()
