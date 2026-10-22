# Desktop helpers for Windows QA runs; see docs/audit-playbook.md.
# Dot-source it in each PowerShell command:
#
#   $env:QA_RUN = "$env:USERPROFILE\revlc-qa\win-<date>"
#   . scripts\qa\windows\qa-desktop.ps1; Focus-ReVLC; Shot '12-what-it-shows'
#
# Shot <name>              whole virtual screen, DPI-aware (shows video frames)
# Shot-Window <el> <name>  one window via PrintWindow (e.g. the uninstaller)
# Record <name> [seconds]  silent ffmpeg gdigrab clip into recordings\
# Focus-ReVLC, Click <x> <y>, Keys <SendKeys string>
# Eval-Js <js>             runs JS in the WebView (cdp.mjs)
# Nsis-Inspect, Nsis-Click <automationId>   drive the NSIS installer
# Input-Desktop            True when the session can take input (not locked)
#
# ffmpeg: $env:QA_FFMPEG, else ffmpeg on PATH, else the essentials build
# under %USERPROFILE%\revlc-qa\tools or
# %USERPROFILE%\revlc-qa\win-*\tools. Don't install it with winget (it timed out).
if (-not $env:QA_RUN) { throw 'Set $env:QA_RUN to the run folder' }
$qaRoot = $env:QA_RUN
$qaScripts = $PSScriptRoot
New-Item -ItemType Directory -Force -Path "$qaRoot\screenshots", "$qaRoot\recordings", "$qaRoot\logs" | Out-Null
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
if (-not ('QAWin' -as [type])) {
  Add-Type @"
using System;
using System.Runtime.InteropServices;
public class QAWin {
 [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
 [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
 [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
 [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
 [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, UIntPtr e);
 [DllImport("user32.dll")] public static extern void keybd_event(byte v, byte s, uint f, UIntPtr e);
 [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hWnd);
 [DllImport("user32.dll")] public static extern IntPtr OpenInputDesktop(uint f, bool inherit, uint access);
 [DllImport("user32.dll")] public static extern bool CloseDesktop(IntPtr h);
 [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint f);
}
"@
}
# Without this, captures and cursor coordinates are scaled wrongly at 125%+.
[QAWin]::SetProcessDPIAware() | Out-Null

function Input-Desktop {
  $desk = [QAWin]::OpenInputDesktop(0, $false, 0x100)
  if ($desk -eq [IntPtr]::Zero) { return $false }
  [QAWin]::CloseDesktop($desk) | Out-Null
  return $true
}

function Shot([string]$name) {
  if (-not (Input-Desktop)) { throw "Session locked or disconnected; refused $name" }
  $bounds = [Windows.Forms.SystemInformation]::VirtualScreen
  $bmp = New-Object Drawing.Bitmap $bounds.Width, $bounds.Height
  $g = [Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($bounds.Location, [Drawing.Point]::Empty, $bounds.Size)
  $path = Join-Path "$qaRoot\screenshots" "$name.png"
  $bmp.Save($path, [Drawing.Imaging.ImageFormat]::Png)
  $g.Dispose(); $bmp.Dispose()
  Write-Output $path
}

function Shot-Window([Windows.Automation.AutomationElement]$win, [string]$name) {
  $r = $win.Current.BoundingRectangle
  $bmp = New-Object Drawing.Bitmap ([int]$r.Width), ([int]$r.Height)
  $g = [Drawing.Graphics]::FromImage($bmp); $dc = $g.GetHdc()
  $ok = [QAWin]::PrintWindow([IntPtr]$win.Current.NativeWindowHandle, $dc, 2)
  $g.ReleaseHdc($dc); $bmp.Save("$qaRoot\screenshots\$name.png"); $g.Dispose(); $bmp.Dispose()
  Write-Output "Window capture $name succeeded=$ok"
}

function Focus-ReVLC {
  $p = Get-Process revlc -ErrorAction Stop | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
  [QAWin]::ShowWindow($p.MainWindowHandle, 9) | Out-Null
  [QAWin]::SetForegroundWindow($p.MainWindowHandle) | Out-Null
}

function Click([int]$x, [int]$y) {
  [QAWin]::SetCursorPos($x, $y) | Out-Null
  [QAWin]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero)
  [QAWin]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero)
}

function Keys([string]$keys) { [Windows.Forms.SendKeys]::SendWait($keys) }

function Find-FFmpeg {
  if ($env:QA_FFMPEG) { return $env:QA_FFMPEG }
  $onPath = Get-Command ffmpeg.exe -ErrorAction SilentlyContinue
  if ($onPath) { return $onPath.Source }
  $found = Get-ChildItem "$env:USERPROFILE\revlc-qa\tools", "$env:USERPROFILE\revlc-qa\win-*\tools" -Filter ffmpeg.exe -Recurse -ErrorAction SilentlyContinue | Select-Object -First 1
  if ($found) { return $found.FullName }
  throw 'ffmpeg not found: set $env:QA_FFMPEG'
}

function Record([string]$name, [int]$seconds = 25) {
  $ff = Find-FFmpeg
  Start-Process -FilePath $ff -WindowStyle Hidden -PassThru `
    -ArgumentList @('-y', '-f', 'gdigrab', '-framerate', '12', '-i', 'desktop', '-t', "$seconds", '-an', '-c:v', 'libx264', '-preset', 'ultrafast', '-crf', '25', '-pix_fmt', 'yuv420p', "$qaRoot\recordings\$name.mp4") `
    -RedirectStandardError "$qaRoot\logs\record-$name.log" | Select-Object Id
}

function Eval-Js([string]$script) {
  $tmp = Join-Path $qaRoot 'logs\current-expression.js'
  [IO.File]::WriteAllText($tmp, $script, [Text.UTF8Encoding]::new($false))
  node "$qaScripts\cdp.mjs" --file $tmp
}

function Nsis-Window {
  [Windows.Automation.AutomationElement]::RootElement.FindAll([Windows.Automation.TreeScope]::Children, [Windows.Automation.Condition]::TrueCondition) |
    Where-Object { $_.Current.Name.Trim() -in 'ReVLC Setup', 'ReVLC Uninstall' } | Select-Object -First 1
}

function Nsis-Click([string]$id) {
  $condition = New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::AutomationIdProperty, $id)
  $b = (Nsis-Window).FindFirst([Windows.Automation.TreeScope]::Descendants, $condition)
  if (-not $b) { throw "Installer control $id missing" }
  $r = $b.Current.BoundingRectangle
  Click ([int]($r.X + $r.Width / 2)) ([int]($r.Y + $r.Height / 2))
}

function Nsis-Inspect {
  (Nsis-Window).FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.Condition]::TrueCondition) |
    ForEach-Object { [pscustomobject]@{ name = $_.Current.Name; id = $_.Current.AutomationId; type = $_.Current.ControlType.ProgrammaticName; rect = $_.Current.BoundingRectangle.ToString() } } |
    ConvertTo-Json -Depth 3
}
