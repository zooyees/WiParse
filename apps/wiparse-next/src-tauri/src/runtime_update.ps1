param([Parameter(Mandatory=$true)][string]$Request)
$ErrorActionPreference='Stop'
Add-Type -AssemblyName System.IO.Compression.FileSystem
$q=Get-Content -LiteralPath $Request -Raw -Encoding UTF8|ConvertFrom-Json
$installLock=$null
function Save-Json($Path,$Value){$temp=$Path+'.'+[Guid]::NewGuid().ToString('N')+'.tmp';$stream=[IO.File]::Create($temp);try{$bytes=[Text.UTF8Encoding]::new($false).GetBytes(($Value|ConvertTo-Json -Depth 32));$stream.Write($bytes,0,$bytes.Length);$stream.Flush($true)}finally{$stream.Dispose()};if(Test-Path -LiteralPath $Path){[IO.File]::Replace($temp,$Path,($Path+'.previous'))}else{[IO.File]::Move($temp,$Path)}}
function Safe-Path([string]$Root,[string]$Relative){
 if(!$Relative -or $Relative.Length -gt 240 -or $Relative -match '[\\:\x00-\x1f]' -or $Relative.StartsWith('/') -or $Relative -match '(^|/)\.\.?(/|$)' -or $Relative -match '(^|/)(con|prn|aux|nul|com[1-9]|lpt[1-9])(\.|/|$)' -or @($Relative.Split('/')|Where-Object{$_ -match '[. ]$' -or !$_}).Count){throw "Invalid runtime path: $Relative"}
 if($Relative -notmatch '^(bin|mcp|test-tools|recipes|Icon)/' -and $Relative -notin @('config.default.json','launch.cmd')){throw "Unmanaged path: $Relative"}
 $base=[IO.Path]::GetFullPath($Root).TrimEnd('\')+'\';$target=[IO.Path]::GetFullPath((Join-Path $Root $Relative));if(!$target.StartsWith($base,[StringComparison]::OrdinalIgnoreCase)){throw 'Path escapes runtime'}
 $walk=$Root;foreach($piece in $Relative.Split('/')){$walk=Join-Path $walk $piece;if((Test-Path -LiteralPath $walk) -and ((Get-Item -LiteralPath $walk -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)){throw 'Reparse point in runtime path'}};return $target
}
function Validate-Manifest($m){
 if($m.product -ne 'wiparse-next' -or $m.schema -ne 1 -or [long]$m.build -le 0 -or !$m.version -or @($m.files).Count -lt 6 -or @($m.files).Count -gt 30000){throw 'Unsupported runtime manifest (legacy single EXE is not supported)'}
 $seen=[Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase);[long]$total=0
 foreach($f in $m.files){$null=Safe-Path $q.root $f.path;if(!$seen.Add($f.path) -or $f.sha256 -notmatch '^[a-fA-F0-9]{64}$' -or [long]$f.bytes -lt 0){throw 'Invalid file manifest'};$total+=[long]$f.bytes};if($total -gt 8GB){throw 'Runtime exceeds 8 GiB installation budget'}
 foreach($p in @('bin/WiParse.exe','bin/WiParse-Host.exe','bin/WiParse-CLI.exe','bin/WiParse-Service.exe','mcp/wiparse/package.json','test-tools/package.json')){if(!$seen.Contains($p)){throw "Missing runtime file: $p"}}
}
function Hash-Matches($path,$f){return (Test-Path -LiteralPath $path -PathType Leaf) -and (Get-Item -LiteralPath $path).Length -eq [long]$f.bytes -and (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -eq $f.sha256}
function Rollback($journal,$session){
 foreach($entry in @($journal.entries)[($journal.entries.Count-1)..0]){if(!$entry){continue};$dst=Safe-Path $q.root $entry.path;$backup=Safe-Path (Join-Path $session 'backup') $entry.path
  if(Test-Path -LiteralPath $backup){[IO.Directory]::CreateDirectory((Split-Path $dst))|Out-Null;[IO.File]::Copy($backup,$dst,$true)}elseif(!$entry.existed -and (Test-Path -LiteralPath $dst)){Remove-Item -LiteralPath $dst -Force}
 }
 $old=Join-Path $session 'previous-manifest.json';if(Test-Path -LiteralPath $old){[IO.File]::Copy($old,(Join-Path $q.root 'runtime-manifest.json'),$true)}
 $journal.phase='rolled_back';Save-Json (Join-Path $session 'journal.json') $journal
}
try{
 foreach($item in @($q.root,(Join-Path $q.root '.updates'),$q.session)){if((Test-Path -LiteralPath $item) -and ((Get-Item -LiteralPath $item -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)){throw 'Reparse point in update root'}}
 $session=[IO.Path]::GetFullPath($q.session);$updates=[IO.Path]::GetFullPath((Join-Path $q.root '.updates')).TrimEnd('\')+'\';if(!$session.StartsWith($updates,[StringComparison]::OrdinalIgnoreCase)){throw 'Invalid update session'}
 if($q.mode -eq 'stage'){
  $baseline=Join-Path $q.root 'runtime-manifest.json';if(!(Test-Path -LiteralPath $baseline)){throw 'This installation has no runtime manifest; use a complete preview directory first'}
  $old=Get-Content -LiteralPath $baseline -Raw -Encoding UTF8|ConvertFrom-Json;Validate-Manifest $old
  [IO.Directory]::CreateDirectory($session)|Out-Null;[IO.File]::Copy($baseline,(Join-Path $session 'previous-manifest.json'),$false)
  $zip=[IO.Compression.ZipFile]::OpenRead($q.archive)
  try{
   $manifestEntries=@($zip.Entries|Where-Object{$_.FullName.Replace('\','/') -eq 'runtime-manifest.json' -or $_.FullName.Replace('\','/') -match '^[^/]+/runtime-manifest.json$'});if($manifestEntries.Count -ne 1){throw 'Archive must contain one runtime-manifest.json'}
   $entry=$manifestEntries[0];if($entry.Length -gt 8MB){throw 'Manifest too large'};$reader=[IO.StreamReader]::new($entry.Open());try{$m=$reader.ReadToEnd()|ConvertFrom-Json}finally{$reader.Dispose()};Validate-Manifest $m
   $manifestName=$entry.FullName.Replace('\','/');$prefix=$manifestName.Substring(0,$manifestName.Length-'runtime-manifest.json'.Length);$index=@{};[long]$sum=0
   if($zip.Entries.Count -gt 40000){throw 'Archive has too many entries'}
   foreach($e in $zip.Entries){$entryName=$e.FullName.Replace('\','/');if($entryName.EndsWith('/')){continue};if(!$entryName.StartsWith($prefix,[StringComparison]::Ordinal)){throw 'Mixed archive roots'};$relative=$entryName.Substring($prefix.Length);if($index.ContainsKey($relative)){throw 'Duplicate archive path'};if($relative -match '(^|/)\.\.?(/|$)|\\|:|^/' -or (($e.ExternalAttributes -shr 16) -band 0xF000) -eq 0xA000){throw 'Unsafe archive entry'};$index[$relative]=$e;$sum+=$e.Length;if($sum -gt 10GB){throw 'Archive expanded budget exceeded'}}
   $stage=Join-Path $session 'stage';foreach($f in $m.files){$e=$index[$f.path];if(!$e -or $e.Length -ne [long]$f.bytes){throw "Archive file missing or length differs: $($f.path)"};$target=Safe-Path $stage $f.path;[IO.Directory]::CreateDirectory((Split-Path $target))|Out-Null;$input=$e.Open();$out=[IO.File]::Open($target,[IO.FileMode]::CreateNew);try{$input.CopyTo($out);$out.Flush($true)}finally{$input.Dispose();$out.Dispose()};if(!(Hash-Matches $target $f)){throw "Checksum mismatch: $($f.path)"}}
   Save-Json (Join-Path $session 'manifest.json') $m
   $oldMap=@{};foreach($f in $old.files){$oldMap[$f.path]=$f};$newMap=@{};foreach($f in $m.files){$newMap[$f.path]=$f}
   $paths=@(@($oldMap.Keys)+@($newMap.Keys)|Sort-Object -Unique);$entries=@()
   foreach($p in $paths){$dst=Safe-Path $q.root $p;$exists=Test-Path -LiteralPath $dst;if($exists -and (!$oldMap.ContainsKey($p) -or !(Hash-Matches $dst $oldMap[$p]))){throw "Installed file has local modifications: $p"};if($oldMap.ContainsKey($p) -and !$exists){throw "Installed managed file missing: $p"};$entries+=@{path=$p;existed=$exists}}
   Save-Json (Join-Path $session 'journal.json') @{phase='prepared';entries=$entries;version=$m.version;build=$m.build}
   @{ok=$true;session=$session;version=$m.version;build=$m.build;files=@($m.files).Count;bytes=($m.files|Measure-Object bytes -Sum).Sum;previous_version=$old.version}|ConvertTo-Json -Compress
  }finally{$zip.Dispose()}
 }elseif($q.mode -in @('apply','rollback')){
  foreach($pidValue in @($q.parent_pid,$q.host_pid)){if([int]$pidValue -gt 0){$process=Get-Process -Id $pidValue -ErrorAction SilentlyContinue;if($process -and !$process.WaitForExit(60000)){throw 'Application is still running; installation remains unchanged'}}}
  $installLock=[IO.File]::Open((Join-Path $q.root '.updates/install.lock'),[IO.FileMode]::OpenOrCreate,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
  $journalPath=Join-Path $session 'journal.json';$journal=Get-Content -LiteralPath $journalPath -Raw -Encoding UTF8|ConvertFrom-Json
  if($q.mode -eq 'rollback'){
   if($journal.phase -notin @('installed','applying')){throw 'This session is not eligible for rollback'}
   foreach($other in Get-ChildItem -LiteralPath (Join-Path $q.root '.updates') -Directory){$otherJournal=Join-Path $other.FullName 'journal.json';if($other.FullName -ne $session -and (Test-Path -LiteralPath $otherJournal)){if((Get-Content -LiteralPath $otherJournal -Raw -Encoding UTF8|ConvertFrom-Json).phase -eq 'applying'){throw 'Recover the interrupted update first'}}}
   if($journal.phase -eq 'installed' -and (Get-FileHash -LiteralPath (Join-Path $q.root 'runtime-manifest.json')).Hash -ne (Get-FileHash -LiteralPath (Join-Path $session 'manifest.json')).Hash){throw 'Only the update matching the installed runtime can be rolled back'}
   Rollback $journal $session
  }else{
   if((Get-FileHash -LiteralPath (Join-Path $q.root 'runtime-manifest.json')).Hash -ne (Get-FileHash -LiteralPath (Join-Path $session 'previous-manifest.json')).Hash){throw 'Installed runtime changed after staging'}
   if($journal.phase -ne 'prepared'){throw 'Update session is not prepared'};$m=Get-Content -LiteralPath (Join-Path $session 'manifest.json') -Raw -Encoding UTF8|ConvertFrom-Json;Validate-Manifest $m
   $old=Get-Content -LiteralPath (Join-Path $session 'previous-manifest.json') -Raw -Encoding UTF8|ConvertFrom-Json;$oldMap=@{};foreach($f in $old.files){$oldMap[$f.path]=$f};$newMap=@{};foreach($f in $m.files){$newMap[$f.path]=$f}
   foreach($entry in $journal.entries){$dst=Safe-Path $q.root $entry.path;if($entry.existed -and !(Hash-Matches $dst $oldMap[$entry.path])){throw 'Installation changed since preparation'};if(!$entry.existed -and (Test-Path -LiteralPath $dst)){throw 'New file conflicts with installation'};if($newMap.ContainsKey($entry.path) -and !(Hash-Matches (Safe-Path (Join-Path $session 'stage') $entry.path) $newMap[$entry.path])){throw 'Staged file changed'}}
   # Back up every old file before changing any managed file. A failed backup is non-destructive.
   foreach($entry in $journal.entries){if($entry.existed){$backup=Safe-Path (Join-Path $session 'backup') $entry.path;[IO.Directory]::CreateDirectory((Split-Path $backup))|Out-Null;[IO.File]::Copy((Safe-Path $q.root $entry.path),$backup,$false)}}
   $journal.phase='applying';Save-Json $journalPath $journal
   try{foreach($entry in $journal.entries){$dst=Safe-Path $q.root $entry.path;if($newMap.ContainsKey($entry.path)){[IO.Directory]::CreateDirectory((Split-Path $dst))|Out-Null;[IO.File]::Copy((Safe-Path (Join-Path $session 'stage') $entry.path),$dst,$true)}elseif(Test-Path -LiteralPath $dst){Remove-Item -LiteralPath $dst -Force}};[IO.File]::Copy((Join-Path $session 'manifest.json'),(Join-Path $q.root 'runtime-manifest.json'),$true);$journal.phase='installed';Save-Json $journalPath $journal}catch{Rollback $journal $session;throw}
  }
  if($q.restart -eq $true){Start-Process -FilePath (Join-Path $q.root 'bin/WiParse.exe') -WorkingDirectory $q.root -WindowStyle Hidden|Out-Null}
 }else{throw 'Unknown update operation'}
}catch{
 if($q.session -and (Test-Path -LiteralPath $q.session)){Save-Json (Join-Path $q.session 'error.json') @{error=$_.Exception.Message;time=[DateTime]::UtcNow.ToString('o')}}
 @{ok=$false;error=$_.Exception.Message}|ConvertTo-Json -Compress;exit 1
}

finally{if($null -ne $installLock){$installLock.Dispose()}}
