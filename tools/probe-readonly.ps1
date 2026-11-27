# Hangar - a community-made updater and launcher for DCS World.
# Copyright (C) 2026 ChonkieMonkie
# SPDX-License-Identifier: GPL-3.0-or-later
# Free software under the GNU GPL v3 or later, WITHOUT ANY WARRANTY. See LICENSE or <https://www.gnu.org/licenses/>.

<#
.SYNOPSIS
  Hangar - read-only probe of a DCS World installation.

.DESCRIPTION
  Answers the open questions in docs/INVESTIGATION.md (level 1) by READING:
    - Eagle Dynamics registry keys (sensitive-looking values are hidden)
    - autoupdate.cfg structure and module IDs
    - bin / bin-mt executables and their file versions
    - existing updater logs (autoupdate_log.txt, %TEMP%\DCS*\autoupdate_templog.txt)
    - Saved Games\DCS* folder names and sizes, format of Logs\dcs.log
    - Tacview and Videos (NVIDIA) folders, Steam traces

  SAFETY
    - Never writes, moves, renames or deletes anything outside the report folder.
    - The only write is Save-Report, which refuses any path outside -OutDir.
    - At start the script parses itself and aborts if it contains a write/delete/
      process-starting cmdlet anywhere.
    - Files are opened read-only with shared access, so nothing is locked.
    - Never reads Config contents or authdata.bin; only sizes.
    - Never starts DCS.exe or DCS_updater.exe.

  Output: <OutDir>\probe-<timestamp>.md and .json (OutDir defaults to ..\investigation).

.NOTES
  Windows PowerShell 5.1 compatible. Not affiliated with Eagle Dynamics SA.
#>
[CmdletBinding()]
param(
  [string]$OutDir = '',
  [int]$LogHeadLines = 60,
  [int]$LogTailLines = 200
)

$ErrorActionPreference = 'Stop'
# PS 5.1: $PSScriptRoot is empty inside param defaults, so resolve here.
if (-not $OutDir) { $OutDir = Join-Path (Split-Path -Parent $PSCommandPath) '..\investigation' }
Set-StrictMode -Version 2

# ---------------------------------------------------------------- self-check
$denied = @(
  'Remove-Item','Set-Content','Add-Content','Out-File','New-Item','Move-Item','Copy-Item',
  'Rename-Item','Clear-Content','Clear-Item','Set-Item','Set-ItemProperty','New-ItemProperty',
  'Remove-ItemProperty','Rename-ItemProperty','Copy-ItemProperty','Move-ItemProperty',
  'Start-Process','Invoke-Item','Stop-Process','Invoke-Expression','Export-Csv','Tee-Object',
  'Set-Acl','Start-Job','Invoke-Command','Export-Clixml'
)
$selfAst = [System.Management.Automation.Language.Parser]::ParseFile($PSCommandPath, [ref]$null, [ref]$null)
$badCalls = $selfAst.FindAll({ param($n) $n -is [System.Management.Automation.Language.CommandAst] }, $true) |
  Where-Object { $denied -contains $_.GetCommandName() }
if ($badCalls) {
  throw "Self-check failed: script contains write/delete cmdlets: $(($badCalls | ForEach-Object { $_.GetCommandName() }) -join ', ')"
}

$OutDir = [System.IO.Path]::GetFullPath($OutDir)
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$report = [ordered]@{ meta = [ordered]@{}; registry = @(); installs = @(); updaterLogs = @(); savedGames = @(); extras = [ordered]@{}; processes = @() }
$md = New-Object System.Collections.Generic.List[string]

function Save-Report([string]$Name, [string]$Text) {
  $target = [System.IO.Path]::GetFullPath((Join-Path $OutDir $Name))
  if (-not $target.StartsWith($OutDir + [System.IO.Path]::DirectorySeparatorChar, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing to write outside the report folder: $target"
  }
  [void][System.IO.Directory]::CreateDirectory($OutDir)
  [System.IO.File]::WriteAllText($target, $Text, (New-Object System.Text.UTF8Encoding($false)))
  return $target
}

# ---------------------------------------------------------------- helpers
$sensitive = '(?i)(key|token|pass|auth|serial|login|user|mail|session|secret|hash|id$)'

function Read-Shared([string]$Path) {
  # Read-only, shared: never locks a file DCS or the updater may be using.
  $fs = New-Object System.IO.FileStream($Path, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read,
    ([System.IO.FileShare]::ReadWrite -bor [System.IO.FileShare]::Delete))
  try {
    $sr = New-Object System.IO.StreamReader($fs, $true)
    return $sr.ReadToEnd()
  } finally { $fs.Dispose() }
}

function Get-FolderSize([string]$Path, [int]$MaxFiles = 400000) {
  # Own walker: skips junctions/symlinks so nothing is counted twice or followed elsewhere.
  $total = [int64]0; $files = 0; $stack = New-Object System.Collections.Stack
  $stack.Push($Path)
  while ($stack.Count -gt 0 -and $files -lt $MaxFiles) {
    $dir = $stack.Pop()
    try {
      foreach ($e in ([System.IO.DirectoryInfo]$dir).EnumerateFileSystemInfos()) {
        if (($e.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) { continue }
        if ($e -is [System.IO.DirectoryInfo]) { $stack.Push($e.FullName) }
        else { $total += $e.Length; $files++ }
      }
    } catch { }
  }
  [ordered]@{ bytes = $total; files = $files; truncated = ($files -ge $MaxFiles) }
}

function Format-Size([int64]$b) {
  if ($b -ge 1GB) { return '{0:N1} GB' -f ($b / 1GB) }
  if ($b -ge 1MB) { return '{0:N1} MB' -f ($b / 1MB) }
  if ($b -ge 1KB) { return '{0:N0} KB' -f ($b / 1KB) }
  return "$b B"
}

function Get-ExeInfo([string]$Path) {
  if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $null }
  $fi = Get-Item -LiteralPath $Path
  $vi = [System.Diagnostics.FileVersionInfo]::GetVersionInfo($Path)
  [ordered]@{ path = $Path; size = $fi.Length; modified = $fi.LastWriteTime.ToString('s'); fileVersion = $vi.FileVersion; productVersion = $vi.ProductVersion }
}

function Describe-Json($obj, [string]$prefix = '') {
  # Returns "path : type = value" lines; hides values whose key looks sensitive.
  $out = @()
  if ($obj -is [System.Management.Automation.PSCustomObject]) {
    foreach ($p in $obj.PSObject.Properties) {
      $out += Describe-Json $p.Value ($(if ($prefix) { "$prefix.$($p.Name)" } else { $p.Name }))
    }
  } elseif ($obj -is [System.Array]) {
    $types = ($obj | ForEach-Object { if ($null -eq $_) { 'null' } else { $_.GetType().Name } } | Select-Object -Unique) -join ','
    $out += "$prefix : array[$($obj.Count)] of $types"
    if ($obj.Count -gt 0 -and $obj[0] -is [System.Management.Automation.PSCustomObject]) { $out += Describe-Json $obj[0] "$prefix[0]" }
  } else {
    $t = if ($null -eq $obj) { 'null' } else { $obj.GetType().Name }
    $leaf = ($prefix -split '\.')[-1]
    $v = if ($leaf -match $sensitive) { '<hidden>' } else { "$obj" }
    if ($v.Length -gt 120) { $v = $v.Substring(0, 120) + '...' }
    $out += "$prefix : $t = $v"
  }
  return $out
}

function Analyze-Log([string]$Path, [string]$Kind) {
  $info = [ordered]@{ kind = $Kind; path = $Path; exists = $false }
  if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $info }
  $fi = Get-Item -LiteralPath $Path
  $text = Read-Shared $Path
  $lines = $text -split "`r?`n"
  $info.exists = $true
  $info.size = $fi.Length
  $info.modified = $fi.LastWriteTime.ToString('s')
  $info.lineCount = $lines.Count
  $info.head = @($lines | Select-Object -First $LogHeadLines)
  $info.tail = @($lines | Select-Object -Last $LogTailLines)
  $info.patterns = [ordered]@{
    percent      = @($lines | Where-Object { $_ -match '\d+(\.\d+)?\s*%' }).Count
    bytesOrSizes = @($lines | Where-Object { $_ -match '(?i)\b\d+(\.\d+)?\s*(bytes|kb|mb|gb|kib|mib|gib)\b' }).Count
    commandLines = @($lines | Where-Object { $_ -match '(?i)dcs_updater\.exe' } | Select-Object -Unique -First 20)
    exitOrResult = @($lines | Where-Object { $_ -match '(?i)(exit|return code|result|finished|complete|success|fail)' } | Select-Object -First 30)
    errors       = @($lines | Where-Object { $_ -match '(?i)\b(error|exception|fatal)\b' } | Select-Object -First 30)
    progressLike = @($lines | Where-Object { $_ -match '(?i)(progress|download|\d+\s*/\s*\d+)' } | Select-Object -First 30)
  }
  # Line "shapes": replace digits so we can see the distinct line formats.
  $info.lineShapes = @($lines | ForEach-Object { ($_ -replace '\d', '9') -replace '^(.{0,60}).*$', '$1' } |
    Group-Object | Sort-Object Count -Descending | Select-Object -First 25 | ForEach-Object { "$($_.Count)x  $($_.Name)" })
  return $info
}

# ---------------------------------------------------------------- meta
$report.meta = [ordered]@{
  probe = 'Hangar probe-readonly.ps1 v1'
  started = (Get-Date).ToString('s')
  psVersion = $PSVersionTable.PSVersion.ToString()
  os = [System.Environment]::OSVersion.VersionString
}
$md.Add("# Hangar probe - $stamp"); $md.Add('')
$md.Add('> Read-only probe. Nothing in DCS folders was changed. Contains local paths: do not publish.'); $md.Add('')

# ---------------------------------------------------------------- registry
$md.Add('## Registry'); $md.Add('')
$regRoots = @('HKCU:\Software\Eagle Dynamics', 'HKLM:\SOFTWARE\Eagle Dynamics', 'HKLM:\SOFTWARE\WOW6432Node\Eagle Dynamics')
$candidateRoots = New-Object System.Collections.Generic.List[string]
foreach ($root in $regRoots) {
  if (-not (Test-Path -LiteralPath $root)) { $md.Add("- ``$root``: not present"); continue }
  $keys = @(Get-Item -LiteralPath $root) + @(Get-ChildItem -LiteralPath $root -Recurse -ErrorAction SilentlyContinue)
  foreach ($k in $keys) {
    $vals = [ordered]@{}
    foreach ($name in $k.GetValueNames()) {
      $data = $k.GetValue($name)
      $show = ($name -notmatch $sensitive) -and (("$data" -match '^[A-Za-z]:\\') -or ($name -match '(?i)(path|version|branch|lang|dir|folder)'))
      $vals[$(if ($name) { $name } else { '(default)' })] = $(if ($show) { "$data" } else { "<hidden, $($k.GetValueKind($name))>" })
      if ($show -and "$data" -match '^[A-Za-z]:\\') { $candidateRoots.Add("$data".TrimEnd('\')) }
    }
    $report.registry += [ordered]@{ key = $k.Name; values = $vals }
    $md.Add("- ``$($k.Name)``")
    foreach ($e in $vals.GetEnumerator()) { $md.Add("  - $($e.Key) = ``$($e.Value)``") }
  }
}
$md.Add('')

# ---------------------------------------------------------------- installs
foreach ($d in @("$env:ProgramFiles\Eagle Dynamics\DCS World", "$env:ProgramFiles\Eagle Dynamics\DCS World OpenBeta", "$env:ProgramFiles\Eagle Dynamics\DCS World.OpenBeta")) {
  $candidateRoots.Add($d)
}
# Steam
$steam = $null
try { $steam = (Get-ItemProperty -LiteralPath 'HKCU:\Software\Valve\Steam' -ErrorAction Stop).SteamPath } catch { }
$steamInfo = [ordered]@{ steamPath = $steam; libraries = @(); dcsManifest = $null }
if ($steam -and (Test-Path -LiteralPath $steam)) {
  $libs = @($steam)
  $vdf = Join-Path $steam 'steamapps\libraryfolders.vdf'
  if (Test-Path -LiteralPath $vdf) {
    $libs += ([regex]::Matches((Read-Shared $vdf), '"path"\s+"([^"]+)"') | ForEach-Object { $_.Groups[1].Value -replace '\\\\', '\' })
  }
  $libs = $libs | Select-Object -Unique
  $steamInfo.libraries = @($libs)
  foreach ($lib in $libs) {
    $acf = Join-Path $lib 'steamapps\appmanifest_223750.acf'
    if (Test-Path -LiteralPath $acf) { $steamInfo.dcsManifest = $acf; $candidateRoots.Add((Join-Path $lib 'steamapps\common\DCSWorld')) }
  }
}
$report.extras.steam = $steamInfo

$md.Add('## Installations'); $md.Add('')
foreach ($root in ($candidateRoots | Select-Object -Unique)) {
  if (-not (Test-Path -LiteralPath $root -PathType Container)) { continue }
  if (-not ((Test-Path -LiteralPath (Join-Path $root 'bin')) -or (Test-Path -LiteralPath (Join-Path $root 'bin-mt')))) { continue }
  $inst = [ordered]@{ root = $root }
  $inst.topLevel = @(Get-ChildItem -LiteralPath $root -Force -ErrorAction SilentlyContinue | ForEach-Object { $(if ($_.PSIsContainer) { "$($_.Name)\" } else { $_.Name }) })
  $inst.dcsExe = [ordered]@{ bin = (Get-ExeInfo (Join-Path $root 'bin\DCS.exe')); binMt = (Get-ExeInfo (Join-Path $root 'bin-mt\DCS.exe')) }
  $inst.updaterExe = (Get-ExeInfo (Join-Path $root 'bin\DCS_updater.exe'))
  $cfgPath = Join-Path $root 'autoupdate.cfg'
  $inst.autoupdateCfg = [ordered]@{ path = $cfgPath; exists = (Test-Path -LiteralPath $cfgPath -PathType Leaf) }
  if ($inst.autoupdateCfg.exists) {
    $raw = Read-Shared $cfgPath
    $inst.autoupdateCfg.size = $raw.Length
    try {
      $json = $raw | ConvertFrom-Json
      $inst.autoupdateCfg.isJson = $true
      $inst.autoupdateCfg.structure = @(Describe-Json $json)
      if ($json.PSObject.Properties.Name -contains 'modules') { $inst.autoupdateCfg.modules = @($json.modules) }
    } catch {
      $inst.autoupdateCfg.isJson = $false
      $inst.autoupdateCfg.parseError = $_.Exception.Message
      $inst.autoupdateCfg.firstLines = @(($raw -split "`r?`n") | Select-Object -First 15)
    }
  }
  try { $drive = New-Object System.IO.DriveInfo(([System.IO.Path]::GetPathRoot($root))); $inst.drive = [ordered]@{ name = $drive.Name; free = $drive.AvailableFreeSpace; total = $drive.TotalSize; format = $drive.DriveFormat } } catch { }
  $report.installs += $inst
  $report.updaterLogs += (Analyze-Log (Join-Path $root 'autoupdate_log.txt') "autoupdate_log.txt ($root)")

  $md.Add("### ``$root``")
  $md.Add("- bin\DCS.exe: $(if ($inst.dcsExe.bin) { $inst.dcsExe.bin.fileVersion } else { 'missing' }) | bin-mt\DCS.exe: $(if ($inst.dcsExe.binMt) { $inst.dcsExe.binMt.fileVersion } else { 'missing' })")
  $md.Add("- DCS_updater.exe: $(if ($inst.updaterExe) { $inst.updaterExe.fileVersion } else { 'missing' })")
  if ($inst.Contains('drive')) { $md.Add("- Drive $($inst.drive.name): $(Format-Size $inst.drive.free) free of $(Format-Size $inst.drive.total)") }
  $md.Add("- autoupdate.cfg: exists=$($inst.autoupdateCfg.exists)$(if ($inst.autoupdateCfg.Contains('isJson')) { ", JSON=$($inst.autoupdateCfg.isJson)" })")
  if ($inst.autoupdateCfg.Contains('structure')) { $md.Add(''); $md.Add('```'); $inst.autoupdateCfg.structure | ForEach-Object { $md.Add($_) }; $md.Add('```') }
  if ($inst.autoupdateCfg.Contains('modules')) { $md.Add(''); $md.Add("Modules ($($inst.autoupdateCfg.modules.Count)): ``$($inst.autoupdateCfg.modules -join '``, ``')``") }
  $md.Add(''); $md.Add("Top level: $($inst.topLevel -join ', ')"); $md.Add('')
}
if ($report.installs.Count -eq 0) { $md.Add('No installation found in registry, default paths or Steam.'); $md.Add('') }

# ---------------------------------------------------------------- temp logs
foreach ($t in @(Get-ChildItem -LiteralPath $env:TEMP -Directory -Filter 'DCS*' -ErrorAction SilentlyContinue)) {
  $report.updaterLogs += (Analyze-Log (Join-Path $t.FullName 'autoupdate_templog.txt') "autoupdate_templog.txt ($($t.Name))")
}
$md.Add('## Updater logs'); $md.Add('')
foreach ($l in $report.updaterLogs) {
  if (-not $l.exists) { $md.Add("- $($l.kind): not found"); continue }
  $md.Add("### $($l.kind)")
  $md.Add("- $(Format-Size $l.size), $($l.lineCount) lines, modified $($l.modified)")
  $md.Add("- lines with %: $($l.patterns.percent) | lines with sizes: $($l.patterns.bytesOrSizes)")
  $md.Add(''); $md.Add('Command lines:'); $md.Add('```'); $l.patterns.commandLines | ForEach-Object { $md.Add($_) }; $md.Add('```')
  $md.Add('Result/exit-like lines:'); $md.Add('```'); $l.patterns.exitOrResult | ForEach-Object { $md.Add($_) }; $md.Add('```')
  $md.Add('Progress-like lines:'); $md.Add('```'); $l.patterns.progressLike | ForEach-Object { $md.Add($_) }; $md.Add('```')
  $md.Add('Line shapes (digits -> 9):'); $md.Add('```'); $l.lineShapes | ForEach-Object { $md.Add($_) }; $md.Add('```')
  $md.Add("Last $LogTailLines lines:"); $md.Add('```'); $l.tail | ForEach-Object { $md.Add($_) }; $md.Add('```'); $md.Add('')
}

# ---------------------------------------------------------------- saved games
$sgRoot = $null
try {
  $sgRoot = (Get-ItemProperty -LiteralPath 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders' -ErrorAction Stop).'{4C5C32FF-BB9D-43b0-B5B4-2D72E54EAAA4}'
  if ($sgRoot) { $sgRoot = [System.Environment]::ExpandEnvironmentVariables($sgRoot) }
} catch { }
if (-not $sgRoot) { $sgRoot = Join-Path $env:USERPROFILE 'Saved Games' }
$md.Add('## Saved Games'); $md.Add(''); $md.Add("Root: ``$sgRoot``"); $md.Add('')
foreach ($sg in @(Get-ChildItem -LiteralPath $sgRoot -Directory -Filter 'DCS*' -ErrorAction SilentlyContinue)) {
  $entry = [ordered]@{ path = $sg.FullName; isReparsePoint = (($sg.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0); folders = @() }
  foreach ($sub in @(Get-ChildItem -LiteralPath $sg.FullName -Force -ErrorAction SilentlyContinue)) {
    $isLink = (($sub.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0)
    if ($sub.PSIsContainer) {
      $size = if ($isLink) { $null } else { Get-FolderSize $sub.FullName }
      $children = @()
      if ($sub.Name -in @('Mods', 'Liveries', 'Kneeboard', 'Scripts')) {
        $children = @(Get-ChildItem -LiteralPath $sub.FullName -Force -ErrorAction SilentlyContinue | ForEach-Object {
          if ($_.PSIsContainer) {
            $grand = @(Get-ChildItem -LiteralPath $_.FullName -Directory -Force -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Name)
            "$($_.Name)\ [$($grand -join ', ')]"
          } else { $_.Name }
        })
      }
      $entry.folders += [ordered]@{ name = "$($sub.Name)\"; link = $isLink; size = $size; children = $children }
    } else {
      $entry.folders += [ordered]@{ name = $sub.Name; link = $isLink; size = [ordered]@{ bytes = $sub.Length; files = 1 }; children = @() }
    }
  }
  $dcsLog = Join-Path $sg.FullName 'Logs\dcs.log'
  $entry.dcsLog = Analyze-Log $dcsLog 'dcs.log'
  $report.savedGames += $entry

  $md.Add("### ``$($sg.Name)``$(if ($entry.isReparsePoint) { ' (link)' })")
  foreach ($f in $entry.folders) {
    $s = if ($f.link) { 'link, not followed' } elseif ($f.size) { "$(Format-Size $f.size.bytes), $($f.size.files) files" } else { '' }
    $md.Add("- $($f.name) - $s")
    foreach ($c in $f.children) { $md.Add("  - $c") }
  }
  if ($entry.dcsLog.exists) {
    $md.Add(''); $md.Add("dcs.log: $(Format-Size $entry.dcsLog.size), $($entry.dcsLog.lineCount) lines, errors (first 30 shown): $($entry.dcsLog.patterns.errors.Count)")
    $md.Add('```'); $entry.dcsLog.lineShapes | ForEach-Object { $md.Add($_) }; $md.Add('```')
  }
  $md.Add('')
}

# ---------------------------------------------------------------- extras
$docs = [System.Environment]::GetFolderPath('MyDocuments')
$tac = Join-Path $docs 'Tacview'
$report.extras.tacview = [ordered]@{ path = $tac; exists = (Test-Path -LiteralPath $tac -PathType Container) }
if ($report.extras.tacview.exists) {
  $report.extras.tacview.size = Get-FolderSize $tac
  $report.extras.tacview.acmiCount = @(Get-ChildItem -LiteralPath $tac -Recurse -Filter '*.acmi' -File -ErrorAction SilentlyContinue).Count
}
$videos = [System.Environment]::GetFolderPath('MyVideos')
$report.extras.videos = [ordered]@{ path = $videos; folders = @() }
foreach ($v in @(Get-ChildItem -LiteralPath $videos -Directory -ErrorAction SilentlyContinue)) {
  $imgs = @(Get-ChildItem -LiteralPath $v.FullName -File -Recurse -ErrorAction SilentlyContinue | Where-Object { $_.Extension -match '^\.(png|jpe?g|bmp|webp|jxr)$' })
  if ($imgs.Count -gt 0) {
    $report.extras.videos.folders += [ordered]@{ name = $v.Name; images = $imgs.Count; byExt = @($imgs | Group-Object Extension | ForEach-Object { "$($_.Name)=$($_.Count)" }) }
  }
}
$report.processes = @(Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.ProcessName -match '^(DCS|DCS_updater)$' } | ForEach-Object { $_.ProcessName })

$md.Add('## Other folders'); $md.Add('')
$md.Add("- Tacview ``$tac``: $(if ($report.extras.tacview.exists) { "$(Format-Size $report.extras.tacview.size.bytes), $($report.extras.tacview.acmiCount) .acmi" } else { 'not found' })")
$md.Add("- Videos ``$videos``: image folders: $(if ($report.extras.videos.folders.Count) { ($report.extras.videos.folders | ForEach-Object { "$($_.name) ($($_.images): $($_.byExt -join ' '))" }) -join '; ' } else { 'none' })")
$md.Add("- Steam: $(if ($steamInfo.steamPath) { "found; DCS manifest: $(if ($steamInfo.dcsManifest) { 'yes' } else { 'no' })" } else { 'not found' })")
$md.Add("- Running now: $(if ($report.processes.Count) { $report.processes -join ', ' } else { 'no DCS processes' })")

# ---------------------------------------------------------------- save
$report.meta.finished = (Get-Date).ToString('s')
$mdPath = Save-Report "probe-$stamp.md" ($md -join "`r`n")
$jsonPath = Save-Report "probe-$stamp.json" ($report | ConvertTo-Json -Depth 10)
Write-Host "Done. Nothing in DCS folders was changed."
Write-Host "Report: $mdPath"
Write-Host "Data:   $jsonPath"
