from pathlib import Path
s=Path('docs/verification/next-candidate-20261108-r12/verify-report-empty.ps1').read_text(encoding='utf-8')
s=s.replace('$before.Count -ne @($parent.source_hashes.PSObject.Properties).Count','$before.Count -ne (@($parent.source_hashes.PSObject.Properties).Count+1)')
a=s.index('$allowed=@(');b=s.index('\nforeach($set',a)
s=s[:a]+'''$allowed=@('apps/wiparse-next/src/SerialWorkspace.tsx','apps/wiparse-next/src/serialSearch.ts','apps/wiparse-next/tests/serial-search.test.mjs','apps/wiparse-next/src/WaveWorkspace.tsx','apps/wiparse-next/src/WavePlot.tsx','apps/wiparse-next/src/waveCursor.ts','apps/wiparse-next/tests/wave-cursor.test.mjs','apps/wiparse-next/src/style.css','apps/wiparse-next/src/localization.en.json','apps/wiparse-next/src-tauri/src/main.rs','apps/wiparse-next/src-tauri/src/wave_view.rs')
if($changed.Count -ne 11 -or @($changed|Where-Object {$_ -notin $allowed}).Count){throw "Only the recorded serial/wave parity sources may differ: $changed"}'''+s[b:]
s=s.replace('../next-candidate-20261108-r11/candidate.json','../next-candidate-20261108-r12/candidate.json')
s=s.replace('Only empty-project report guidance and its translation changed.','Only the recorded serial/wave navigation, rendering component and native waveform export adapter changed. Core/drivers/host/plugins/MCP remain hash-identical.')
Path('docs/verification/next-candidate-20261108-r13/verify-parity.ps1').write_text(s,encoding='utf-8',newline='')
