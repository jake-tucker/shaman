# Takes every toast of one AppUserModelID out of the notification center.
param([Parameter(Mandatory)][string]$AppId)
[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null
try { [Windows.UI.Notifications.ToastNotificationManager]::History.Clear($AppId); "cleared $AppId" } catch { "could not clear ${AppId}: $_" }
