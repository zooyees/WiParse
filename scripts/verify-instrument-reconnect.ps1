param([Parameter(Mandatory=$true)][string]$Exe, [int]$Cycles=10)
$ErrorActionPreference='Stop'
$repoRoot=Split-Path -Parent $PSScriptRoot
$exePath=(Resolve-Path -LiteralPath $Exe).Path
$auditRoot=Join-Path $repoRoot ('tmp/usb-reconnect-runtime-'+[Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $auditRoot | Out-Null
$listener=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,0)
$listener.Start()
$apiPort=$listener.LocalEndpoint.Port
$listener.Stop()
$env:WIPARSE_PROJECT_ROOT=$auditRoot
$env:WIPARSE_APP_ROOT=$auditRoot
$env:WIPARSE_CONFIG=Join-Path $auditRoot 'config.json'
$env:WIPARSE_API_BIND="127.0.0.1:$apiPort"
[IO.File]::WriteAllText($env:WIPARSE_CONFIG,'{"ui":{"debug_mode":false},"apps":{"test_tool":{"marketplace":{"enabled":false}}}}',[Text.UTF8Encoding]::new($false))
$base="http://127.0.0.1:$apiPort"
function Invoke-Instrument($method,$parameters) {
    $body=@{method=$method;params=$parameters}|ConvertTo-Json -Depth 8
    $reply=Invoke-RestMethod "$base/v1/invoke" -Method Post -ContentType 'application/json' -Body $body -TimeoutSec 10
    if(-not $reply.ok){throw "$method failed: $($reply.error)"}
    return $reply.data
}
function Wait-Session($deviceId,$present) {
    $limit=[DateTime]::UtcNow.AddSeconds(10)
    do {
        $inventory=Invoke-Instrument 'instrument.overview' @{}
        $found=@($inventory.devices | Where-Object device_id -eq $deviceId).Count -gt 0
        if($found -eq $present){return $inventory}
        Start-Sleep -Milliseconds 50
    } while([DateTime]::UtcNow -lt $limit)
    throw "Session $deviceId presence did not become $present"
}
# Only simulated transports: this tests the final EXE's lifecycle and duplicate
# open guard. Physical unplug failures are covered separately by fault injection.
$testProcess=Start-Process -FilePath $exePath -WorkingDirectory $auditRoot -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $auditRoot 'stdout.log') -RedirectStandardError (Join-Path $auditRoot 'stderr.log')
try {
    $limit=[DateTime]::UtcNow.AddSeconds(20)
    $health=$null
    do {
        try {$health=Invoke-RestMethod "$base/v1/health" -TimeoutSec 1} catch {Start-Sleep -Milliseconds 100}
        if($testProcess.HasExited){throw 'Preview exited before API readiness'}
    } while(-not $health -and [DateTime]::UtcNow -lt $limit)
    if(-not $health.ok){throw 'API unavailable'}
    $null=Invoke-Instrument 'ui.show' @{tab='instruments'}
    $other=Invoke-Instrument 'instrument.connect' @{resource='DEMO::DC Source';kind='dc_source'}
    $null=Wait-Session $other.device_id $true
    $previousId=$null
    for($cycle=1;$cycle -le $Cycles;$cycle++) {
        $scope=Invoke-Instrument 'instrument.connect' @{resource='DEMO::Oscilloscope';kind='oscilloscope'}
        $inventory=Wait-Session $scope.device_id $true
        if($scope.device_id -eq $previousId){throw 'Replacement reused stale device ID'}
        $duplicate=Invoke-Instrument 'instrument.connect' @{resource='DEMO::Oscilloscope';kind='oscilloscope'}
        Start-Sleep -Milliseconds 200
        $inventory=Invoke-Instrument 'instrument.overview' @{}
        if(@($inventory.devices).Count -ne 2){
            $inventory|ConvertTo-Json -Depth 12|Set-Content -LiteralPath (Join-Path $auditRoot 'failed-inventory.json') -Encoding UTF8
            throw 'Duplicate session was opened or another session was lost'
        }
        if(@($inventory.devices | Where-Object device_id -eq $duplicate.device_id).Count){throw 'Duplicate connect was accepted by worker'}
        $null=Invoke-Instrument 'instrument.disconnect' @{device_id=$scope.device_id}
        $inventory=Wait-Session $scope.device_id $false
        if(@($inventory.devices | Where-Object device_id -eq $other.device_id).Count -ne 1){throw 'Unrelated instrument was disconnected'}
        $previousId=$scope.device_id
    }
    $result=@{kind='simulated_session_lifecycle';cycles=$Cycles;passed=$true;version=$health.data.version;exe_sha256=(Get-FileHash -LiteralPath $exePath -Algorithm SHA256).Hash.ToLowerInvariant();physical_usb_test=$false}
    $result|ConvertTo-Json|Set-Content -LiteralPath (Join-Path $auditRoot 'result.json') -Encoding UTF8
    Write-Output "PASS: $Cycles reconnect cycles, duplicate guard, second instrument preserved. Evidence: $auditRoot"
} finally {
    if(-not $testProcess.HasExited){Stop-Process -Id $testProcess.Id -ErrorAction SilentlyContinue}
}
