# Sets the display scale through Windows Settings, for DPI checks. Record the
# current scale first and set it back afterwards (125% on the test PC).
#
#   Start-Process ms-settings:display; Start-Sleep 2
#   powershell -ExecutionPolicy Bypass -File scripts\qa\windows\set-scale.ps1 -Scale 150
param([Parameter(Mandatory)][int]$Scale)
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$A = [Windows.Automation.AutomationElement]
$byName = New-Object Windows.Automation.PropertyCondition($A::NameProperty, 'Settings')
$win = $A::RootElement.FindFirst([Windows.Automation.TreeScope]::Children, $byName)
if (-not $win) { throw 'Windows Settings window not found; open ms-settings:display first' }
$byType = { param($t) New-Object Windows.Automation.PropertyCondition($A::ControlTypeProperty, $t) }
$combos = $win.FindAll([Windows.Automation.TreeScope]::Descendants, (& $byType ([Windows.Automation.ControlType]::ComboBox)))
$scaleBox = @($combos | Where-Object { $_.Current.Name -eq 'Scale' })[0]
if (-not $scaleBox) { throw 'Scale box not found' }
$scaleBox.GetCurrentPattern([Windows.Automation.ExpandCollapsePattern]::Pattern).Expand()
Start-Sleep -Milliseconds 150
$items = $win.FindAll([Windows.Automation.TreeScope]::Descendants, (& $byType ([Windows.Automation.ControlType]::ListItem)))
$item = @($items | Where-Object { $_.Current.Name -in "$Scale%", "$Scale% (Recommended)" })[0]
if (-not $item) { throw "Scale option $Scale% not found" }
$item.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
Write-Output "Requested display scale $Scale%"
