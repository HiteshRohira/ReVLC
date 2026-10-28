# Runs inside Windows Sandbox, as its logon command (run-sandbox.ps1 writes
# the .wsb): installs ReVLC the way a user would (setup.exe /S), starts Cua
# Driver in this desktop session, runs the Cua suite against the installed
# app with run-windows.ps1 (screen-reader flag, app data backup, analytics
# off, no sound), copies the run folder to the mapped folder and shuts the
# Sandbox down. Nothing is downloaded: Python + Pillow and Cua Driver come
# read-only from the mapped tools folder, ffmpeg with ReVLC.
#
# Writes to $Share\out: result.txt, heartbeat.txt (the suite log's length,
# for the host's stall check), the run folder, and done.txt last.
param(
    [string]$Share = "C:\revlc",
    [string]$Tools = "C:\revlc-tools",
    [string]$Only
)
$ErrorActionPreference = "Stop"
$out = Join-Path $Share "out"
New-Item -ItemType Directory -Force $out | Out-Null
$result = Join-Path $out "result.txt"
$done = Join-Path $out "done.txt"
Start-Transcript -Path (Join-Path $out "in-sandbox.log") | Out-Null
trap {
    "$_`n$($_.ScriptStackTrace)" | Add-Content $result
    "failed" | Set-Content $done
    Stop-Transcript | Out-Null
    shutdown.exe /s /t 5
    exit 1
}

$env:Path = "$Tools\python;$Tools\python\Scripts;$Tools\cua-driver;$env:Path"
$installer = Get-ChildItem $Share -Filter *.exe | Select-Object -First 1 -ExpandProperty FullName
if (-not $installer) { throw "No installer in $Share" }

# Cua's daemon, in this session.
$ErrorActionPreference = "Continue"
Start-Process "$Tools\cua-driver\cua-driver.exe" -ArgumentList "serve" -WindowStyle Hidden -WorkingDirectory $env:USERPROFILE
foreach ($i in 1..30) { Start-Sleep 1; & cua-driver status *> $null; if ($LASTEXITCODE -eq 0) { break } }
$ErrorActionPreference = "Stop"
"Cua Driver: $(& cua-driver --version)" | Set-Content $result

# The Sandbox's registry comes from its base image, but Program Files is
# this PC's: once the host's WebView2 updates, the registered version names
# a folder that isn't there and ReVLC reports the runtime missing (fixing
# the registered version isn't enough). WebView2's documented override
# points it at the folder that is; ReVLC's launches inherit it.
"OS: $((Get-CimInstance Win32_OperatingSystem).Caption) $([Environment]::OSVersion.Version)" | Add-Content $result
$wvDir = "${env:ProgramFiles(x86)}\Microsoft\EdgeWebView\Application"
$present = Get-ChildItem $wvDir -Directory -ErrorAction SilentlyContinue | Where-Object { $_.Name -match '^\d+(\.\d+){3}$' -and (Test-Path "$($_.FullName)\msedgewebview2.exe") } |
    Sort-Object { [version]$_.Name } | Select-Object -Last 1 -ExpandProperty Name
if (-not $present) { throw "No WebView2 runtime in $wvDir" }
$env:WEBVIEW2_BROWSER_EXECUTABLE_FOLDER = Join-Path $wvDir $present
"WebView2: $env:WEBVIEW2_BROWSER_EXECUTABLE_FOLDER" | Add-Content $result
$t = Get-Date
$p = Start-Process $installer -ArgumentList "/S" -Wait -PassThru
"Installer exit code: $($p.ExitCode) after $([int]((Get-Date) - $t).TotalSeconds) s" | Add-Content $result
$entry = Get-ItemProperty "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*", "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*" -ErrorAction SilentlyContinue |
    Where-Object { $_.DisplayName -like "ReVLC*" } | Select-Object -First 1
$dir = if ($entry -and $entry.InstallLocation) { $entry.InstallLocation.Trim('"') } else { Join-Path $env:LOCALAPPDATA "ReVLC" }
$exe = Get-ChildItem $dir -Filter *.exe -ErrorAction SilentlyContinue | Where-Object { $_.Name -notmatch "^(uninstall|ffmpeg)" } | Select-Object -First 1 -ExpandProperty FullName
"Uninstall entry: $($entry.DisplayName) $($entry.DisplayVersion)" | Add-Content $result
"Installed exe: $exe" | Add-Content $result
if (-not $exe) { throw "No ReVLC executable in $dir after installing" }

# The suite log grows as checks run; the host watches its length.
$heartbeat = Start-Job -ArgumentList $env:USERPROFILE, (Join-Path $out "heartbeat.txt") -ScriptBlock {
    param($profileDir, $file)
    while ($true) {
        $log = Get-ChildItem (Join-Path $profileDir "revlc-qa") -Recurse -Filter suite.log -ErrorAction SilentlyContinue | Select-Object -First 1
        "$(Get-Date -Format o) $(if ($log) { $log.Length } else { 0 })" | Set-Content $file
        Start-Sleep 30
    }
}

$runArgs = @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "$Share\repo\scripts\qa\device\run-windows.ps1", "-Exe", $exe)
if ($Only) { $runArgs += @("-Only", $Only) }
$ErrorActionPreference = "Continue"
& powershell @runArgs 2>&1 | ForEach-Object { "$_" } | Set-Content (Join-Path $out "run-windows.log") -Encoding UTF8
$status = $LASTEXITCODE
$ErrorActionPreference = "Stop"
Stop-Job $heartbeat; Remove-Job $heartbeat -Force

$run = Get-ChildItem (Join-Path $env:USERPROFILE "revlc-qa") -Directory -Filter "device-windows-*" | Sort-Object Name | Select-Object -Last 1
if ($run) {
    # The suite's media is regenerated each run; leave it behind.
    robocopy $run.FullName (Join-Path $out $run.Name) /E /XD media /NFL /NDL /NJH /NJS /NP | Out-Null
}
"Suite exit code: $status" | Add-Content $result
"Run folder: $(if ($run) { $run.Name })" | Add-Content $result
"ok" | Set-Content $done
Stop-Transcript | Out-Null
shutdown.exe /s /t 5
