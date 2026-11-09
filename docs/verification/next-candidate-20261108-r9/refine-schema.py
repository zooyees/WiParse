from pathlib import Path
p=Path('docs/verification/next-candidate-20261108-r9/EngineeringAssistant.tsx');s=p.read_text(encoding='utf-8');s=s.replace('const fields=object(settings?.fields)','const fields=object(settings)');s=s.replace("if(result.state==='running'){if(Date.now()-started>330000){setError(localizeLabel('AI 结果仍未确认，请重新查询；不会自动重发。'));return;}timer", "if(result.state==='running'){timer")
p.write_text(s,encoding='utf-8',newline='')
