from pathlib import Path
p=Path('docs/verification/next-candidate-20261108-r9/verify-frontend.ps1');s=p.read_text(encoding='utf-8');s=s.replace('$before.Count -ne @($parent.source_hashes.PSObject.Properties).Count','$before.Count -ne (@($parent.source_hashes.PSObject.Properties).Count+3)')
s=s.replace("'apps/wiparse-next/tests/wave-cursor.test.mjs')","'apps/wiparse-next/tests/wave-cursor.test.mjs','apps/wiparse-next/src/SettingsWorkspace.tsx','apps/wiparse-next/src/localization.en.json','apps/wiparse-next/src/EngineeringAssistant.tsx','apps/wiparse-next/src/engineeringPrompt.ts','apps/wiparse-next/tests/engineering-prompt.test.mjs')")
s=s.replace('$changed.Count -ne 4','$changed.Count -ne 9')
s=s.replace('Only recorded frontend presentation and finite-extent corrections changed.','Only recorded frontend presentation, finite-extent corrections and the engineering-assistant UI changed; its existing typed host API is unchanged.')
p.write_text(s,encoding='utf-8',newline='')
