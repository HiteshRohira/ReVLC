# Runs the Cua suite (scripts/qa/ci/cua_suite.py) on a fresh Windows in
# Windows Sandbox: a clean copy of this PC's Windows each time, nothing to
# maintain, gone when it closes.
#
#   scripts\qa\sandbox\run-sandbox.ps1 -Installer ReVLC-0.2.11-alpha-x64-setup.exe
#   scripts\qa\sandbox\run-sandbox.ps1 -Installer ... -Only playback,effects
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
# installed here (its binaries are copied), and no other Sandbox open.
param(
    [Parameter(Mandatory = $true)][string]$Installer,
    [string]$Only,
    [string]$Root = "G:\revlc-sandbox",
    [int]$TimeoutMinutes = 150,
    [int]$StallMinutes = 15,
    [string]$PythonVersion = "3.13.15"
)
$ErrorActionPreference = "Stop"
$qa = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$Installer = (Resolve-Path $Installer).Path
if ([IO.Path]::GetExtension($Installer) -ne ".exe") { throw "Pass the setup .exe" }
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
Copy-Item $qa "$share\repo\scripts\qa" -Recurse
$logon = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:\revlc\repo\scripts\qa\sandbox\in-sandbox.ps1"
if ($Only) { $logon += " -Only $Only" }
$wsb = Join-Path $share "revlc.wsb"
@"
<Configuration>
  <MappedFolders>
    <MappedFolder><HostFolder>$tools</HostFolder><SandboxFolder>C:\revlc-tools</SandboxFolder><ReadOnly>true</ReadOnly></MappedFolder>
    <MappedFolder><HostFolder>$share</HostFolder><SandboxFolder>C:\revlc</SandboxFolder><ReadOnly>false</ReadOnly></MappedFolder>
  </MappedFolders>
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
    Write-Host "Starting Windows Sandbox ($share)"
    Start-Process $sandboxExe -ArgumentList "`"$wsb`""
    $done = Join-Path $share "out\done.txt"
    $heartbeat = Join-Path $share "out\heartbeat.txt"
    $deadline = (Get-Date).AddMinutes($TimeoutMinutes)
    $lastBeat = $null; $lastChange = Get-Date; $seen = $false
    while (-not (Test-Path $done)) {
        Start-Sleep 20
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
