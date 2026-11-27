# Opens the notification center in the signed-in session, says which of the probe's toasts stand in it, and can press
# one. Prints only the lines that name the probe, since the center holds the person's own notifications too. Issue #801.
param([Parameter(Mandatory)][string]$Label, [string]$Press = '', [string]$Action = 'click', [string]$Kind = 'Text', [string[]]$Logs = @(), [switch]$Quiet)
$root = Join-Path $env:TEMP 'shaman-801-20261127'; $run = "$root\run"
$ps = 'powershell -NoProfile -ExecutionPolicy Bypass -File'
Remove-Item "$run\$Label.json", "$run\$Label.png", "$run\$Label.watching" -ErrorAction SilentlyContinue
$counts = @{}; foreach ($l in $Logs) { $counts[$l] = if (Test-Path $l) { (Get-Content $l).Count } else { 0 } }
$extra = if ($Press) { " -Action $Action -Match `"$Press`" -Kind $Kind -After 5" } else { '' }
& "$run\it.ps1" -Task shaman-801-obs -Exe 'conhost.exe' -Arguments "--headless $ps `"$run\observe.ps1`" -Out `"$run`" -Label $Label -Target center -Wait 10$extra" | Out-Null
$deadline = (Get-Date).AddSeconds(45)
while ((Get-Date) -lt $deadline -and -not (Test-Path "$run\$Label.json")) { Start-Sleep -Milliseconds 300 }
if (Test-Path "$run\$Label.json") {
    $j = Get-Content "$run\$Label.json" -Raw | ConvertFrom-Json
    "center found: $($j.found)$(if ($j.error) { ' ERROR ' + $j.error })  (elements read: $(@($j.tree).Count))"
    if (-not $Quiet) { @($j.tree) | Where-Object { $_ -match '^\s*(ListItem|Group) name="[^"]*(Shaman|probe|title|PowerShell|Control )' } | ForEach-Object { $_.Trim() } }
    if ($j.acted) { "foreground before: $($j.foregroundBefore)"; "acted on: $($j.acted) $($j.clickedAt) ($($j.how); patterns: $($j.patterns -join ', '))"; "foreground after: $($j.foregroundAfter); center still open: $($j.centerOpenAfter)" }
} else { 'the watcher wrote nothing' }
Start-Sleep -Seconds 2
foreach ($l in $Logs) { "--- new in $l"; if (Test-Path $l) { Get-Content $l | Select-Object -Skip $counts[$l] } }
"--- probes running: " + (@(Get-Process rune-toast -ErrorAction SilentlyContinue | ForEach-Object { "$($_.Id) $($_.Path)" }) -join '; ')
