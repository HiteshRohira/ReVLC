# Runs the Cua suite (scripts/qa/ci/cua_suite.py) on this Windows PC against
# the build you're about to ship, with real video. Run it when you want to,
# for example before a release:
#
#   scripts\qa\device\run-windows.ps1 -Exe "$env:LOCALAPPDATA\ReVLC\revlc.exe"
#   scripts\qa\device\run-windows.ps1 -Installer ReVLC_0.2.11_x64-setup.exe   # VMs only
#   ... -Only playback,effects
#
# -Exe tests an installed (or unpacked) ReVLC where it is. -Installer
# installs into the run folder first; that rewrites ReVLC's registration
# (Start menu, uninstall entry), so it's for a throwaway VM, not your PC.
#
# WebView2 only shows the page to UI Automation when it thinks a screen
# reader is running, so this turns on Windows' screen-reader flag for the run
# and restores it after. The app itself isn't changed.
#
# Your ReVLC data is backed up before the first launch and restored at the
# end (scripts/qa/appdata.py); every launch has analytics off and no sound.
# It takes over the screen at times, so leave the PC unlocked and alone.
# Needs Cua Driver (running), Python with Pillow, and ReVLC not running.
param(
    [string]$Exe,
    [string]$Installer,
    [string]$Only
)
$ErrorActionPreference = "Stop"
$root = Resolve-Path (Join-Path $PSScriptRoot "..\..\..")
if (-not $Exe -and -not $Installer) { throw "Pass -Exe or -Installer" }

$run = Join-Path $env:USERPROFILE ("revlc-qa\device-windows-" + (Get-Date -Format "yyyyMMdd-HHmmss"))
New-Item -ItemType Directory -Force $run | Out-Null
Write-Host "Run folder: $run"

# Preconditions: each one is a reason a run would fail for the wrong reason.
if (Get-Process revlc -ErrorAction SilentlyContinue) { throw "ReVLC is running. Quit it first: the test build would hand its files to it." }
if (-not (Get-Command cua-driver -ErrorAction SilentlyContinue)) { throw "Cua Driver isn't installed: irm https://cua.ai/driver/install.ps1 | iex" }
& cua-driver status *> $null
if ($LASTEXITCODE -ne 0) { throw "Cua Driver's daemon isn't running: start it with 'cua-driver serve'" }
& python -c "import PIL" 2>$null
if ($LASTEXITCODE -ne 0) { throw "Python needs Pillow: python -m pip install pillow" }

if ($Installer) {
    $dest = Join-Path $run "app"
    # NSIS: /S silent, /D= install folder (must come last, unquoted).
    Start-Process -FilePath $Installer -ArgumentList "/S", "/D=$dest" -Wait
    $Exe = Get-ChildItem $dest -Filter *.exe | Where-Object { $_.Name -notmatch "^(uninstall|ffmpeg)" } | Select-Object -First 1 -ExpandProperty FullName
    if (-not $Exe) { throw "No ReVLC executable in $dest after installing" }
}
$Exe = (Resolve-Path $Exe).Path
$ffmpeg = Join-Path (Split-Path $Exe) "ffmpeg.exe"
if (-not (Test-Path $ffmpeg)) { $ffmpeg = (Get-Command ffmpeg -ErrorAction SilentlyContinue).Source }
if (-not $ffmpeg) { throw "No ffmpeg.exe next to $Exe or on PATH (it makes the test media)" }
(Get-Item $Exe).VersionInfo.ProductVersion | Set-Content (Join-Path $run "version.txt")
Write-Host "Testing $Exe ($((Get-Content (Join-Path $run 'version.txt'))))"

Add-Type -Namespace RevlcQa -Name ScreenReader -MemberDefinition @'
[DllImport("user32.dll", SetLastError = true)]
public static extern bool SystemParametersInfo(uint action, uint param, ref bool value, uint winIni);
[DllImport("user32.dll", SetLastError = true)]
public static extern bool SystemParametersInfo(uint action, uint param, System.IntPtr value, uint winIni);
'@
$SPI_GETSCREENREADER = 0x0046; $SPI_SETSCREENREADER = 0x0047; $SPIF_SENDCHANGE = 0x2
$was = $false
[void][RevlcQa.ScreenReader]::SystemParametersInfo($SPI_GETSCREENREADER, 0, [ref]$was, 0)
$status = 0
try {
    [void][RevlcQa.ScreenReader]::SystemParametersInfo($SPI_SETSCREENREADER, 1, [System.IntPtr]::Zero, $SPIF_SENDCHANGE)
    $suiteArgs = @("$root\scripts\qa\ci\cua_suite.py", "--out", $run, "--ffmpeg", $ffmpeg)
    if ($Only) { $suiteArgs += @("--only", $Only) }
    $suiteArgs += @("--", $Exe)
    & python @suiteArgs 2>&1 | Tee-Object -FilePath (Join-Path $run "suite.log")
    $status = $LASTEXITCODE
}
finally {
    [void][RevlcQa.ScreenReader]::SystemParametersInfo($SPI_SETSCREENREADER, [uint32]$was, [System.IntPtr]::Zero, $SPIF_SENDCHANGE)
}
Write-Host ""
Write-Host "Report: $run\report.md"
exit $status
