param(
    [Parameter(Mandatory = $true)]
    [string] $ExePath,
    [string] $ReportId = '',
    [int] $LookbackDays = 14,
    [string] $OutputPath = ''
)

$ErrorActionPreference = 'Stop'
$exe = [IO.Path]::GetFullPath($ExePath)
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) {
    throw "Executable not found: $exe"
}
if (-not $OutputPath) {
    $OutputPath = Join-Path (Get-Location).Path 'wiparse-crash-diagnostics.json'
}
$output = [IO.Path]::GetFullPath($OutputPath)
$exeItem = Get-Item -LiteralPath $exe
$root = Split-Path -Parent $exe
if ((Split-Path -Leaf $root) -ieq 'bin') {
    $root = Split-Path -Parent $root
}

$events = @(Get-WinEvent -FilterHashtable @{
    LogName = 'Application'
    Id = 1000, 1001, 1026
    StartTime = (Get-Date).AddDays(-[Math]::Abs($LookbackDays))
} -ErrorAction SilentlyContinue | Where-Object {
    ($_.Message -match '(?i)wiparse\.exe') -or
    ($ReportId -and $_.Message -and $_.Message.Contains($ReportId))
} | Select-Object TimeCreated, Id, ProviderName, Message)

$werRoots = @(
    'C:\ProgramData\Microsoft\Windows\WER\ReportArchive',
    'C:\ProgramData\Microsoft\Windows\WER\ReportQueue'
)
$werReports = @(
    foreach ($werRoot in $werRoots) {
        if (Test-Path -LiteralPath $werRoot) {
            Get-ChildItem -LiteralPath $werRoot -Directory -ErrorAction SilentlyContinue |
                Where-Object { $_.Name -match '(?i)wiparse\.exe' -or ($ReportId -and $_.Name.Contains($ReportId)) } |
                ForEach-Object {
                    $reportFile = Join-Path $_.FullName 'Report.wer'
                    [pscustomobject]@{
                        Directory = $_.FullName
                        LastWriteTime = $_.LastWriteTime
                        Report = if (Test-Path -LiteralPath $reportFile -ErrorAction SilentlyContinue) {
                            Get-Content -LiteralPath $reportFile -Raw -ErrorAction SilentlyContinue
                        } else { $null }
                    }
                }
        }
    }
)

$logs = @(
    foreach ($name in @('wiparse.log', 'crash.log')) {
        $path = Join-Path (Join-Path $root 'logs') $name
        if (Test-Path -LiteralPath $path -PathType Leaf) {
            $item = Get-Item -LiteralPath $path
            [pscustomobject]@{
                Path = $path
                LastWriteTime = $item.LastWriteTime
                Length = $item.Length
                Tail = @(Get-Content -LiteralPath $path -Tail 100 -ErrorAction SilentlyContinue)
            }
        }
    }
)
$dumpDir = Join-Path $env:LOCALAPPDATA 'CrashDumps'
$dumps = @(
    if (Test-Path -LiteralPath $dumpDir) {
        Get-ChildItem -LiteralPath $dumpDir -File -Filter 'WiParse.exe*.dmp' -ErrorAction SilentlyContinue |
            Select-Object FullName, Length, LastWriteTime
    }
)
$result = [pscustomobject]@{
    CollectedAt = Get-Date
    ComputerName = $env:COMPUTERNAME
    ExePath = $exe
    ExeLength = $exeItem.Length
    ExeLastWriteTime = $exeItem.LastWriteTime
    ExeVersion = $exeItem.VersionInfo.FileVersion
    ExeSha256 = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash
    ReportId = $ReportId
    Events = $events
    WerReports = $werReports
    Logs = $logs
    DumpFiles = $dumps
}
$result | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $output -Encoding UTF8
Write-Output $output
