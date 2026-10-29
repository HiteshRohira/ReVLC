# Runs the Cua suite (scripts/qa/ci/cua_suite.py) on a fresh Windows in
# Windows Sandbox: a clean copy of this PC's Windows each time, nothing to
# maintain, gone when it closes.
#
#   scripts\qa\sandbox\run-sandbox.ps1 -Installer ReVLC-0.2.11-alpha-x64-setup.exe
#   scripts\qa\sandbox\run-sandbox.ps1 -Installer ... -Only playback,effects   (or -Quick)
#   scripts\qa\sandbox\run-sandbox.ps1 -Installer ... -NoGpu
#   scripts\qa\sandbox\run-sandbox.ps1 -Installer ... -Previous ReVLC-0.2.10-x64-setup.exe
#
# -NoGpu turns off the Sandbox's virtual GPU: a PC with no GPU decoding,
# where the Windows preview and video-output fallbacks get used. -Previous
# first installs that older release, uses it, and installs -Installer over
# it, checking settings and history survive the update
# (scripts\qa\upgrade_check.py); then the suite runs as usual.
#
# -Root (default %USERPROFILE%\revlc-sandbox, or $env:REVLC_SANDBOX_ROOT)
# holds the tools, the generated test media (kept between runs) and each
# run's folder while it runs.
#
# The first run prepares the tools the Sandbox needs in -Root\tools: Python
# (python.org's NuGet package, no installer) with Pillow, and a copy of this
# PC's Cua Driver. ffmpeg comes with ReVLC. Later runs download nothing.
#
# Then it starts a Sandbox with the tools (read-only) and a run folder
# (installer + this repo's scripts\qa) mapped in. in-sandbox.ps1 installs
# ReVLC with setup.exe /S and runs the suite through run-windows.ps1. The
# Sandbox shows as a window; leave it alone while it runs. Results land in
# %USERPROFILE%\revlc-qa\sandbox-<timestamp>; the exit code is the suite's.
#
# Needs: Windows Sandbox (Windows Features > Windows Sandbox, then a reboot),
# Python 3.13 with pip on this PC (to fetch Pillow once), Cua Driver
# installed here (its binaries are copied), no other Sandbox open, and for
# -Previous, ffmpeg on PATH.
param(
    [Parameter(Mandatory = $true)][string]$Installer,
    [string]$Only,
    [switch]$Quick,
    [switch]$NoGpu,
    [string]$Previous,
    [string]$Root = $(if ($env:REVLC_SANDBOX_ROOT) { $env:REVLC_SANDBOX_ROOT } else { Join-Path $env:USERPROFILE "revlc-sandbox" }),
    [int]$TimeoutMinutes = 150,
    [int]$StallMinutes = 15,
    [string]$PythonVersion = "3.13.15"
)
$ErrorActionPreference = "Stop"
$qa = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$Installer = (Resolve-Path $Installer).Path
if ([IO.Path]::GetExtension($Installer) -ne ".exe") { throw "Pass the setup .exe" }
if ($Previous) { $Previous = (Resolve-Path $Previous).Path }
$sandboxExe = Join-Path $env:SystemRoot "System32\WindowsSandbox.exe"
if (-not (Test-Path $sandboxExe)) { throw "Windows Sandbox isn't enabled: Windows Features > Windows Sandbox, then reboot" }
function Get-SandboxProcesses { @(Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Name -like "WindowsSandbox*" }) }
if (Get-SandboxProcesses) { throw "A Windows Sandbox is already open; close it first (only one can run)" }

# Tools, once.
$tools = Join-Path $Root "tools"
if (-not (Test-Path "$tools\python\python.exe") -or -not (Test-Path "$tools\cua-driver\cua-driver.exe")) {
    Write-Host "Preparing tools in $tools"
    $dl = Join-Path $Root "download"
    New-Item -ItemType Directory -Force $dl, "$tools\cua-driver" | Out-Null
    $ProgressPreference = "SilentlyContinue"
    Invoke-WebRequest -UseBasicParsing "https://www.nuget.org/api/v2/package/python/$PythonVersion" -OutFile "$dl\python.zip"
    Expand-Archive "$dl\python.zip" -DestinationPath "$dl\python" -Force
    $sig = Get-AuthenticodeSignature "$dl\python\tools\python.exe"
    if ($sig.Status -ne "Valid" -or $sig.SignerCertificate.Subject -notmatch "Python Software Foundation") { throw "python.exe in the NuGet package isn't signed by the PSF" }
    Remove-Item "$tools\python" -Recurse -Force -ErrorAction SilentlyContinue
    Copy-Item "$dl\python\tools" "$tools\python" -Recurse
    # Same Python version here, so its pip fetches the matching Pillow wheel.
    $ErrorActionPreference = "Continue"
    & python -m pip install --disable-pip-version-check --no-warn-script-location --target "$tools\python\Lib\site-packages" pillow
    $ErrorActionPreference = "Stop"
    & "$tools\python\python.exe" -I -c "import PIL" ; if ($LASTEXITCODE -ne 0) { throw "Pillow didn't install into $tools\python" }
    $cua = Split-Path (Get-Command cua-driver -ErrorAction Stop).Source
    Copy-Item "$cua\*.exe" "$tools\cua-driver\" -Force
    Remove-Item $dl -Recurse -Force
}

# The run folder the Sandbox sees as C:\revlc.
$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$share = Join-Path $Root "runs\$stamp"
New-Item -ItemType Directory -Force "$share\repo\scripts" | Out-Null
Copy-Item $Installer $share
if ($Previous) {
    New-Item -ItemType Directory -Force "$share\previous" | Out-Null
    Copy-Item $Previous "$share\previous"
    # The update check's clip, made here: releases before 0.2.10 ship no
    # FFmpeg, and the new one isn't installed yet when it's needed.
    $ffmpeg = (Get-Command ffmpeg -ErrorAction SilentlyContinue).Source
    if (-not $ffmpeg) { throw "-Previous needs ffmpeg on this PC's PATH, to make the update check's clip" }
    $ErrorActionPreference = "Continue"
    & $ffmpeg -hide_banner -loglevel error -y -f lavfi -i "testsrc2=size=640x360:rate=24" -t 30 -c:v mpeg4 -q:v 3 "$share\previous\upgrade-clip.mp4"
    $ErrorActionPreference = "Stop"
    if (-not (Test-Path "$share\previous\upgrade-clip.mp4")) { throw "ffmpeg didn't make the update check's clip" }
}
Copy-Item $qa "$share\repo\scripts\qa" -Recurse
# Generated test media, kept between runs: the suite only makes what's missing.
$media = Join-Path $Root "media-cache"
New-Item -ItemType Directory -Force $media | Out-Null
$commit = & git -C $qa rev-parse HEAD 2>$null
$logon = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:\revlc\repo\scripts\qa\sandbox\in-sandbox.ps1 -Commit $(if ($commit) { $commit } else { 'unknown' })"
if ($Only) { $logon += " -Only $Only" }
if ($Quick) { $logon += " -Quick" }
$wsb = Join-Path $share "revlc.wsb"
@"
<Configuration>
  <MappedFolders>
    <MappedFolder><HostFolder>$tools</HostFolder><SandboxFolder>C:\revlc-tools</SandboxFolder><ReadOnly>true</ReadOnly></MappedFolder>
    <MappedFolder><HostFolder>$share</HostFolder><SandboxFolder>C:\revlc</SandboxFolder><ReadOnly>false</ReadOnly></MappedFolder>
    <MappedFolder><HostFolder>$media</HostFolder><SandboxFolder>C:\revlc-media</SandboxFolder><ReadOnly>false</ReadOnly></MappedFolder>
  </MappedFolders>
  <vGPU>$(if ($NoGpu) { "Disable" } else { "Enable" })</vGPU>
  <LogonCommand><Command>$logon</Command></LogonCommand>
  <Networking>Disable</Networking>
  <MemoryInMB>4096</MemoryInMB>
  <AudioInput>Disable</AudioInput>
  <VideoInput>Disable</VideoInput>
  <ClipboardRedirection>Disable</ClipboardRedirection>
  <PrinterRedirection>Disable</PrinterRedirection>
</Configuration>
"@ | Set-Content $wsb -Encoding UTF8

$results = Join-Path $env:USERPROFILE "revlc-qa\sandbox-$stamp"
New-Item -ItemType Directory -Force $results | Out-Null
Start-Transcript -Path (Join-Path $results "run-sandbox.log") | Out-Null
$status = 1
try {
    Write-Host "Starting Windows Sandbox ($share)$(if ($NoGpu) { ', GPU off' })$(if ($Previous) { ', update from ' + (Split-Path $Previous -Leaf) })"
    Start-Process $sandboxExe -ArgumentList "`"$wsb`""
    $done = Join-Path $share "out\done.txt"
    $heartbeat = Join-Path $share "out\heartbeat.txt"
    $deadline = (Get-Date).AddMinutes($TimeoutMinutes)
    $lastBeat = $null; $lastChange = Get-Date; $seen = $false
    while (-not (Test-Path $done)) {
        Start-Sleep 20
        # The Sandbox shuts down seconds after writing done.txt: look again
        # before taking a closed Sandbox for a run cut short.
        if (Test-Path $done) { break }
        $running = [bool](Get-SandboxProcesses)
        if ($running) { $seen = $true }
        if ($seen -and -not $running) { throw "The Sandbox closed before the run finished" }
        if ((Get-Date) -gt $deadline) { throw "No result after $TimeoutMinutes min" }
        # Progress: the suite log's length, or before the suite starts, any
        # new file in the run folder's out\.
        $beat = if (Test-Path $heartbeat) { (Get-Content $heartbeat -Raw) -replace '^\S+ ', '' } else { (Get-ChildItem "$share\out" -Recurse -ErrorAction SilentlyContinue | Measure-Object).Count }
        if ("$beat" -ne "$lastBeat") { $lastBeat = $beat; $lastChange = Get-Date }
        elseif (((Get-Date) - $lastChange).TotalMinutes -ge $StallMinutes) { throw "No progress for $StallMinutes min (suite log stuck at $beat bytes)" }
    }
    $result = Get-Content (Join-Path $share "out\result.txt")
    $result | ForEach-Object { Write-Host "  $_" }
    $code = ($result | Where-Object { $_ -match "^Suite exit code: \d+$" } | Select-Object -Last 1) -replace "^Suite exit code: ", ""
    if ($code) { $status = [int]$code }
}
finally {
    # in-sandbox.ps1 shuts the Sandbox down; make sure it's gone.
    $wait = (Get-Date).AddSeconds(90)
    while ((Get-SandboxProcesses) -and (Get-Date) -lt $wait) { Start-Sleep 5 }
    Get-SandboxProcesses | Stop-Process -Force -ErrorAction SilentlyContinue
    if (Test-Path "$share\out") { Copy-Item "$share\out\*" $results -Recurse -Force }
    Copy-Item $wsb $results
    Remove-Item $share -Recurse -Force -ErrorAction SilentlyContinue
    $report = Get-ChildItem $results -Recurse -Filter report.md | Select-Object -First 1
    if ($report) { Write-Host ""; Get-Content $report.FullName -Encoding UTF8 | Write-Host }
    Write-Host "Results: $results"
    Stop-Transcript | Out-Null
}
exit $status
