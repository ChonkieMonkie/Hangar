<#
.SYNOPSIS
  Hangar - read-only watcher for a DCS update that YOU start (level 2 in docs/INVESTIGATION.md).

.DESCRIPTION
  Start this script, then start the DCS update the way you normally do (from DCS or with
  DCS_updater.exe). The script waits for DCS_updater.exe, observes it until it exits and writes
  a report. It answers:
    - Does _downloads grow toward the "To download" size (unpacked)?  -> exact progress for the ETA
    - Network (compressed) vs unpacked speed, and their ratio
    - Phase timings from autoupdate_log.txt STATUS lines
    - Exit codes of the updater processes, and whether they run elevated (UAC)
    - Can the updater window be read with Windows APIs?
        * Win32: EnumWindows + GetWindowText + GetClassName (title and class)
        * UI Automation: control tree, names, progress-bar values (what screen readers use)

  SAFETY (see CLAUDE.md, most important rule)
    - Only reads. Never starts, stops, clicks, types into or sends input to any program.
    - UI Automation is used for reading only (Name, Value, RangeValue). No Invoke/Toggle/SetValue.
    - Opens process handles with PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE only.
    - Files are opened read-only with shared access; nothing in DCS folders is changed.
    - The only writes go to the report folder (-OutDir). The script parses itself at start and
      aborts if it contains a write/delete/process-starting cmdlet.

  Output (in ..\investigation by default):
    watch-<stamp>.ndjson      one JSON sample per tick
    watch-<stamp>-uia.txt     UI Automation dumps (only when the window content changes)
    watch-<stamp>-summary.md  summary written when the updater has exited (or on Ctrl+C)

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File tools\watch-updater.ps1
  powershell -ExecutionPolicy Bypass -File tools\watch-updater.ps1 -SelfTest

.NOTES
  Windows PowerShell 5.1 compatible. Keep this file ASCII-only. Not affiliated with Eagle Dynamics SA.
#>
[CmdletBinding()]
param(
  [string]$Root = '',
  [int]$IntervalMs = 1000,
  [int]$WaitMinutes = 180,
  [int]$MaxMinutes = 360,
  [int]$UiaEverySeconds = 2,
  [string]$OutDir = '',
  [switch]$NoUia,
  [switch]$SelfTest,
  # Test only: watch another (harmless) process name instead of DCS_updater.exe, and stop after N seconds.
  [string]$ProcessName = 'DCS_updater.exe',
  [int]$MaxSeconds = 0
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2
if (-not $OutDir) { $OutDir = Join-Path (Split-Path -Parent $PSCommandPath) '..\investigation' }
$OutDir = [System.IO.Path]::GetFullPath($OutDir)

# ---------------------------------------------------------------- self-check
$denied = @(
  'Remove-Item','Set-Content','Add-Content','Out-File','New-Item','Move-Item','Copy-Item',
  'Rename-Item','Clear-Content','Clear-Item','Set-Item','Set-ItemProperty','New-ItemProperty',
  'Remove-ItemProperty','Rename-ItemProperty','Copy-ItemProperty','Move-ItemProperty',
  'Start-Process','Invoke-Item','Stop-Process','Invoke-Expression','Export-Csv','Tee-Object',
  'Set-Acl','Start-Job','Invoke-Command','Export-Clixml','Invoke-CimMethod','Invoke-WmiMethod'
)
$selfAst = [System.Management.Automation.Language.Parser]::ParseFile($PSCommandPath, [ref]$null, [ref]$null)
$badCalls = $selfAst.FindAll({ param($n) $n -is [System.Management.Automation.Language.CommandAst] }, $true) |
  Where-Object { $denied -contains $_.GetCommandName() }
if ($badCalls) { throw "Self-check failed: $(($badCalls | ForEach-Object { $_.GetCommandName() }) -join ', ')" }

function Write-Out([string]$Name, [string]$Text, [switch]$Append) {
  $target = [System.IO.Path]::GetFullPath((Join-Path $OutDir $Name))
  if (-not $target.StartsWith($OutDir + [System.IO.Path]::DirectorySeparatorChar, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing to write outside the report folder: $target"
  }
  [void][System.IO.Directory]::CreateDirectory($OutDir)
  $enc = New-Object System.Text.UTF8Encoding($false)
  if ($Append) { [System.IO.File]::AppendAllText($target, $Text, $enc) } else { [System.IO.File]::WriteAllText($target, $Text, $enc) }
}

# ---------------------------------------------------------------- Win32 (read-only calls)
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class HangarWin {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr l);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("kernel32.dll", SetLastError = true)] static extern IntPtr OpenProcess(uint access, bool inherit, uint pid);
  [DllImport("kernel32.dll")] public static extern bool CloseHandle(IntPtr h);
  [DllImport("kernel32.dll")] static extern bool GetExitCodeProcess(IntPtr h, out uint code);
  [DllImport("advapi32.dll", SetLastError = true)] static extern bool OpenProcessToken(IntPtr p, uint access, out IntPtr tok);
  [DllImport("advapi32.dll", SetLastError = true)] static extern bool GetTokenInformation(IntPtr t, int cls, out int info, int len, out int ret);

  // Returns [pid, hwnd, visible, class, title] for top-level windows owned by the given pids.
  public static List<string[]> Windows(uint[] pids) {
    var set = new HashSet<uint>(pids);
    var res = new List<string[]>();
    EnumProc cb = (h, l) => {
      uint pid; GetWindowThreadProcessId(h, out pid);
      if (set.Contains(pid)) {
        var t = new StringBuilder(512); GetWindowText(h, t, 512);
        var c = new StringBuilder(256); GetClassName(h, c, 256);
        res.Add(new string[] { pid.ToString(), h.ToInt64().ToString(), IsWindowVisible(h) ? "1" : "0", c.ToString(), t.ToString() });
      }
      return true;
    };
    EnumWindows(cb, IntPtr.Zero);
    GC.KeepAlive(cb);
    return res;
  }
  // PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE: enough to read exit code later, nothing more.
  public static IntPtr OpenForQuery(uint pid) { return OpenProcess(0x1000 | 0x00100000, false, pid); }
  public static int LastError() { return Marshal.GetLastWin32Error(); }
  // 259 = STILL_ACTIVE, -2 = no handle, -3 = call failed
  public static long ExitCode(IntPtr h) { uint c; if (h == IntPtr.Zero) return -2; if (!GetExitCodeProcess(h, out c)) return -3; return (long)c; }
  public static string Elevation(IntPtr h) {
    IntPtr tok; if (h == IntPtr.Zero) return "no-handle";
    if (!OpenProcessToken(h, 8, out tok)) return "token-denied(" + Marshal.GetLastWin32Error() + ")";
    int e, r; bool ok = GetTokenInformation(tok, 20, out e, 4, out r); CloseHandle(tok);
    return ok ? (e != 0 ? "elevated" : "not-elevated") : "unknown";
  }
}
'@

$uiaOk = $false
if (-not $NoUia) {
  try { Add-Type -AssemblyName UIAutomationClient; Add-Type -AssemblyName UIAutomationTypes; $uiaOk = $true } catch { $uiaOk = $false }
}

# ---------------------------------------------------------------- helpers
function Get-FolderSize([string]$Path) {
  $total = [int64]0; $files = 0
  if (-not (Test-Path -LiteralPath $Path -PathType Container)) { return @{ bytes = [int64]0; files = 0; exists = $false } }
  $stack = New-Object System.Collections.Stack; $stack.Push($Path)
  while ($stack.Count -gt 0) {
    $dir = $stack.Pop()
    try {
      foreach ($e in ([System.IO.DirectoryInfo]$dir).EnumerateFileSystemInfos()) {
        if (($e.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) { continue }
        if ($e -is [System.IO.DirectoryInfo]) { $stack.Push($e.FullName) } else { $total += $e.Length; $files++ }
      }
    } catch { }
  }
  @{ bytes = $total; files = $files; exists = $true }
}

function Get-NetRx {
  $sum = [int64]0
  foreach ($n in [System.Net.NetworkInformation.NetworkInterface]::GetAllNetworkInterfaces()) {
    if ($n.OperationalStatus -ne 'Up') { continue }
    if ($n.NetworkInterfaceType -eq 'Loopback' -or $n.NetworkInterfaceType -eq 'Tunnel') { continue }
    try { $sum += $n.GetIPStatistics().BytesReceived } catch { }
  }
  $sum
}

# Tail reader: returns new complete lines since last call; handles rotation (file replaced/shrunk).
# Files that exist when the watcher starts are read from their end; files that appear later from the start.
$tails = @{}
function Start-Tail([string]$Path) {
  if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return }
  $fi = Get-Item -LiteralPath $Path
  $tails[$Path.ToLowerInvariant()] = @{ offset = [int64]$fi.Length; created = $fi.CreationTimeUtc; rest = '' }
}
function Read-NewLines([string]$Path) {
  if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return @() }
  $fi = Get-Item -LiteralPath $Path
  $key = $Path.ToLowerInvariant()
  if (-not $tails.ContainsKey($key)) { $tails[$key] = @{ offset = [int64]0; created = $fi.CreationTimeUtc; rest = '' } }
  $st = $tails[$key]
  if ($fi.Length -lt $st.offset -or $fi.CreationTimeUtc -ne $st.created) { $st.offset = [int64]0; $st.created = $fi.CreationTimeUtc; $st.rest = '' }
  if ($fi.Length -eq $st.offset) { return @() }
  $fs = New-Object System.IO.FileStream($Path, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read,
    ([System.IO.FileShare]::ReadWrite -bor [System.IO.FileShare]::Delete))
  try {
    [void]$fs.Seek($st.offset, [System.IO.SeekOrigin]::Begin)
    $sr = New-Object System.IO.StreamReader($fs)
    $chunk = $sr.ReadToEnd()
    $st.offset = $fs.Position
  } finally { $fs.Dispose() }
  $text = $st.rest + $chunk
  $parts = $text -split "`r?`n"
  $st.rest = $parts[-1]
  if ($parts.Count -le 1) { return @() }
  return @($parts[0..($parts.Count - 2)])
}

function Get-UiaDump([long]$hwnd) {
  $lines = New-Object System.Collections.Generic.List[string]
  try {
    $el = [System.Windows.Automation.AutomationElement]::FromHandle([IntPtr]$hwnd)
    $all = $el.FindAll([System.Windows.Automation.TreeScope]::Subtree, [System.Windows.Automation.Condition]::TrueCondition)
    $i = 0
    foreach ($e in $all) {
      if ($i++ -ge 400) { $lines.Add('... (truncated at 400 elements)'); break }
      $c = $e.Current
      $line = '{0} | name="{1}" | id={2} | class={3}' -f ($c.ControlType.ProgrammaticName -replace '^ControlType\.', ''), $c.Name, $c.AutomationId, $c.ClassName
      $o = $null
      if ($e.TryGetCurrentPattern([System.Windows.Automation.RangeValuePattern]::Pattern, [ref]$o)) {
        $line += ' | range={0}/{1}..{2}' -f $o.Current.Value, $o.Current.Minimum, $o.Current.Maximum
      }
      $o = $null
      if (-not $c.IsPassword -and $e.TryGetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern, [ref]$o)) {
        $line += ' | value="{0}"' -f $o.Current.Value
      }
      $lines.Add($line)
    }
    if ($lines.Count -eq 0) { $lines.Add('(no elements exposed)') }
  } catch {
    $lines.Add("(UIA error: $($_.Exception.GetType().Name): $($_.Exception.Message))")
  }
  return ,$lines
}

function Find-Root {
  if ($Root) { return $Root }
  foreach ($k in @('HKCU:\Software\Eagle Dynamics\DCS World', 'HKCU:\Software\Eagle Dynamics\DCS World OpenBeta')) {
    try { $p = (Get-ItemProperty -LiteralPath $k -ErrorAction Stop).Path; if ($p) { return $p.TrimEnd('\') } } catch { }
  }
  throw 'DCS install not found in registry. Pass -Root "X:\Path\DCS World".'
}

function Get-UpdaterProcs {
  @(Get-CimInstance -ClassName Win32_Process -Filter "Name='$ProcessName'" -ErrorAction SilentlyContinue)
}

# ---------------------------------------------------------------- setup
$Root = Find-Root
$logMain = Join-Path $Root 'autoupdate_log.txt'
$downloads = Join-Path $Root '_downloads'
$drive = New-Object System.IO.DriveInfo(([System.IO.Path]::GetPathRoot($Root)))
$tempLogs = @(Get-ChildItem -LiteralPath $env:TEMP -Directory -Filter 'DCS*' -ErrorAction SilentlyContinue | ForEach-Object { Join-Path $_.FullName 'autoupdate_templog.txt' })

if ($SelfTest) {
  Write-Host "Self-test (nothing is written):"
  Write-Host "  root:            $Root"
  Write-Host "  log exists:      $(Test-Path -LiteralPath $logMain)"
  Start-Tail $logMain; [void](Read-NewLines $logMain)
  Write-Host "  tail reader:     ok (positioned at end of file, no new lines)"
  $d = Get-FolderSize $downloads
  Write-Host "  _downloads:      exists=$($d.exists) bytes=$($d.bytes) files=$($d.files)"
  Write-Host "  net rx total:    $(Get-NetRx)"
  Write-Host "  free on drive:   $([math]::Round($drive.AvailableFreeSpace / 1GB, 1)) GB"
  Write-Host "  updater running: $(@(Get-UpdaterProcs).Count)"
  $self = [uint32]$PID
  $h = [HangarWin]::OpenForQuery($self)
  Write-Host "  win32 handle:    $(if ($h -ne [IntPtr]::Zero) { 'ok' } else { 'failed' }), elevation of this shell: $([HangarWin]::Elevation($h)), exit code (259=running): $([HangarWin]::ExitCode($h))"
  if ($h -ne [IntPtr]::Zero) { [void][HangarWin]::CloseHandle($h) }
  $ex = @(Get-Process -Name explorer -ErrorAction SilentlyContinue | ForEach-Object { [uint32]$_.Id })
  $w = if ($ex.Count) { [HangarWin]::Windows([uint32[]]$ex) } else { @() }
  Write-Host "  EnumWindows:     $(@($w).Count) top-level windows owned by explorer (titles not shown)"
  if ($uiaOk) {
    $tray = @($w | Where-Object { $_[3] -eq 'Shell_TrayWnd' } | Select-Object -First 1)
    if ($tray.Count) { $dump = Get-UiaDump ([long]$tray[0][1]); Write-Host "  UI Automation:   ok, taskbar exposes $($dump.Count) elements (names not shown)" }
    else { Write-Host "  UI Automation:   loaded, no taskbar window to test" }
  } else { Write-Host "  UI Automation:   not available" }
  return
}

$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$ndjson = "watch-$stamp.ndjson"; $uiaFile = "watch-$stamp-uia.txt"; $sumFile = "watch-$stamp-summary.md"

# ---------------------------------------------------------------- wait for updater
Write-Host "Hangar watcher (read-only). Install: $Root"
Write-Host "Start the DCS update now. Waiting up to $WaitMinutes min for $ProcessName ... (Ctrl+C to stop)"
$loopError = $null
foreach ($p in @($logMain) + $tempLogs) { Start-Tail $p }   # existing logs: start at their current end
$waitUntil = (Get-Date).AddMinutes($WaitMinutes)
while (@(Get-UpdaterProcs).Count -eq 0) {
  if ((Get-Date) -gt $waitUntil) { Write-Host 'No updater seen. Nothing written.'; return }
  # A new log may appear before the process is visible to us; keep tail state fresh.
  Start-Sleep -Milliseconds 500
}

# ---------------------------------------------------------------- observe
$t0 = Get-Date
$samples = New-Object System.Collections.Generic.List[object]
$events = New-Object System.Collections.Generic.List[object]
$handles = @{}          # pid -> handle (kept open to read exit code after exit)
$procMeta = @{}         # pid -> info
$windowsSeen = @{}      # "pid|class|title" -> first t
$lastUiaSig = @{}; $lastUia = Get-Date '2000-01-01'
$uiaSummary = New-Object System.Collections.Generic.List[string]
$toDownload = $null; $toFiles = $null; $phase = ''
$goneSince = $null; $lastPrint = Get-Date '2000-01-01'
$interesting = '(?i)(STATUS:|To download:|Installed \d+ files|Successfully|Not enough|cmdline:|=== Log|ERROR|error|fail|deltas to download|packs to download)'

try {
  while ($true) {
    $now = Get-Date; $t = [math]::Round(($now - $t0).TotalSeconds, 2)
    $procs = @(Get-UpdaterProcs)   # @() so .Count works with 0 or 1 process under StrictMode

    foreach ($p in $procs) {
      $id = [uint32]$p.ProcessId
      if (-not $handles.ContainsKey($id)) {
        $h = [HangarWin]::OpenForQuery($id)
        $handles[$id] = $h
        $procMeta[$id] = [ordered]@{
          pid = $id; ppid = $p.ParentProcessId; firstSeen = $t; lastSeen = $t
          commandLine = $(if ($p.CommandLine) { $p.CommandLine } else { '(not readable)' })
          handle = $(if ($h -ne [IntPtr]::Zero) { 'ok' } else { "failed($([HangarWin]::LastError()))" })
          elevation = [HangarWin]::Elevation($h); exitCode = $null
        }
        $events.Add([ordered]@{ t = $t; kind = 'process-start'; text = "pid $id ppid $($p.ParentProcessId): $($procMeta[$id].commandLine) [$($procMeta[$id].elevation)]" })
      } else { $procMeta[$id].lastSeen = $t }
    }
    # exit codes for processes that are gone
    foreach ($id in @($handles.Keys)) {
      if ($null -eq $procMeta[$id].exitCode -and -not ($procs | Where-Object { [uint32]$_.ProcessId -eq $id })) {
        $code = [HangarWin]::ExitCode($handles[$id])
        $procMeta[$id].exitCode = $code
        $events.Add([ordered]@{ t = $t; kind = 'process-exit'; text = "pid $id exit code $code" })
      }
    }

    # log lines
    $newLines = @()
    foreach ($p in @($logMain) + $tempLogs) { foreach ($l in (Read-NewLines $p)) { $newLines += ,@((Split-Path -Leaf $p), $l) } }
    foreach ($nl in $newLines) {
      $l = $nl[1]
      if ($l -match 'To download:\s*(\d+)\s*bytes in\s*(\d+)\s*files') { $toDownload = [int64]$Matches[1]; $toFiles = [int]$Matches[2] }
      if ($l -match 'STATUS:\s*(.+)$') { $phase = $Matches[1].Trim() }
      if ($l -match $interesting) { $events.Add([ordered]@{ t = $t; kind = $nl[0]; text = $l }) }
    }

    # windows (Win32) + UI Automation
    $pids = @($procs | ForEach-Object { [uint32]$_.ProcessId })
    $wins = @()
    if ($pids.Count) { $wins = @([HangarWin]::Windows([uint32[]]$pids)) }
    foreach ($w in $wins) {
      $k = "$($w[0])|$($w[3])|$($w[4])"
      if (-not $windowsSeen.ContainsKey($k)) { $windowsSeen[$k] = $t; $events.Add([ordered]@{ t = $t; kind = 'window'; text = "pid $($w[0]) visible=$($w[2]) class=$($w[3]) title=`"$($w[4])`"" }) }
    }
    if ($uiaOk -and (($now - $lastUia).TotalSeconds -ge $UiaEverySeconds)) {
      $lastUia = $now
      foreach ($w in $wins) {
        if ($w[2] -ne '1') { continue }   # visible windows only (explicit loop: a pipeline would unroll a single string[])
        $dump = Get-UiaDump ([long]$w[1])
        $sig = ($dump -join "`n")
        if (-not $lastUiaSig.ContainsKey($w[1]) -or $lastUiaSig[$w[1]] -ne $sig) {
          $lastUiaSig[$w[1]] = $sig
          Write-Out $uiaFile ("--- t=$t pid=$($w[0]) hwnd=$($w[1]) class=$($w[3]) title=`"$($w[4])`"`r`n" + ($dump -join "`r`n") + "`r`n") -Append
          if ($uiaSummary.Count -lt 40) { $uiaSummary.Add("t=$t pid=$($w[0]): $($dump.Count) elements; first: $(@($dump)[0])") }
        }
      }
    }

    # measurements
    $dl = Get-FolderSize $downloads   # DriveInfo.AvailableFreeSpace is queried live on each access
    $sample = [pscustomobject][ordered]@{
      t = $t; utc = $now.ToUniversalTime().ToString('o'); phase = $phase
      netRx = (Get-NetRx); dlBytes = $dl.bytes; dlFiles = $dl.files; free = $drive.AvailableFreeSpace
      toDownload = $toDownload
      procs = @($procs | ForEach-Object { [ordered]@{ pid = $_.ProcessId; w = $_.WriteTransferCount; r = $_.ReadTransferCount } })
    }
    $samples.Add($sample)
    Write-Out $ndjson (($sample | ConvertTo-Json -Compress -Depth 5) + "`n") -Append

    if (($now - $lastPrint).TotalSeconds -ge 10) {
      $lastPrint = $now
      $n = $samples.Count; $prev = $samples[[math]::Max(0, $n - 11)]; $dt = [math]::Max(0.5, $t - $prev.t)
      $netMB = ($sample.netRx - $prev.netRx) / $dt / 1MB; $dlMB = ($sample.dlBytes - $prev.dlBytes) / $dt / 1MB
      $tot = if ($toDownload) { ' / {0:N2} GB' -f ($toDownload / 1GB) } else { '' }
      Write-Host ('[{0,7:N0}s] {1,-28} _downloads {2:N2} GB{3} | net {4,6:N1} MB/s | unpacked {5,6:N1} MB/s' -f $t, $phase, ($dl.bytes / 1GB), $tot, $netMB, $dlMB)
    }

    # stop when all updater processes are gone for 15 s, or after MaxMinutes
    if ($procs.Count -eq 0) { if (-not $goneSince) { $goneSince = $now } elseif (($now - $goneSince).TotalSeconds -ge 15) { break } } else { $goneSince = $null }
    if (($now - $t0).TotalMinutes -ge $MaxMinutes) { $events.Add([ordered]@{ t = $t; kind = 'watcher'; text = 'MaxMinutes reached' }); break }
    if ($MaxSeconds -gt 0 -and ($now - $t0).TotalSeconds -ge $MaxSeconds) { $events.Add([ordered]@{ t = $t; kind = 'watcher'; text = 'MaxSeconds reached (test)' }); break }
    Start-Sleep -Milliseconds $IntervalMs
  }
}
catch {
  $loopError = "$($_.Exception.Message) (line $($_.InvocationInfo.ScriptLineNumber))"
  Write-Warning "Watcher error: $loopError"
  $events.Add([ordered]@{ t = -1; kind = 'watcher-error'; text = $loopError })
}
finally {
  # final exit codes and handle cleanup
  foreach ($id in @($handles.Keys)) {
    if ($null -eq $procMeta[$id].exitCode) { $procMeta[$id].exitCode = [HangarWin]::ExitCode($handles[$id]) }
    if ($handles[$id] -ne [IntPtr]::Zero) { [void][HangarWin]::CloseHandle($handles[$id]) }
  }

  # ------------------------------------------------------------ summary
  $md = New-Object System.Collections.Generic.List[string]
  $md.Add("# Hangar watcher - $stamp"); $md.Add('')
  $md.Add('> Read-only observation. Nothing in DCS folders was changed. Contains local paths: do not publish.'); $md.Add('')
  $md.Add("- Install: ``$Root``"); $md.Add("- Duration observed: $([math]::Round(((Get-Date) - $t0).TotalMinutes, 1)) min, $($samples.Count) samples")
  $md.Add("- To download (log): $(if ($toDownload) { '{0:N0} bytes ({1:N2} GB) in {2} files' -f $toDownload, ($toDownload / 1GB), $toFiles } else { 'not seen' })")
  if ($loopError) { $md.Add("- **Watcher stopped on an error:** $loopError") }
  if ($samples.Count -gt 1) { try {
    $peak = ($samples | Measure-Object -Property dlBytes -Maximum).Maximum
    $pctText = if ($toDownload) { ' = {0:P1} of To download' -f ($peak / $toDownload) } else { '' }
    $md.Add(('- Peak size of _downloads: {0:N0} bytes ({1:N2} GB)' -f $peak, ($peak / 1GB)) + $pctText)
    # Download phase: any "Downloading ..." STATUS except the short index/version-info steps
    # (works for updates and for module installs).
    $dlSamples = @($samples | Where-Object { $_.phase -match '^Downloading' -and $_.phase -notmatch '(?i)version info|index' })
    if ($dlSamples.Count -gt 5) {
      $a = $dlSamples[0]; $b = $dlSamples[-1]; $dt = [math]::Max(1, $b.t - $a.t)
      $net = ($b.netRx - $a.netRx) / $dt; $unp = ($b.dlBytes - $a.dlBytes) / $dt
      $md.Add(("- Download phase: {0:N0} s | network {1:N1} MB/s (compressed, whole PC) | unpacked {2:N1} MB/s | ratio unpacked/network {3:N2}" -f $dt, ($net / 1MB), ($unp / 1MB), $(if ($net -gt 0) { $unp / $net } else { 0 })))
      $mid = $dlSamples[[int]($dlSamples.Count / 2)]
      if ($toDownload -and $unp -gt 0) {
        $etaMid = ($toDownload - $mid.dlBytes) / $unp
        $md.Add(("- ETA check: at t={0:N0}s the formula (To download - _downloads) / unpacked rate gave {1:N0} s; the download phase actually ended {2:N0} s later" -f $mid.t, $etaMid, ($b.t - $mid.t)))
      }
    } else { $md.Add('- Download phase: too few samples in a "Downloading ..." phase') }
    $w0 = $samples[0]; $w1 = $samples[-1]
    $md.Add(('- Free space: {0:N1} GB at start, {1:N1} GB at end' -f ($w0.free / 1GB), ($w1.free / 1GB)))
  } catch { $md.Add("- (statistics failed: $($_.Exception.Message), line $($_.InvocationInfo.ScriptLineNumber)). Raw data is in $ndjson") } }
  $md.Add(''); $md.Add('## Processes'); $md.Add('')
  foreach ($m in $procMeta.Values) {
    $md.Add("- pid $($m.pid) (parent $($m.ppid)), seen $($m.firstSeen)-$($m.lastSeen) s, handle $($m.handle), $($m.elevation), exit code $($m.exitCode)")
    $md.Add("  - ``$($m.commandLine)``")
  }
  $md.Add(''); $md.Add('## Windows (Win32 GetWindowText / GetClassName)'); $md.Add('')
  if ($windowsSeen.Count) { foreach ($k in $windowsSeen.Keys) { $md.Add("- t=$($windowsSeen[$k]) s: ``$k``") } } else { $md.Add('- no top-level windows found for updater processes') }
  $md.Add(''); $md.Add('## UI Automation'); $md.Add('')
  if (-not $uiaOk) { $md.Add('- not used') }
  elseif ($uiaSummary.Count) { $uiaSummary | ForEach-Object { $md.Add("- $_") }; $md.Add(''); $md.Add("Full dumps: ``$uiaFile``") }
  else { $md.Add('- no visible updater window was dumped') }
  $md.Add(''); $md.Add('## Events (log lines and process changes)'); $md.Add(''); $md.Add('```')
  foreach ($e in $events) { $md.Add(('{0,9:N1}s {1,-24} {2}' -f $e.t, $e.kind, $e.text)) }
  $md.Add('```')
  if ($samples.Count) { Write-Out $sumFile ($md -join "`r`n") }
  Write-Host "Done. Nothing in DCS folders was changed."
  if ($samples.Count) { Write-Host "Summary: $(Join-Path $OutDir $sumFile)" }
}
