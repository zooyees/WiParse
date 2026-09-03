#Requires -Version 5.1
<#
.SYNOPSIS
  Install WiParse MCP on this machine and write Cursor mcp.json with real paths.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\setup-mcp.ps1 -RegisterUser
#>
param(
    [switch]$RegisterUser,
    [string]$ProjectDir = "",
    [string]$Url = "http://127.0.0.1:7878",
    [switch]$SkipNpm,
    [switch]$VerifyOnly
)

$ErrorActionPreference = "Stop"
$McpDir = $PSScriptRoot
$IndexJs = Join-Path $McpDir "dist\index.js"
$Parent = Split-Path $McpDir -Parent
$Root = Split-Path $Parent -Parent
if (-not (Test-Path (Join-Path $Root "WiParse.exe"))) {
    $Root = $McpDir
}

function Fail([string]$Message) {
    Write-Host "ERROR: $Message" -ForegroundColor Red
    exit 1
}

function ToPosix([string]$Path) {
    $Path.Replace("\", "/")
}

function Find-Node {
    $cmd = Get-Command node -ErrorAction SilentlyContinue
    if ($cmd -and $cmd.Source) {
        return $cmd.Source
    }
    $guesses = @(
        (Join-Path $env:ProgramFiles "nodejs\node.exe"),
        (Join-Path ${env:ProgramFiles(x86)} "nodejs\node.exe"),
        (Join-Path $env:LOCALAPPDATA "Programs\nodejs\node.exe")
    )
    foreach ($g in $guesses) {
        if ($g -and (Test-Path $g)) { return $g }
    }
    return $null
}

function Write-Utf8NoBom([string]$Path, [string]$Text) {
    $enc = New-Object System.Text.UTF8Encoding $false
    [System.IO.File]::WriteAllText($Path, $Text, $enc)
}

function New-McpJson([string]$NodeExe, [string]$IndexPath, [string]$ApiUrl) {
    $n = ToPosix $NodeExe
    $i = ToPosix $IndexPath
    @"
{
  "mcpServers": {
    "wiparse": {
      "command": "$n",
      "args": ["$i"],
      "env": {
        "WIPARSE_URL": "$ApiUrl"
      }
    }
  }
}
"@
}

function Merge-McpJson([string]$NodeExe, [string]$DestPath, [string]$WiparseJson) {
    $dir = Split-Path $DestPath -Parent
    if (-not (Test-Path $dir)) {
        New-Item -ItemType Directory -Path $dir | Out-Null
    }
    $mergeJs = Join-Path $env:TEMP "wiparse-merge-mcp.js"
    $payload = Join-Path $env:TEMP "wiparse-server.json"
    Write-Utf8NoBom $payload $WiparseJson
    Write-Utf8NoBom $mergeJs @'
const fs = require("fs");
const dest = process.argv[2];
const wiparse = JSON.parse(fs.readFileSync(process.argv[3], "utf8"));
let root = { mcpServers: {} };
if (fs.existsSync(dest)) {
  try {
    root = JSON.parse(fs.readFileSync(dest, "utf8"));
  } catch {
    fs.copyFileSync(dest, dest + ".bak");
    root = { mcpServers: {} };
  }
  if (!root || typeof root !== "object") root = {};
  if (!root.mcpServers || typeof root.mcpServers !== "object") root.mcpServers = {};
}
root.mcpServers.wiparse = wiparse;
fs.writeFileSync(dest, JSON.stringify(root, null, 2) + "\n");
'@
    & $NodeExe $mergeJs $DestPath $payload
    if ($LASTEXITCODE -ne 0) { Fail "failed to write $DestPath" }
}

if (-not (Test-Path $IndexJs)) {
    Fail "missing dist\index.js under $McpDir (copy the whole mcp\wiparse folder)"
}

$node = Find-Node
if (-not $node) {
    Fail "Node.js 18+ not found. Install LTS from https://nodejs.org and reopen the terminal."
}

$nodeVer = & $node -v
Write-Host "Node: $node $nodeVer"
if ($nodeVer -match "v(\d+)") {
    $major = [int]$Matches[1]
    if ($major -lt 18) {
        Fail "Node $nodeVer is too old; need 18+"
    }
}

$sdk = Join-Path $McpDir "node_modules\@modelcontextprotocol\sdk"
if ($VerifyOnly) {
    if (-not (Test-Path $sdk)) {
        Fail "node_modules missing; run setup-mcp.ps1 without -VerifyOnly"
    }
} elseif (-not $SkipNpm -and -not (Test-Path $sdk)) {
    $npm = Get-Command npm -ErrorAction SilentlyContinue
    if (-not $npm) {
        Fail "npm not found; install Node.js LTS (includes npm), or copy node_modules with this folder"
    }
    Write-Host "Installing production deps (npm install --omit=dev)..."
    Push-Location $McpDir
    try {
        npm install --omit=dev
        if ($LASTEXITCODE -ne 0) { Fail "npm install failed" }
    } finally {
        Pop-Location
    }
} elseif (Test-Path $sdk) {
    Write-Host "Using existing node_modules"
} elseif ($SkipNpm) {
    Fail "node_modules missing and -SkipNpm was set"
}

$fullJson = New-McpJson $node $IndexJs $Url
$wiparseJson = @"
{"command":"$(ToPosix $node)","args":["$(ToPosix $IndexJs)"],"env":{"WIPARSE_URL":"$Url"}}
"@

$generated = Join-Path $McpDir "cursor.mcp.generated.json"
Write-Utf8NoBom $generated $fullJson
Write-Host "Wrote $generated"

if (Test-Path (Join-Path $Root "WiParse.exe")) {
    $rootCfg = Join-Path $Root "cursor.mcp.json"
    Write-Utf8NoBom $rootCfg $fullJson
    Write-Host "Wrote $rootCfg"
}

if ($RegisterUser) {
    $userMcp = Join-Path $env:USERPROFILE ".cursor\mcp.json"
    Merge-McpJson $node $userMcp $wiparseJson
    Write-Host "Updated $userMcp"
}

if ($ProjectDir) {
    $projMcp = Join-Path $ProjectDir ".cursor\mcp.json"
    Merge-McpJson $node $projMcp $wiparseJson
    Write-Host "Updated $projMcp"
}

Write-Host ""
Write-Host "Health check $Url ..."
try {
    $health = Invoke-RestMethod -Uri "$Url/v1/health" -TimeoutSec 3
    Write-Host ("GUI API ok: " + ($health | ConvertTo-Json -Compress))
} catch {
    Write-Host "GUI API not reachable. Start WiParse.exe first, then restart Cursor." -ForegroundColor Yellow
}

Write-Host ""
Write-Host "Done. Restart Cursor and confirm tools: wiparse_brief, wiparse_select, wiparse_test, wiparse_send, wiparse_report_pack"
if (-not $RegisterUser -and -not $ProjectDir) {
    Write-Host "This run did not register Cursor. Re-run with -RegisterUser, or copy cursor.mcp.generated.json into %USERPROFILE%\.cursor\mcp.json"
}
