param([string]$Exe=(Join-Path $PSScriptRoot '../target/debug/wiparse-gui.exe'),[string]$Cli=(Join-Path $PSScriptRoot '../target/debug/wiparse.exe'))
$ErrorActionPreference='Stop'
$repo=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$exePath=(Resolve-Path -LiteralPath $Exe).Path
$root=Join-Path $repo ('tmp/fpga-compiled-'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path (Join-Path $root 'recipes') | Out-Null
Copy-Item -LiteralPath (Join-Path $repo 'recipes/fpga-firmware-test') -Destination (Join-Path $root 'recipes/fpga-firmware-test') -Recurse
Get-ChildItem -LiteralPath (Join-Path $root 'recipes/fpga-firmware-test/cases') -Filter '*.json' -File | ForEach-Object {
 $value=Get-Content -LiteralPath $_.FullName -Raw -Encoding UTF8 | ConvertFrom-Json
 $value.log_capture.duration_s=3
 foreach($wave in $value.waveform_captures){$wave.at_s=0.03;$wave.timeout_s=1}
 [IO.File]::WriteAllText($_.FullName,($value|ConvertTo-Json -Depth 50),[Text.UTF8Encoding]::new($false))
}
$node=(Get-Command node -ErrorAction Stop).Source
$config=@{ui=@{language='zh';theme='dark'};system=@{db_name=(Join-Path $root 'verification.db');log_file=(Join-Path $root 'verification.log')};apps=@{test_tool=@{plugins_dir=(Join-Path $repo 'test-tools/plugins');cli_path=(Resolve-Path -LiteralPath $Cli).Path;node_path=$node;data_root=$root;marketplace=@{enabled=$false}}}}
$configPath=Join-Path $root 'config.json'
[IO.File]::WriteAllText($configPath,($config|ConvertTo-Json -Depth 20),[Text.UTF8Encoding]::new($false))
$listener=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,0);$listener.Start();$port=$listener.LocalEndpoint.Port;$listener.Stop()
$env:WIPARSE_PROJECT_ROOT=$repo;$env:WIPARSE_APP_ROOT=$root;$env:WIPARSE_CONFIG=$configPath;$env:WCM_CONFIG=$configPath;$env:WIPARSE_API_BIND="127.0.0.1:$port"
$process=Start-Process -FilePath $exePath -WorkingDirectory $repo -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $root 'stdout.log') -RedirectStandardError (Join-Path $root 'stderr.log')
$base="http://127.0.0.1:$port"
function Invoke-Fpga($method,$parameters){$body=@{method=$method;params=$parameters}|ConvertTo-Json -Depth 25 -Compress;$reply=Invoke-RestMethod "$base/v1/invoke" -Method Post -ContentType 'application/json; charset=utf-8' -Body ([Text.Encoding]::UTF8.GetBytes($body)) -TimeoutSec 70;if(-not $reply.ok){throw "$method failed: $($reply|ConvertTo-Json -Depth 10 -Compress)"};return $reply.data}
function Wait-Fpga($predicate){$deadline=[DateTime]::UtcNow.AddSeconds(60);do{if($process.HasExited){throw 'Verification program exited'};$state=Invoke-Fpga 'ui.test_tool.matrix.get' @{plugin='fpga-firmware-test';limit=500};if(&$predicate $state){return $state};Start-Sleep -Milliseconds 100}while([DateTime]::UtcNow -lt $deadline);throw 'Verification timed out'}
try{
 $deadline=[DateTime]::UtcNow.AddSeconds(30);$health=$null
 while(-not $health -and [DateTime]::UtcNow -lt $deadline){try{$health=Invoke-RestMethod "$base/v1/health" -TimeoutSec 1}catch{Start-Sleep -Milliseconds 100}}
 if(-not $health.ok){throw 'GUI did not start'}
 $null=Invoke-Fpga 'ui.show' @{tab='test_tool'}
 $parameters=@{execution_mode='simulation';config_dir='recipes/fpga-firmware-test/cases'}
 $listed=Invoke-Fpga 'ui.test_tool.config.list' @{plugin='fpga-firmware-test';limit=500}
 if(@($listed.items).Count -ne 20){throw 'Expected 20 valid configurations'}
 $plan=Invoke-Fpga 'ui.test_tool.project.migration.preview' @{plugin='fpga-firmware-test'}
 if(-not $plan.ok -or $plan.conflicts.Count -gt 0){throw 'Isolated simulation migration conflicts'}
 $null=Invoke-Fpga 'ui.test_tool.project.migration.commit' @{plugin='fpga-firmware-test';plan_sha256=$plan.plan_sha256}
 $listed=Invoke-Fpga 'ui.test_tool.config.list' @{plugin='fpga-firmware-test';limit=500}
 if(@($listed.items|Where-Object {$_.errors.Count -gt 0}).Count -gt 0){throw 'Expected valid migrated configurations'}
 $ids=@('fpga_01_pll_clock','fpga_02_reset_controller')
 $null=Invoke-Fpga 'ui.test_tool.run' @{id='fpga-firmware-test';preflight=$true;params=$parameters;test_matrix=@{selected_item_ids=$ids}}
 $deadline=[DateTime]::UtcNow.AddSeconds(60)
 do{Start-Sleep -Milliseconds 200;$state=Invoke-Fpga 'ui.state' @{};if(-not $state.test_tool.running){break}}while([DateTime]::UtcNow -lt $deadline)
 $pre=Invoke-Fpga 'ui.test_tool.preflight.state' @{plugin='fpga-firmware-test'};if(-not $pre.valid){throw 'Preflight not valid'}
 $null=Invoke-Fpga 'ui.test_tool.run' @{id='fpga-firmware-test';preflight=$false;params=$parameters;test_matrix=@{config_manifest_sha256=$pre.manifest_sha256;preflight_revision=$pre.revision;selected_item_ids=$ids}}
 $running=Wait-Fpga {param($s) $s.summary.running -eq 1}
 $null=Invoke-Fpga 'ui.test_tool.run.control' @{plugin='fpga-firmware-test';run_id=$running.run_id;action='pause'}
 $paused=Wait-Fpga {param($s) $s.control.state -eq 'paused'}
 if($paused.summary.running -ne 0 -or $paused.summary.pending -lt 1){throw 'Pause was not at an item boundary'}
 $null=Invoke-Fpga 'ui.test_tool.run.control' @{plugin='fpga-firmware-test';run_id=$paused.run_id;action='resume'}
 $continued=Wait-Fpga {param($s) $s.summary.running -eq 1 -and $s.active_item_id -eq 'fpga_02_reset_controller'}
 $null=Invoke-Fpga 'ui.test_tool.stop' @{}
 $final=Wait-Fpga {param($s) $s.summary.running -eq 0 -and $s.summary.aborted -eq 1}
 $result=@{ok=$true;simulation_only=$true;configuration_count=20;pause='acknowledged_at_boundary';resume='second_item_started';stop='second_item_aborted';summary=$final.summary;data_root=$root;executable=$exePath}
 [IO.File]::WriteAllText((Join-Path $root 'verification.json'),($result|ConvertTo-Json -Depth 20),[Text.UTF8Encoding]::new($false))
 $result|ConvertTo-Json -Depth 20
}finally{if(-not $process.HasExited){Stop-Process -Id $process.Id -ErrorAction SilentlyContinue}}
