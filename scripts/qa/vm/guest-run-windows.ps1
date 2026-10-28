# Runs inside a Windows guest (from run-vm.ps1, in the logged-on user's
# desktop session): installs ReVLC the way a user would (setup.exe /S), then
# runs the Cua suite against the installed app with run-windows.ps1, which
# turns on the screen-reader flag for the run and restores it after.
# Writes $Out\result.txt (install details, the run folder) and $Out\done.txt.
param(
    [Parameter(Mandatory = $true)][string]$Installer,
    [Parameter(Mandatory = $true)][string]$Out,
    [string]$Only
)
$ErrorActionPreference = "Stop"
New-Item -ItemType Directory -Force $Out | Out-Null
$result = Join-Path $Out "result.txt"
$Done = Join-Path $Out "done.txt"
trap { "$_`n$($_.ScriptStackTrace)" | Add-Content $result; "failed" | Set-Content $Done; exit 1 }
$repo = Resolve-Path (Join-Path $PSScriptRoot "..\..\..")

# Cua's daemon, in this session (tools were installed with -NoAutoStart).
$cua = Join-Path $env:LOCALAPPDATA "Programs\Cua\cua-driver\bin\cua-driver.exe"
$env:Path = "$(Split-Path $cua);" + [Environment]::GetEnvironmentVariable("Path", "User") + ";" + [Environment]::GetEnvironmentVariable("Path", "Machine")
$ErrorActionPreference = "Continue"
& $cua status *> $null
if ($LASTEXITCODE -ne 0) {
    Start-Process $cua -ArgumentList "serve" -WindowStyle Hidden -WorkingDirectory $env:USERPROFILE
    foreach ($i in 1..30) { Start-Sleep 1; & $cua status *> $null; if ($LASTEXITCODE -eq 0) { break } }
}
$ErrorActionPreference = "Stop"

# Install as a user would, silently. Tauri's NSIS installer installs per-user
# by default (no UAC) and fetches WebView2 if the OS lacks it.
"WebView2 before install: $((Get-ItemProperty 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}' -ErrorAction SilentlyContinue).pv)" | Set-Content $result
$t = Get-Date
$p = Start-Process $Installer -ArgumentList "/S" -Wait -PassThru
"Installer exit code: $($p.ExitCode) after $([int]((Get-Date) - $t).TotalSeconds) s" | Add-Content $result
$entry = Get-ItemProperty "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*", "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*" -ErrorAction SilentlyContinue |
    Where-Object { $_.DisplayName -like "ReVLC*" } | Select-Object -First 1
$dir = if ($entry -and $entry.InstallLocation) { $entry.InstallLocation.Trim('"') } else { Join-Path $env:LOCALAPPDATA "ReVLC" }
$exe = Get-ChildItem $dir -Filter *.exe -ErrorAction SilentlyContinue | Where-Object { $_.Name -notmatch "^(uninstall|ffmpeg)" } | Select-Object -First 1 -ExpandProperty FullName
"Uninstall entry: $($entry.DisplayName) $($entry.DisplayVersion)" | Add-Content $result
"Installed exe: $exe" | Add-Content $result
"WebView2 after install: $((Get-ItemProperty 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}' -ErrorAction SilentlyContinue).pv)" | Add-Content $result
if (-not $exe) { throw "No ReVLC executable in $dir after installing" }

$runArgs = @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "$repo\scripts\qa\device\run-windows.ps1", "-Exe", $exe)
if ($Only) { $runArgs += @("-Only", $Only) }
$ErrorActionPreference = "Continue"
& powershell @runArgs 2>&1 | ForEach-Object { "$_" } | Set-Content (Join-Path $Out "run-windows.log") -Encoding UTF8
$status = $LASTEXITCODE
$ErrorActionPreference = "Stop"
$run = Get-ChildItem (Join-Path $env:USERPROFILE "revlc-qa") -Directory -Filter "device-windows-*" | Sort-Object Name | Select-Object -Last 1 -ExpandProperty FullName
"Suite exit code: $status" | Add-Content $result
"Run folder: $run" | Add-Content $result
"ok" | Set-Content $Done
