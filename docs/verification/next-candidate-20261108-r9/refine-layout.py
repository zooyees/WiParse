from pathlib import Path
p=Path('docs/verification/next-candidate-20261108-r9/EngineeringAssistant.tsx');s=p.read_text(encoding='utf-8');s=s.replace("}catch(reason){if(active){setError(String(reason));timer=setTimeout(poll,2500);}}", "}catch(reason){if(active){setError(String(reason));if(String(reason).includes('AI.CHAT_NOT_FOUND')){setToken('');setStatus('');}else timer=setTimeout(poll,2500);}}")
s=s.replace('<div className="settings-actions">{token', '<div className="settings-actions"><button className="button secondary" disabled={locked||(!text&&!context&&!preview)} onClick={()=>{setText(\'\');setContext(\'\');setPreview(\'\');}}><LocalizedLabel text="清空草稿"/></button>{token')
p.write_text(s,encoding='utf-8',newline='')
p=Path('docs/verification/next-candidate-20261108-r9/apply-assistant.py');s=p.read_text(encoding='utf-8-sig');s=s.replace('.engineering-assistant {', '.settings-page:has(.engineering-assistant) { display:flex; flex-direction:column; overflow:hidden; }\n.settings-page:has(.engineering-assistant) > :is(.section-heading,.settings-tabs) { flex:none; }\n.engineering-assistant {').replace('.engineering-composer { flex:none;', '.engineering-composer { max-height:58%; overflow:auto; flex:none;')
s=s.replace("'工程助手':'Engineering assistant'", "'清空草稿':'Clear draft','工程助手':'Engineering assistant'")
p.write_text(s,encoding='utf-8',newline='')
