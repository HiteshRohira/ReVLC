# Shared helpers for the VirtualBox scripts in this folder (new-vms.ps1,
# run-vm.ps1). Dot-source it: . "$PSScriptRoot\vm-common.ps1"
#
# The VMs: revlc-win11, revlc-win10 (Windows, local admin user with
# auto-logon) and revlc-linux (Ubuntu 24.04 desktop, auto-login). All three
# use the same throwaway account, revlc / revlc.

$VBox = Join-Path $env:ProgramFiles "Oracle\VirtualBox\VBoxManage.exe"
if (-not (Test-Path $VBox)) { throw "VirtualBox isn't installed: winget install Oracle.VirtualBox" }

$GuestUser = "revlc"
$GuestPassword = "revlc"

$VmSpecs = @{
    "revlc-win11" = @{ Os = "windows"; OsType = "Windows11_64"; Iso = "Win11_*_x64*.iso"; Edition = "Windows 11 Pro" }
    "revlc-win10" = @{ Os = "windows"; OsType = "Windows10_64"; Iso = "Win10_22H2_*_x64*.iso"; Edition = "Windows 10 Pro"; Firmware = "efi"; Paravirt = "minimal" }
    "revlc-linux" = @{ Os = "linux"; OsType = "Ubuntu24_LTS_64"; Iso = "ubuntu-24.04*-desktop-amd64.iso" }
}

function Invoke-VBox {
    # VBoxManage writes progress to stderr; Windows PowerShell 5.1 would turn
    # that into errors, so only the exit code decides.
    $old = $ErrorActionPreference; $ErrorActionPreference = "Continue"
    $out = & $VBox @args 2>&1 | ForEach-Object { "$_" }
    $code = $LASTEXITCODE
    $ErrorActionPreference = $old
    if ($code -ne 0) { throw "VBoxManage $($args -join ' ') failed ($code):`n$($out -join "`n")" }
    $out
}

function Test-Vm([string]$Vm) {
    $old = $ErrorActionPreference; $ErrorActionPreference = "Continue"
    & $VBox showvminfo $Vm --machinereadable *> $null
    $ok = $LASTEXITCODE -eq 0
    $ErrorActionPreference = $old
    $ok
}

function Get-VmInfo([string]$Vm, [string]$Key) {
    $line = Invoke-VBox showvminfo $Vm --machinereadable | Where-Object { $_ -like "$Key=*" } | Select-Object -First 1
    if ($line) { $line.Substring($Key.Length + 1).Trim('"') }
}

function Wait-Until([scriptblock]$Condition, [string]$What, [int]$TimeoutSec, [int]$EverySec = 15) {
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $deadline) {
        if (& $Condition) { return }
        Start-Sleep -Seconds $EverySec
    }
    throw "Timed out after $TimeoutSec s waiting for $What"
}

$VmProcesses = @{}

function Get-HeadlessIds { @(Get-Process VBoxHeadless -ErrorAction SilentlyContinue | ForEach-Object Id) }

function Start-Headless([string]$Vm, [scriptblock]$Start) {
    # Remembers the VBoxHeadless processes a start creates: on this host
    # (VirtualBox on Hyper-V) one has outlived its powered-off VM, holding
    # its 4 GB until the next VM starved. Wait-PoweredOff ends them.
    $before = Get-HeadlessIds
    & $Start
    Start-Sleep -Seconds 3
    $script:VmProcesses[$Vm] = @(Get-HeadlessIds | Where-Object { $before -notcontains $_ })
}

function Remove-Leftovers([string]$Vm) {
    $ids = $script:VmProcesses[$Vm]
    if (-not $ids) { return }
    $deadline = (Get-Date).AddSeconds(60)
    while ((Get-Date) -lt $deadline -and (Get-Process -Id $ids -ErrorAction SilentlyContinue)) { Start-Sleep -Seconds 5 }
    $left = Get-Process -Id $ids -ErrorAction SilentlyContinue
    if ($left) {
        Write-Host "[$Vm] ending $($left.Count) VirtualBox process(es) left after power-off"
        $left | Stop-Process -Force
    }
    $script:VmProcesses.Remove($Vm)
}

function Wait-PoweredOff([string]$Vm, [int]$TimeoutSec = 600) {
    Wait-Until { (Get-VmInfo $Vm "VMState") -eq "poweroff" } "$Vm to power off" $TimeoutSec
    Remove-Leftovers $Vm
}

function Stop-Vm([string]$Vm) {
    # A clean shutdown first, so snapshots and the guest's disk are consistent.
    if ((Get-VmInfo $Vm "VMState") -eq "poweroff") { return }
    try { Invoke-VBox controlvm $Vm acpipowerbutton | Out-Null; Wait-PoweredOff $Vm 240 }
    catch { Invoke-VBox controlvm $Vm poweroff | Out-Null; Wait-PoweredOff $Vm 60 }
}

function Get-GuestProperty([string]$Vm, [string]$Name) {
    $line = Invoke-VBox guestproperty get $Vm $Name
    if ($line -match "^Value:\s*(.*)$") { $Matches[1] }
}

function Wait-GuestLogin([string]$Vm, [int]$TimeoutSec = 900) {
    # Guest Additions report the logged-in users once the desktop is up.
    Wait-Until { [int](Get-GuestProperty $Vm "/VirtualBox/GuestInfo/OS/LoggedInUsers") -ge 1 } "a user to be logged in on $Vm" $TimeoutSec
    # The desktop keeps starting things for a while after the logon.
    Start-Sleep -Seconds 45
}

function Invoke-Guest([string]$Vm, [string]$Exe, [string[]]$Arguments = @(), [string[]]$Env = @(), [int]$TimeoutSec = 1800, [switch]$AllowFailure) {
    # No --exe: with it, Windows guests pass argv[0] on as a first argument.
    $a = @("guestcontrol", $Vm, "run", "--username", $GuestUser, "--password", $GuestPassword,
        "--timeout", ($TimeoutSec * 1000), "--wait-stdout", "--wait-stderr")
    foreach ($e in $Env) { $a += @("--putenv", $e) }
    $a += @("--", $Exe) + $Arguments
    $old = $ErrorActionPreference; $ErrorActionPreference = "Continue"
    $out = & $VBox @a 2>&1 | ForEach-Object { "$_" }
    $code = $LASTEXITCODE
    $ErrorActionPreference = $old
    if ($code -ne 0 -and -not $AllowFailure) { throw "In $Vm, $Exe $($Arguments -join ' ') failed ($code):`n$($out -join "`n")" }
    $out
}

function Test-GuestPath([string]$Vm, [string]$Path) {
    $old = $ErrorActionPreference; $ErrorActionPreference = "Continue"
    & $VBox guestcontrol $Vm stat --username $GuestUser --password $GuestPassword $Path *> $null
    $ok = $LASTEXITCODE -eq 0
    $ErrorActionPreference = $old
    $ok
}

function Copy-ToGuest([string]$Vm, [string]$Source, [string]$GuestDir) {
    # Copies a file into $GuestDir, or a folder to $GuestDir/<folder name>
    # (copyto on its own copies a folder's contents, not the folder).
    $dir = (Test-Path $Source -PathType Container)
    if ($dir) { $GuestDir = $GuestDir.TrimEnd("\", "/") + $(if ($GuestDir -like "*\*") { "\" } else { "/" }) + (Split-Path $Source -Leaf) }
    Invoke-VBox guestcontrol $Vm mkdir --username $GuestUser --password $GuestPassword --parents $GuestDir | Out-Null
    # A file needs a trailing separator, or copyto treats the existing
    # folder as the destination file.
    $target = if ($dir) { $GuestDir } else { $GuestDir.TrimEnd("\", "/") + $(if ($GuestDir -like "*\*") { "\" } else { "/" }) }
    $a = @("guestcontrol", $Vm, "copyto", "--username", $GuestUser, "--password", $GuestPassword, "--target-directory", $target)
    if ($dir) { $a += "--recursive" }
    Invoke-VBox @a $Source | Out-Null
}

function Copy-FromGuest([string]$Vm, [string]$GuestPath, [string]$HostDir) {
    # A guest folder lands in $HostDir\<folder name>, a file in $HostDir.
    if (-not (Test-GuestPath $Vm $GuestPath)) { throw "$GuestPath isn't in $Vm" }
    $leaf = ($GuestPath.TrimEnd("\", "/") -split '[\\/]')[-1]
    if ($leaf -notmatch '\.[A-Za-z0-9]+$') { $HostDir = Join-Path $HostDir $leaf }
    New-Item -ItemType Directory -Force $HostDir | Out-Null
    Invoke-VBox guestcontrol $Vm copyfrom --username $GuestUser --password $GuestPassword --recursive --target-directory $HostDir $GuestPath | Out-Null
}

function Invoke-GuestDesktop([string]$Vm, [string]$Command, [string]$DoneFile, [int]$TimeoutSec) {
    # Windows guests: guestcontrol starts processes outside the user's
    # desktop, where nothing can see or drive windows. A scheduled task with
    # /IT runs in the logged-on user's session instead. The command must
    # create $DoneFile when it finishes.
    $task = "revlc-vm-step"
    Invoke-Guest $Vm "C:\Windows\System32\schtasks.exe" @("/create", "/tn", $task, "/tr", $Command, "/sc", "once", "/st", "23:59", "/it", "/f") | Out-Null
    Invoke-Guest $Vm "C:\Windows\System32\schtasks.exe" @("/run", "/tn", $task) | Out-Null
    $script:stoppedPolls = 0
    Wait-Until {
        if (Test-GuestPath $Vm $DoneFile) { return $true }
        # Finished (has run: Last Result isn't 267011, and Ready again) on two
        # polls running without $DoneFile: the command itself failed. A fresh
        # Windows 10 can take a while to start the task at all.
        $info = Invoke-Guest $Vm "C:\Windows\System32\schtasks.exe" @("/query", "/tn", $task, "/v", "/fo", "list") -AllowFailure | Where-Object { $_ -match "^(Status|Last Result):" }
        $ran = -not ($info -match "^Last Result:\s+267011$")
        $ready = [bool]($info -match "^Status:\s+Ready$")
        $script:stoppedPolls = if ($ran -and $ready) { $script:stoppedPolls + 1 } else { 0 }
        if ($script:stoppedPolls -ge 2 -and -not (Test-GuestPath $Vm $DoneFile)) {
            throw "The task in $Vm stopped without writing $DoneFile`n$($info -join "`n")"
        }
        $false
    } "$DoneFile in $Vm" $TimeoutSec 20
    Invoke-Guest $Vm "C:\Windows\System32\schtasks.exe" @("/delete", "/tn", $task, "/f") -AllowFailure | Out-Null
}
