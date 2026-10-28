# Runs inside a Windows guest (from new-vms.ps1, in the logged-on user's
# desktop session) to add what the Cua suite needs, all per-user and without
# admin: Python + Pillow, ffmpeg and Cua Driver. Nothing here installs the
# Visual C++ runtime system-wide (python.org's installer keeps its own copy
# of vcruntime140.dll next to python.exe); tools.txt records the system's VC++
# runtimes and uninstall entries before and after, to show it.
param(
    [Parameter(Mandatory = $true)][string]$Out,
    [string]$PythonVersion = "3.13.15"
)
$ErrorActionPreference = "Stop"
New-Item -ItemType Directory -Force $Out | Out-Null
# The host waits for done.txt: write it whatever happens.
$Done = Join-Path $Out "done.txt"
trap { "$_`n$($_.ScriptStackTrace)" | Set-Content (Join-Path $Out "error.txt"); "failed" | Set-Content $Done; exit 1 }
$log = Join-Path $Out "tools.txt"
Start-Transcript -Path (Join-Path $Out "tools-transcript.txt") | Out-Null

function Get-Installed {
    $keys = "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*",
        "HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*",
        "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*"
    Get-ItemProperty $keys -ErrorAction SilentlyContinue | Where-Object DisplayName |
        ForEach-Object { "$($_.DisplayName) $($_.DisplayVersion)" } | Sort-Object -Unique
}
function Get-VcRuntimes {
    $r = @(Get-ChildItem "$env:SystemRoot\System32\vcruntime140*.dll", "$env:SystemRoot\System32\msvcp140*.dll" -ErrorAction SilentlyContinue | ForEach-Object { "$($_.Name) $($_.VersionInfo.FileVersion)" })
    $k = Get-ItemProperty "HKLM:\SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64" -ErrorAction SilentlyContinue
    if ($k) { $r += "VC++ 14 x64 runtime registered: $($k.Version)" }
    if (-not $r) { $r = @("none") }
    $r
}
$before = Get-Installed
$vcBefore = Get-VcRuntimes
$tmp = Join-Path $env:TEMP "revlc-tools"
New-Item -ItemType Directory -Force $tmp | Out-Null
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$ProgressPreference = "SilentlyContinue"

# Python (per-user, on the user PATH, no py launcher: that one is system-wide).
$py = Join-Path $tmp "python-$PythonVersion-amd64.exe"
Invoke-WebRequest -UseBasicParsing "https://www.python.org/ftp/python/$PythonVersion/python-$PythonVersion-amd64.exe" -OutFile $py
$p = Start-Process $py -ArgumentList "/quiet", "InstallAllUsers=0", "PrependPath=1", "Include_launcher=0", "Include_test=0", "Include_doc=0", "Include_tcltk=0", "Shortcuts=0" -Wait -PassThru
if ($p.ExitCode -ne 0) { throw "Python installer exited $($p.ExitCode)" }
$pyDir = Join-Path $env:LOCALAPPDATA ("Programs\Python\Python" + ($PythonVersion -replace '^(\d+)\.(\d+).*$', '$1$2'))
& "$pyDir\python.exe" -m pip install --disable-pip-version-check --no-warn-script-location pillow
if ($LASTEXITCODE -ne 0) { throw "pip install pillow failed" }

# ffmpeg (makes the suite's test media): gyan.dev's static build, per-user.
$zip = Join-Path $tmp "ffmpeg.zip"
Invoke-WebRequest -UseBasicParsing "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip" -OutFile $zip
$ff = Join-Path $env:LOCALAPPDATA "Programs\ffmpeg"
Expand-Archive $zip -DestinationPath $tmp -Force
$src = Get-ChildItem $tmp -Directory -Filter "ffmpeg-*" | Select-Object -First 1
New-Item -ItemType Directory -Force $ff | Out-Null
Copy-Item "$($src.FullName)\bin" $ff -Recurse -Force
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($userPath -notlike "*$ff\bin*") { [Environment]::SetEnvironmentVariable("Path", "$userPath;$ff\bin", "User") }

# Cua Driver, per-user. Its default auto-start needs a UAC prompt, so skip
# it: guest-run-windows.ps1 starts the daemon in the desktop session itself.
$cua = Join-Path $tmp "cua-install.ps1"
Invoke-WebRequest -UseBasicParsing "https://cua.ai/driver/install.ps1" -OutFile $cua
& powershell -NoProfile -ExecutionPolicy Bypass -File $cua -NoAutoStart
$cuaExe = Join-Path $env:LOCALAPPDATA "Programs\Cua\cua-driver\bin\cua-driver.exe"
if (-not (Test-Path $cuaExe)) { throw "Cua Driver didn't install to $cuaExe" }

$after = Get-Installed
@(
    "Python: $(& "$pyDir\python.exe" --version 2>&1) at $pyDir"
    "Pillow: $(& "$pyDir\python.exe" -c 'import PIL; print(PIL.__version__)')"
    "ffmpeg: $((& "$ff\bin\ffmpeg.exe" -version | Select-Object -First 1))"
    "Cua Driver: $(& $cuaExe --version)"
    ""
    "VC++ runtime files/registration before: $($vcBefore -join '; ')"
    "VC++ runtime files/registration after:  $((Get-VcRuntimes) -join '; ')"
    ""
    "Uninstall entries added:"
    (Compare-Object $before $after | Where-Object SideIndicator -eq "=>" | ForEach-Object { "  $($_.InputObject)" })
) | Set-Content $log -Encoding UTF8
Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
Stop-Transcript | Out-Null
"ok" | Set-Content $Done
