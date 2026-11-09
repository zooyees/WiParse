from pathlib import Path
p=Path('docs/verification/next-candidate-20261108-r9/EngineeringAssistant.tsx');s=p.read_text(encoding='utf-8-sig');s=s.replace("import {object,string} from './model';","import {object,string} from './model';\nimport {engineeringPrompt} from './engineeringPrompt';")
s=s.replace("[busy,setBusy]=useState(false),[error,setError]", "[pollAttempt,setPollAttempt]=useState(0),[busy,setBusy]=useState(false),[error,setError]")
a=s.index('    const previous=history?');b=s.index("    setError('');setPreview(value);",a)
s=s[:a]+"    let value:string;try{value=engineeringPrompt(text,context,history?turns:[]);}catch(reason){setError(String(reason));return;}\n"+s[b:]
s=s.replace("const poll=async()=>{try{const result=", "const poll=async()=>{if(Date.now()-started>330000){if(active)setError(localizeLabel('AI 结果仍未确认，请重新查询；不会自动重发。'));return;}try{const result=")
s=s.replace("},[token]);","},[token,pollAttempt]);")
s=s.replace('<div className="settings-actions"><button', '<div className="settings-actions">{token&&error&&<button className="button secondary" onClick={()=>{setError(\'\');setPollAttempt(value=>value+1);}}><LocalizedLabel text="重新查询结果"/></button>}<button')
p.write_text(s,encoding='utf-8',newline='')
