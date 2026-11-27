# Posts one toast under a given AppUserModelID straight through WinRT, with nothing of Rune's: the control that says
# whether this desktop shows banners at all. Issue #801.
param([Parameter(Mandatory)][string]$AppId, [string]$Title = 'Control toast', [string]$Log = '')
$ErrorActionPreference = 'Stop'
try {
    [Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null
    [Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime] | Out-Null
    $doc = New-Object Windows.Data.Xml.Dom.XmlDocument
    $doc.LoadXml("<toast><visual><binding template=`"ToastGeneric`"><text>$Title</text><text>Fix the build</text></binding></visual><audio silent=`"true`"/></toast>")
    $toast = New-Object Windows.UI.Notifications.ToastNotification $doc
    [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier($AppId).Show($toast)
    $line = "posted under $AppId"
} catch { $line = "failed under ${AppId}: $_" }
if ($Log) { Add-Content -Path $Log -Value $line } else { $line }
