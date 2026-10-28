# Installs a ReVLC build on a clean VM the way a user would and runs the Cua
# suite (scripts/qa/ci/cua_suite.py) against it, inside the guest:
#
#   scripts\qa\vm\run-vm.ps1 -Vm revlc-win11 -Installer ReVLC-0.2.10-alpha-x64-setup.exe
#   scripts\qa\vm\run-vm.ps1 -Vm revlc-linux -Installer ReVLC-0.2.10-alpha-amd64.deb -Only playback
#
# Restores the VM's "tools" snapshot (new-vms.ps1), starts it headless,
# copies the installer and this repo's scripts\qa in, installs (setup.exe /S,
# or apt install ./x.deb), runs the suite in the logged-on desktop session
# (Windows: through run-windows.ps1, with the screen-reader flag), copies
# the results to %USERPROFILE%\revlc-qa\vm-<name>-<timestamp> and powers the
# VM off. The next run restores the snapshot again, so nothing carries over.
param(
    [Parameter(Mandatory = $true)][string]$Vm,
    [Parameter(Mandatory = $true)][string]$Installer,
    [string]$Only,
    [int]$TimeoutMinutes = 180
)
$ErrorActionPreference = "Stop"
. "$PSScriptRoot\vm-common.ps1"
$qa = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$spec = $VmSpecs[$Vm]
if (-not $spec) { throw "Unknown VM $Vm (one of: $($VmSpecs.Keys -join ', '))" }
$Installer = (Resolve-Path $Installer).Path
# The guest command line (a scheduled task's) passes paths unquoted.
if ((Split-Path $Installer -Leaf) -match '\s') { throw "Rename the installer without spaces: $(Split-Path $Installer -Leaf)" }
$ext = [IO.Path]::GetExtension($Installer)
if (($spec.Os -eq "windows" -and $ext -ne ".exe") -or ($spec.Os -eq "linux" -and $ext -ne ".deb")) {
    throw "$Vm needs a $(if ($spec.Os -eq 'windows') { 'setup .exe' } else { '.deb' }), not $(Split-Path $Installer -Leaf)"
}

$status = 1
$out = Join-Path $env:USERPROFILE ("revlc-qa\vm-$Vm-" + (Get-Date -Format "yyyyMMdd-HHmmss"))
New-Item -ItemType Directory -Force $out | Out-Null
Write-Host "Results: $out"
Start-Transcript -Path (Join-Path $out "run-vm.log") | Out-Null

try {
    Stop-Vm $Vm
    Invoke-VBox snapshot $Vm restore tools | Out-Null
    Start-Headless $Vm { Invoke-VBox startvm $Vm --type headless | Out-Null }
    Write-Host "[$Vm] started; waiting for the desktop"
    Wait-GuestLogin $Vm 1800

    $leaf = Split-Path $Installer -Leaf
    if ($spec.Os -eq "windows") {
        $g = "C:\Users\$GuestUser\revlc-vm"
        Copy-ToGuest $Vm $Installer $g
        Copy-ToGuest $Vm $qa "$g\repo\scripts"
        $cmd = "powershell.exe -NoProfile -WindowStyle Minimized -ExecutionPolicy Bypass -File $g\repo\scripts\qa\vm\guest-run-windows.ps1 -Installer $g\$leaf -Out $g\out"
        if ($Only) { $cmd += " -Only $Only" }
        Write-Host "[$Vm] installing $leaf and running the suite"
        Invoke-GuestDesktop $Vm $cmd "$g\out\done.txt" ($TimeoutMinutes * 60)
        Copy-FromGuest $Vm "$g\out" $out
    } else {
        $g = "/home/$GuestUser/revlc-vm"
        Copy-ToGuest $Vm $Installer $g
        Copy-ToGuest $Vm $qa "$g/repo/scripts"
        $a = @("$g/repo/scripts/qa/vm/guest-run-linux.sh", "$g/$leaf", "$g/out")
        if ($Only) { $a += @("--only", $Only) }
        Write-Host "[$Vm] installing $leaf and running the suite"
        Invoke-Guest $Vm "/bin/bash" $a -Env @("SUDO_PASS=$GuestPassword") -TimeoutSec ($TimeoutMinutes * 60) -AllowFailure | Out-Null
        Copy-FromGuest $Vm "$g/out" $out
    }

    $result = Get-Content (Join-Path $out "out\result.txt") -ErrorAction SilentlyContinue
    $result | ForEach-Object { Write-Host "  $_" }
    $guestRun = ($result | Where-Object { $_ -like "Run folder: *" } | Select-Object -Last 1) -replace "^Run folder: ", ""
    # The suite's exit code: 0 only if it ran and nothing failed.
    $code = ($result | Where-Object { $_ -match "^Suite exit code: \d+$" } | Select-Object -Last 1) -replace "^Suite exit code: ", ""
    if ($code) { $status = [int]$code }
    if ($guestRun) {
        Copy-FromGuest $Vm $guestRun $out
        $report = Get-ChildItem $out -Recurse -Filter report.md | Select-Object -First 1
        if ($report) { Write-Host ""; Get-Content $report.FullName | Write-Host }
    } else {
        Write-Host "[$Vm] the suite didn't run; see $out\out"
    }
}
finally {
    # Power off without saving: restoring "tools" next time discards it all.
    if ((Get-VmInfo $Vm "VMState") -ne "poweroff") { Invoke-VBox controlvm $Vm poweroff | Out-Null }
    Wait-PoweredOff $Vm 120
    Stop-Transcript | Out-Null
}
Write-Host "Results: $out"
exit $status
