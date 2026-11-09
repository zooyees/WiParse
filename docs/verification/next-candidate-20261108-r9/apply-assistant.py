from pathlib import Path
import json,shutil
base=Path('docs/verification/next-candidate-20261108-r9');dst=Path('apps/wiparse-next/src')
for name in ['EngineeringAssistant.tsx','engineeringPrompt.ts']:shutil.copyfile(base/name,dst/name)
shutil.copyfile(base/'engineering-prompt.test.mjs',Path('apps/wiparse-next/tests/engineering-prompt.test.mjs'))
p=dst/'SettingsWorkspace.tsx';s=p.read_text(encoding='utf-8');s=s.replace("import {useEffect,useMemo,useState} from 'react';","import {lazy,Suspense,useEffect,useMemo,useState} from 'react';\nconst EngineeringAssistant=lazy(()=>import('./EngineeringAssistant').then(module=>({default:module.EngineeringAssistant})));")
s=s.replace("useState<'general'|'ai'>('general')","useState<'general'|'ai'|'assistant'>('general')").replace('[preferencesEditing,setPreferencesEditing]=useState(false);','[preferencesEditing,setPreferencesEditing]=useState(false),[assistantEditing,setAssistantEditing]=useState(false);').replace('||preferencesEditing;','||preferencesEditing||assistantEditing;')
old='<LocalizedLabel text={"AI 服务"}/></button></div>'
new='<LocalizedLabel text={"AI 服务"}/></button><button className={`button secondary ${tab===\'assistant\'?\'active\':\'\'}`} role="tab" aria-selected={tab===\'assistant\'} disabled={editing} onClick={()=>setTab(\'assistant\')}><LocalizedLabel text="工程助手"/></button></div>'
assert old in s;s=s.replace(old,new)
old="<PreferencesPanel onEditingChange={setPreferencesEditing}/>:<>";new="<PreferencesPanel onEditingChange={setPreferencesEditing}/>:tab==='assistant'?<Suspense fallback={<p><LocalizedLabel text=\"正在加载…\"/></p>}><EngineeringAssistant onEditingChange={setAssistantEditing}/></Suspense>:<>";assert old in s;s=s.replace(old,new);p.write_text(s,encoding='utf-8',newline='')
p=dst/'style.css';s=p.read_text(encoding='utf-8');s+='''
.settings-page:has(.engineering-assistant) { display:flex; flex-direction:column; overflow:hidden; }
.settings-page:has(.engineering-assistant) > :is(.section-heading,.settings-tabs) { flex:none; }
.engineering-assistant { display:flex; flex-direction:column; min-width:0; min-height:0; flex:1; max-width:1200px; width:100%; align-self:center; }
.engineering-conversation { flex:1; min-height:160px; overflow:auto; padding:12px; }
.engineering-turn { padding:12px 16px; border-bottom:1px solid var(--line); min-width:0; }
.engineering-turn.role-user { margin:8px 0; background:var(--raised); border:1px solid var(--line); border-radius:6px; }
.engineering-turn > header { display:flex; align-items:center; justify-content:space-between; color:var(--muted); font-size:12px; }
.engineering-turn .report-markdown { font-size:14px; line-height:1.65; padding:8px 0; }
.engineering-turn .report-markdown :is(h1,h2,h3,h4) { font-size:15px; margin:12px 0 6px; }
.engineering-composer { max-height:58%; overflow:auto; flex:none; padding:12px 16px; border-top:1px solid var(--line); display:grid; gap:8px; }
.engineering-composer > label:first-child { display:grid; gap:6px; }
.engineering-composer textarea { display:block; width:100%; min-height:80px; max-height:180px; resize:vertical; background:var(--panel); color:var(--text); border:1px solid var(--line); border-radius:5px; padding:8px; font:14px/1.5 'Microsoft YaHei UI','Segoe UI',sans-serif; }
.engineering-composer .assistant-preview pre { max-height:180px; overflow:auto; white-space:pre-wrap; overflow-wrap:anywhere; }
''';p.write_text(s,encoding='utf-8',newline='')
p=dst/'localization.en.json';d=json.loads(p.read_text(encoding='utf-8'));d.update({'清空草稿':'Clear draft','工程助手':'Engineering assistant','工程分析助手':'Engineering analysis assistant','分析问题、日志片段和测试结果；设备自动化请使用串口 AI。':'Analyze questions, log excerpts and test results. Use Serial AI for device automation.','未配置模型':'Model not configured','请先在 AI 服务中配置并启用模型。':'Configure and enable a model in AI services first.','描述需要分析的问题':'Describe the problem to analyze','只发送下方确认的文字，不自动附加工程文件或串口数据。':'Only the confirmed text is sent. Project files and serial data are not attached automatically.','不会自动重发；请核对 AI 服务状态。':'No automatic resend. Verify the AI service state.','正在等待 AI 回复…':'Waiting for AI response…','AI 结果仍未确认，请重新查询；不会自动重发。':'AI result is unconfirmed. Query again; the request will not be resent.','问题与分析要求':'Question and analysis requirements','补充上下文（可选）':'Additional context (optional)','补充上下文':'Additional context','附加最近对话（最多 8 条）':'Include recent conversation (up to 8 messages)','即将发送的完整内容':'Complete text to send','重新查询结果':'Query result again','重新读取设置':'Reload settings','预览发送内容':'Preview request','返回编辑':'Back to editing'});p.write_text(json.dumps(d,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
