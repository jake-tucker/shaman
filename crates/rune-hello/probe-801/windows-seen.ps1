# Lists every visible top-level window that appears over some seconds, by FindWindowEx, which also walks the bands a
# banner and the notification center stand in. Used to learn how this build of Windows presents a banner. Issue #801.
param([Parameter(Mandatory)][string]$Out, [string]$Label = 'windows', [int]$Seconds = 12)
Add-Type @"
using System; using System.Runtime.InteropServices; using System.Text;
public static class W801 {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowEx(IntPtr parent, IntPtr after, string cls, string title);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
}
"@
[W801]::SetProcessDPIAware() | Out-Null
$seen = [ordered]@{}
$start = Get-Date
do {
    $h = [IntPtr]::Zero
    while ($true) {
        $h = [W801]::FindWindowEx([IntPtr]::Zero, $h, [NullString]::Value, [NullString]::Value)
        if ($h -eq [IntPtr]::Zero) { break }
        if (-not [W801]::IsWindowVisible($h)) { continue }
        $r = New-Object W801+RECT; [W801]::GetWindowRect($h, [ref]$r) | Out-Null
        if (($r.R - $r.L) -le 0 -or ($r.B - $r.T) -le 0) { continue }
        $c = New-Object System.Text.StringBuilder 256; [W801]::GetClassName($h, $c, 256) | Out-Null
        $t = New-Object System.Text.StringBuilder 256; [W801]::GetWindowText($h, $t, 256) | Out-Null
        $procId = 0; [W801]::GetWindowThreadProcessId($h, [ref]$procId) | Out-Null
        $key = '{0} | {1} | {2} | {3},{4} {5}x{6}' -f $c, $t, (Get-Process -Id $procId -ErrorAction SilentlyContinue).ProcessName, $r.L, $r.T, ($r.R - $r.L), ($r.B - $r.T)
        if (-not $seen.Contains($key)) { $seen[$key] = '+{0:n1}s' -f ((Get-Date) - $start).TotalSeconds }
    }
    Start-Sleep -Milliseconds 200
} while (((Get-Date) - $start).TotalSeconds -lt $Seconds)
@($seen.GetEnumerator() | ForEach-Object { "$($_.Value) | $($_.Key)" }) | Set-Content -Encoding UTF8 (Join-Path $Out "$Label.txt")
