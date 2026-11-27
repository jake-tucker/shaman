# Says what the notification center holds for an AppUserModelID, and what Windows has recorded about that id.
param([Parameter(Mandatory)][string]$AppId)
[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null
try {
    $h = [Windows.UI.Notifications.ToastNotificationManager]::History.GetHistory($AppId)
    "history for ${AppId}: $($h.Count) toast(s)"
    foreach ($t in $h) { "  tag=$($t.Tag) group=$($t.Group) text=$($t.Content.GetElementsByTagName('text') | ForEach-Object { $_.InnerText })" }
} catch { "history for ${AppId}: failed: $_" }
$k = "HKCU:\Software\Classes\AppUserModelId\$AppId"
if (Test-Path $k) { "registry key: " + ((Get-ItemProperty $k | Select-Object * -ExcludeProperty PS* | Out-String).Trim()) } else { "registry key: none" }
$s = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Notifications\Settings\$AppId"
if (Test-Path $s) { "notification settings: " + ((Get-ItemProperty $s | Select-Object * -ExcludeProperty PS* | Out-String).Trim()) } else { "notification settings: none" }
