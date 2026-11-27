# Runs one case of the probe from a session with no desktop: gives it a directory and an identity of its own, starts it
# through Explorer in the signed-in session, has it post, watches the banner, and prints what was logged. Issue #801.
param(
    [Parameter(Mandatory)][string]$Case,
    [string]$Label = '',
    [string]$Id = '',
    [string]$Mode = 'adopt',
    [string]$Name = '',
    [string]$Icon = '',
    [string]$Scheme = '',
    [switch]$Single,
    [switch]$Front,
    [string]$Launch = '',          # a shortcut to start the probe by; its own executable when empty
    [switch]$NoLaunch,             # the probe of this case is already running
    [string]$Post = 'post',        # post, post-button or none
    [string]$Action = 'none',      # what the watcher does to the banner: none, invoke or click
    [string]$Match = '',
    [string]$Kind = '',
    [switch]$Minimize,
    [switch]$Keep                  # leave the probe running afterwards
)
$root = Join-Path $env:TEMP 'shaman-801-20261127'
$run = "$root\run"; $dir = "$root\cases\$Case"
if (-not $Id) { $Id = "ai.shaman.probe801.$Case" }
if (-not $Label) { $Label = $Case }
$ps = 'powershell -NoProfile -ExecutionPolicy Bypass -File'
New-Item -ItemType Directory -Force -Path $dir | Out-Null
$exe = "$dir\rune-toast.exe"
if (-not (Test-Path $exe)) { Copy-Item "$root\src\target\release\rune-toast.exe" $exe }
$log = "$dir\probe.log"
$before = if (Test-Path $log) { (Get-Content $log).Count } else { 0 }
if (-not $NoLaunch) {
    @("mode=$Mode", "id=$Id", "name=$Name", "icon=$Icon", "scheme=$Scheme", "single=$($Single.ToString().ToLower())", "front=$($Front.ToString().ToLower())") | Set-Content -Encoding ASCII "$dir\probe.cfg"
    $target = if ($Launch) { $Launch } else { $exe }
    & "$run\it.ps1" -Task shaman-801-run -Exe 'explorer.exe' -Arguments "`"$target`"" -WaitSeconds 10 | Out-Null
    $deadline = (Get-Date).AddSeconds(20)
    while ((Get-Date) -lt $deadline) {
        if ((Test-Path $log) -and ((Get-Content $log | Select-Object -Skip $before) -match ' ready$')) { break }
        Start-Sleep -Milliseconds 300
    }
}
if ($Post -ne 'none') {
    Remove-Item "$run\$Label.json", "$run\$Label.png", "$run\$Label.watching" -ErrorAction SilentlyContinue
    $extra = ''
    if ($Match) { $extra += " -Match `"$Match`"" }
    if ($Kind) { $extra += " -Kind $Kind" }
    if ($Minimize) { $extra += ' -Minimize rune-toast' }
    & "$run\it.ps1" -Task shaman-801-obs -Exe 'conhost.exe' -Arguments "--headless $ps `"$run\observe.ps1`" -Out `"$run`" -Label $Label -Wait 14 -Action $Action$extra" | Out-Null
    $deadline = (Get-Date).AddSeconds(40)
    while ((Get-Date) -lt $deadline -and -not (Test-Path "$run\$Label.watching")) { Start-Sleep -Milliseconds 200 }
    "$Post n-$Label $Label title" | Set-Content -Encoding ASCII "$dir\probe.cmd.txt"
    while ((Get-Date) -lt $deadline -and (Test-Path "$dir\probe.cmd.txt")) { Start-Sleep -Milliseconds 100 }
    $deadline = (Get-Date).AddSeconds(30)
    while ((Get-Date) -lt $deadline -and -not (Test-Path "$run\$Label.json")) { Start-Sleep -Milliseconds 300 }
    if (Test-Path "$run\$Label.json") {
        $j = Get-Content "$run\$Label.json" -Raw | ConvertFrom-Json
        "banner found: $($j.found)$(if ($j.error) { ' ERROR ' + $j.error })"
        if ($j.tree) { $j.tree | Where-Object { $_ -match 'NormalToastView|AppLogo|SenderName|Title|MessageText|Button name="(?!Settings|Move)' } }
        if ($j.acted) { "foreground before: $($j.foregroundBefore)"; "app before: $($j.appBefore)"; "acted on: $($j.acted) $($j.clickedAt)"; Start-Sleep -Seconds 1; "foreground after: $($j.foregroundAfter)"; "app after: $($j.appAfter)" }
    } else { 'the watcher wrote nothing' }
}
'--- log'
if (Test-Path $log) { Get-Content $log | Select-Object -Skip $before } else { 'no log' }
'--- windows'
& "$run\history.ps1" -AppId $Id
if (-not $Keep) { 'quit' | Set-Content -Encoding ASCII "$dir\probe.cmd.txt"; Start-Sleep -Milliseconds 800 }
