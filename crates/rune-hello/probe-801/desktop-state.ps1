# Says what stands between a toast and the screen on this desktop: do not disturb (focus assist), and every top-level
# window UI Automation sees, by class and name. Issue #801.
param([Parameter(Mandatory)][string]$Out, [string]$Label = 'state', [int]$Delay = 0, [string]$Shots = '')
$ShotAt = @($Shots -split ',' | Where-Object { $_ } | ForEach-Object { [double]$_ })
$r = [ordered]@{}
try {
    Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Drawing, System.Windows.Forms
    Add-Type @"
using System; using System.Runtime.InteropServices;
public static class Wnf801 {
  [DllImport("shell32.dll")] public static extern int SHQueryUserNotificationState(out int state);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("ntdll.dll")] public static extern int NtQueryWnfStateData(ref ulong name, IntPtr type, IntPtr scope, out uint stamp, byte[] buffer, ref uint size);
  public static int Quiet() { ulong n = 0xD83063EA3BF1C75UL; uint stamp; byte[] b = new byte[4]; uint size = 4;
    int s = NtQueryWnfStateData(ref n, IntPtr.Zero, IntPtr.Zero, out stamp, b, ref size); return s != 0 ? -1 : BitConverter.ToInt32(b, 0); }
}
"@
    [Wnf801]::SetProcessDPIAware() | Out-Null
    # 1 not present, 2 busy, 3 a full-screen Direct3D app, 4 presentation mode, 5 accepts notifications, 6 quiet time, 7 a Store app
    $state = 0; [Wnf801]::SHQueryUserNotificationState([ref]$state) | Out-Null; $r.userNotificationState = $state
    $r.focusAssist = [Wnf801]::Quiet()   # 0 off, 1 priority only, 2 alarms only, -1 could not be read
    # Every top-level window seen at any moment over $Delay seconds, with when it was first seen.
    $root = [System.Windows.Automation.AutomationElement]::RootElement
    $seen = [ordered]@{}
    $start = Get-Date
    do {
        foreach ($w in $root.FindAll('Children', [System.Windows.Automation.Condition]::TrueCondition)) {
            try {
                $c = $w.Current; $b = $c.BoundingRectangle
                $key = '{0} | {1} | pid {2} ({3})' -f $c.ClassName, $c.Name, $c.ProcessId, (Get-Process -Id $c.ProcessId -ErrorAction SilentlyContinue).ProcessName
                if (-not $seen.Contains($key)) { $seen[$key] = ('+{0:n1}s {1},{2},{3},{4}' -f ((Get-Date) - $start).TotalSeconds, [int]$b.X, [int]$b.Y, [int]$b.Width, [int]$b.Height) }
            } catch {}
        }
        $elapsed = ((Get-Date) - $start).TotalSeconds
        foreach ($at in $ShotAt) {
            $path = Join-Path $Out ("$Label-at-$at.png")
            if ($elapsed -ge $at -and -not (Test-Path $path)) {
                # The corner a banner slides into: the bottom right of the primary screen, above the taskbar.
                $area = [System.Windows.Forms.Screen]::PrimaryScreen.WorkingArea
                $w = [Math]::Min(1000, $area.Width); $h = [Math]::Min(520, $area.Height)
                $bmp = New-Object System.Drawing.Bitmap $w, $h
                $g = [System.Drawing.Graphics]::FromImage($bmp)
                $g.CopyFromScreen($area.Right - $w, $area.Bottom - $h, 0, 0, $bmp.Size)
                $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png); $g.Dispose(); $bmp.Dispose()
            }
        }
        Start-Sleep -Milliseconds 200
    } while (((Get-Date) - $start).TotalSeconds -lt $Delay)
    $r.windows = @($seen.GetEnumerator() | ForEach-Object { "$($_.Value) | $($_.Key)" })
    $r.session = (Get-Process -Id $PID).SessionId
} catch { $r.error = $_.ToString() }
$r | ConvertTo-Json -Depth 4 | Set-Content -Encoding UTF8 (Join-Path $Out "$Label.json")
