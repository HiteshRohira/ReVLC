# (Re)creates the throwaway VirtualBox VMs that run-vm.ps1 tests ReVLC in,
# each with two snapshots:
#
#   clean  a freshly installed OS, powered off: Guest Additions, auto-logon,
#          and (Windows) Windows Update and the first-run privacy and
#          "finish setting up" screens off. Nothing else.
#   tools  clean + what the test run needs inside the guest: Python with
#          Pillow, ffmpeg and Cua Driver (guest-tools-*.{ps1,sh} lists what
#          each adds; tools.txt in the VM folder records what got installed).
#
#   scripts\qa\vm\new-vms.ps1 -IsoDir G:\revlc-vms\iso                 # all three
#   scripts\qa\vm\new-vms.ps1 -IsoDir ... -Vm revlc-win11 -Recreate    # start over
#   scripts\qa\vm\new-vms.ps1 -Vm revlc-linux -ToolsOnly               # redo "tools" from "clean"
#
# ISOs (put them in -IsoDir; the names are matched by wildcard):
#   Win11_*_x64*.iso, Win10_22H2_*_x64*.iso  Microsoft's, from
#       microsoft.com/software-download or the Fido script (same links)
#   ubuntu-24.04*-desktop-amd64.iso          releases.ubuntu.com/24.04
#
# VMs: 4 GB RAM, 2 CPUs, 64 GB dynamic disk, in -BaseFolder. User revlc /
# password revlc (vm-common.ps1). A Windows install takes 30-60 minutes,
# Ubuntu's about 20; the VMs run headless and end powered off.
param(
    [string[]]$Vm = @("revlc-win11", "revlc-win10", "revlc-linux"),
    [string]$IsoDir = "G:\revlc-vms\iso",
    [string]$BaseFolder = "G:\revlc-vms",
    [switch]$Recreate,
    [switch]$ToolsOnly
)
$ErrorActionPreference = "Stop"
. "$PSScriptRoot\vm-common.ps1"
$qa = Resolve-Path (Join-Path $PSScriptRoot "..")

function ConvertTo-EncodedCommand([string]$Script) {
    [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($Script))
}

# Runs once, elevated, at the first logon (the unattended install's
# FirstLogonCommands), after the Guest Additions are installed.
$WindowsPostInstall = @"
`$ErrorActionPreference = 'Continue'
`$wl = 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon'
Set-ItemProperty `$wl AutoAdminLogon '1'
Set-ItemProperty `$wl DefaultUserName '$GuestUser'
Set-ItemProperty `$wl DefaultPassword '$GuestPassword'
Remove-ItemProperty `$wl AutoLogonCount -ErrorAction SilentlyContinue
function Set-Dword(`$key, `$name, `$value) { New-Item `$key -Force -ErrorAction SilentlyContinue | Out-Null; New-ItemProperty `$key -Name `$name -Value `$value -PropertyType DWord -Force | Out-Null }
Set-Dword 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU' NoAutoUpdate 1
Set-Dword 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU' AUOptions 1
Set-Dword 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate' DisableDualScan 1
Set-Dword 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\DataCollection' AllowTelemetry 0
Set-Dword 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\OOBE' DisablePrivacyExperience 1
Set-Dword 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\CloudContent' DisableWindowsConsumerFeatures 1
Set-Dword 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\Personalization' NoLockScreen 1
Set-Dword 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon' EnableFirstLogonAnimation 0
Set-Dword 'HKCU:\Software\Microsoft\Windows\CurrentVersion\UserProfileEngagement' ScoobeSystemSettingEnabled 0
sc.exe config wuauserv start= disabled | Out-Null
sc.exe stop wuauserv | Out-Null
powercfg /change monitor-timeout-ac 0
powercfg /change standby-timeout-ac 0
powercfg /hibernate off
shutdown.exe /s /t 60 /c 'revlc: unattended install done'
"@

# Runs as root inside the installed system (the autoinstall's late-commands).
$LinuxPostInstall = @"
set -x
mkdir -p /etc/gdm3
if grep -q '^\[daemon\]' /etc/gdm3/custom.conf 2>/dev/null; then
  sed -i '/^\[daemon\]/a AutomaticLogin=$GuestUser' /etc/gdm3/custom.conf
  sed -i '/^\[daemon\]/a AutomaticLoginEnable=true' /etc/gdm3/custom.conf
else
  printf '[daemon]\nAutomaticLoginEnable=true\nAutomaticLogin=$GuestUser\n' >> /etc/gdm3/custom.conf
fi
# No "Welcome to Ubuntu" wizard on the first login.
install -d -o $GuestUser -g $GuestUser /home/$GuestUser/.config || true
echo yes > /home/$GuestUser/.config/gnome-initial-setup-done
chown $GuestUser`:$GuestUser /home/$GuestUser/.config/gnome-initial-setup-done || true
"@

function Get-ImageIndex([string]$Iso, [string]$Edition) {
    # "unattended detect" lists the editions in a Windows ISO's install.wim.
    # Lines look like: ImageIndex6="Windows 11 Pro (10.0.26300.9457 / x64 / en-US)"
    $out = Invoke-VBox unattended detect "--iso=$Iso" --machine-readable
    foreach ($line in $out) {
        if ($line -match '^ImageIndex(\d+)="(.+?) \(' -and $Matches[2] -eq $Edition) { return $Matches[1] }
    }
    throw "No '$Edition' image in $Iso. unattended detect said:`n$($out -join "`n")"
}

function Remove-InstallMedia([string]$Name) {
    # Unattended installs leave the OS ISO and their own answer-file ISO in
    # the DVD drives; empty them so the snapshots don't depend on either.
    $info = Invoke-VBox showvminfo $Name --machinereadable
    foreach ($line in $info) {
        if ($line -match '^"(.+)-(\d+)-(\d+)"="(.+\.(iso|viso))"$') {
            Invoke-VBox storageattach $Name --storagectl $Matches[1] --port $Matches[2] --device $Matches[3] --medium emptydrive | Out-Null
        }
    }
}

function Stop-Guest([string]$Name, [string]$Os) {
    if ($Os -eq "windows") {
        Invoke-Guest $Name "C:\Windows\System32\shutdown.exe" @("/s", "/t", "5") -AllowFailure | Out-Null
    } else {
        Invoke-Guest $Name "/bin/bash" @("-c", "printf '%s\n' '$GuestPassword' | sudo -S -p '' systemctl poweroff") -AllowFailure | Out-Null
    }
    try { Wait-PoweredOff $Name 300 } catch { Stop-Vm $Name }
}

function New-CleanVm([string]$Name, $Spec) {
    $iso = Get-ChildItem $IsoDir -Filter $Spec.Iso | Sort-Object Name | Select-Object -Last 1 -ExpandProperty FullName
    if (-not $iso) { throw "No ISO matching $($Spec.Iso) in $IsoDir" }
    Write-Host "[$Name] creating from $iso"
    Invoke-VBox createvm --name $Name --ostype $Spec.OsType --basefolder $BaseFolder --register --default | Out-Null
    Invoke-VBox modifyvm $Name --memory 4096 --cpus 2 --vram 128 --audio-enabled on --clipboard-mode disabled | Out-Null
    # Windows 10 on VirtualBox over Hyper-V (this PC): with the defaults
    # (BIOS, Hyper-V paravirtualisation) setup spins forever at "Getting
    # ready" or the boot spinner. EFI and the minimal provider install.
    if ($Spec.Firmware) { Invoke-VBox modifyvm $Name --firmware $Spec.Firmware | Out-Null }
    if ($Spec.Paravirt) { Invoke-VBox modifyvm $Name --paravirt-provider $Spec.Paravirt | Out-Null }
    $ctl = Get-VmInfo $Name "storagecontrollername0"
    $disk = Join-Path $BaseFolder "$Name\$Name.vdi"
    Invoke-VBox createmedium disk --filename $disk --size 65536 --format VDI --variant Standard | Out-Null
    Invoke-VBox storageattach $Name --storagectl $ctl --port 0 --device 0 --type hdd --medium $disk | Out-Null

    $a = @("unattended", "install", $Name, "--iso=$iso", "--user=$GuestUser", "--password=$GuestPassword",
        "--full-user-name=ReVLC QA", "--hostname=$Name.local", "--time-zone=UTC", "--locale=en_US", "--country=US",
        "--install-additions", "--start-vm=headless")
    if ($Spec.Os -eq "windows") {
        $a += "--image-index=$(Get-ImageIndex $iso $Spec.Edition)"
        $a += "--post-install-command=powershell.exe -NoProfile -ExecutionPolicy Bypass -EncodedCommand $(ConvertTo-EncodedCommand $WindowsPostInstall)"
    } else {
        $b64 = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($LinuxPostInstall -replace "`r", ""))
        $a += "--post-install-command=/bin/bash -c 'echo $b64 | base64 -d | /bin/bash'"
    }
    Start-Headless $Name { Invoke-VBox @a | Out-Null }
    $started = Get-Date

    if ($Spec.Os -eq "windows") {
        # The post-install step shuts Windows down when it's done.
        Wait-PoweredOff $Name (4 * 3600)
    } else {
        # The autoinstall reboots into the desktop; shut it down from there.
        Wait-GuestLogin $Name (3 * 3600)
        Stop-Guest $Name $Spec.Os
    }
    Write-Host "[$Name] installed in $([int]((Get-Date) - $started).TotalMinutes) min"
    Remove-InstallMedia $Name
    Invoke-VBox snapshot $Name take clean --description "Fresh install from $(Split-Path $iso -Leaf): Guest Additions, auto-logon as $GuestUser; nothing else." | Out-Null
    Write-Host "[$Name] snapshot clean taken"
}

function Add-Tools([string]$Name, $Spec) {
    Write-Host "[$Name] installing test tools"
    Invoke-VBox snapshot $Name restore clean | Out-Null
    Start-Headless $Name { Invoke-VBox startvm $Name --type headless | Out-Null }
    Wait-GuestLogin $Name 1800
    $hostOut = Join-Path $BaseFolder "$Name\tools"
    if ($Spec.Os -eq "windows") {
        $g = "C:\Users\$GuestUser\revlc-tools"
        Copy-ToGuest $Name "$qa\vm" $g
        Invoke-GuestDesktop $Name "powershell.exe -NoProfile -WindowStyle Minimized -ExecutionPolicy Bypass -File $g\vm\guest-tools-windows.ps1 -Out $g\out" "$g\out\done.txt" 3600
        Copy-FromGuest $Name "$g\out" $hostOut
        Invoke-Guest $Name "C:\Windows\System32\cmd.exe" @("/c", "rmdir", "/s", "/q", $g) -AllowFailure | Out-Null
    } else {
        $g = "/home/$GuestUser/revlc-tools"
        Copy-ToGuest $Name "$qa\vm" $g
        Invoke-Guest $Name "/bin/bash" @("$g/vm/guest-tools-linux.sh", "$g/out") -Env @("SUDO_PASS=$GuestPassword") -TimeoutSec 3600 -AllowFailure | Out-Null
        Copy-FromGuest $Name "$g/out" $hostOut
        Invoke-Guest $Name "/bin/rm" @("-rf", $g) -AllowFailure | Out-Null
    }
    $done = Get-Content (Join-Path $hostOut "out\done.txt") -ErrorAction SilentlyContinue
    if ($done -ne "ok") { throw "[$Name] installing the tools failed; see $hostOut\out" }
    Stop-Guest $Name $Spec.Os
    Invoke-VBox snapshot $Name take tools --description ((Get-Content (Join-Path $hostOut "out\tools.txt")) -join "`n") | Out-Null
    Write-Host "[$Name] snapshot tools taken; installed:"
    Get-Content (Join-Path $hostOut "out\tools.txt") | ForEach-Object { "  $_" }
}

foreach ($name in $Vm) {
    $spec = $VmSpecs[$name]
    if (-not $spec) { throw "Unknown VM $name (one of: $($VmSpecs.Keys -join ', '))" }
    if (-not $ToolsOnly) {
        if (Test-Vm $name) {
            if (-not $Recreate) { throw "$name exists; pass -Recreate to start over or -ToolsOnly to redo its tools snapshot" }
            Stop-Vm $name
            Invoke-VBox unregistervm $name --delete | Out-Null
        }
        New-CleanVm $name $spec
    } else {
        $snaps = Invoke-VBox snapshot $name list --machinereadable
        if ($snaps -match '="tools"$') { Invoke-VBox snapshot $name delete tools | Out-Null }
    }
    Add-Tools $name $spec
}
