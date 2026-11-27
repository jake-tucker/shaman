# Runs one command in the signed-in user's desktop session, from a session that has no desktop (SSH), by a scheduled
# task that runs only while that user is logged on. Issue #801.
param(
    [Parameter(Mandatory)][string]$Exe,
    [string]$Arguments = '',
    [int]$WaitSeconds = 0,
    [string]$Task = 'shaman-801-run'
)
$ErrorActionPreference = 'Stop'
$user = [System.Security.Principal.WindowsIdentity]::GetCurrent().Name
$action = if ($Arguments) { New-ScheduledTaskAction -Execute $Exe -Argument $Arguments } else { New-ScheduledTaskAction -Execute $Exe }
$principal = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Limited
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit (New-TimeSpan -Minutes 10)
Register-ScheduledTask -TaskName $Task -Action $action -Principal $principal -Settings $settings -Force | Out-Null
Start-ScheduledTask -TaskName $Task
$deadline = (Get-Date).AddSeconds($WaitSeconds)
while ((Get-Date) -lt $deadline) {
    Start-Sleep -Milliseconds 300
    if ((Get-ScheduledTask -TaskName $Task).State -ne 'Running') { break }
}
$info = Get-ScheduledTaskInfo -TaskName $Task
"task ${Task}: state $((Get-ScheduledTask -TaskName $Task).State), last result $($info.LastTaskResult)"
if ($WaitSeconds -gt 0) { Unregister-ScheduledTask -TaskName $Task -Confirm:$false }
