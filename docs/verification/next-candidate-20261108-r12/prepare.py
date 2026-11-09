from pathlib import Path
import json
p=Path('apps/wiparse-next/src/ReportWorkspace.tsx');s=p.read_text(encoding='utf-8');s=s.replace("setTreeError(message.includes('桌面应用')?", "setTreeError(message.includes('PROJECT.NOT_FOUND')?'请先在集成测试中创建或选择有效工程。':message.includes('桌面应用')?")
s=s.replace("treeError.includes('桌面窗口')?", "(treeError.includes('桌面窗口')||treeError==='请先在集成测试中创建或选择有效工程。')?")
s=s.replace('>{treeError}</div>', '>{localizeLabel(treeError)}</div>');p.write_text(s,encoding='utf-8',newline='')
p=Path('apps/wiparse-next/src/localization.en.json');d=json.loads(p.read_text(encoding='utf-8'));d['请先在集成测试中创建或选择有效工程。']='Create or select a valid project in Integrated Tests first.';p.write_text(json.dumps(d,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
s=Path('docs/verification/next-candidate-20261108-r10/verify-panels.ps1').read_text(encoding='utf-8');s=s.replace("$allowed=@('apps/wiparse-next/src/style.css','apps/wiparse-next/src/DeviceControls.tsx')","$allowed=@('apps/wiparse-next/src/ReportWorkspace.tsx','apps/wiparse-next/src/localization.en.json')").replace('../next-candidate-20261108-r9/candidate.json','../next-candidate-20261108-r11/candidate.json').replace('Only panel layout and boolean option labels changed.','Only empty-project report guidance and its translation changed.')
Path('docs/verification/next-candidate-20261108-r12/verify-report-empty.ps1').write_text(s,encoding='utf-8',newline='')
