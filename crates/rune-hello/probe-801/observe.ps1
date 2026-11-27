# Looks at the desktop from inside the signed-in session: waits for a toast banner (or opens the notification
# center), writes down what UI Automation reads of it, keeps a picture cropped to it, and can press it. Issue #801.
param(
    [Parameter(Mandatory)][string]$Out,
    [string]$Label = 'obs',
    [int]$Wait = 10,
    [ValidateSet('banner', 'center')][string]$Target = 'banner',
    [ValidateSet('none', 'invoke', 'click')][string]$Action = 'none',
    [string]$Match = '',
    [int]$After = 3
)
$ErrorActionPreference = 'Stop'
$result = [ordered]@{ label = $Label; target = $Target; action = $Action; started = (Get-Date).ToString('o') }
try {
    Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Drawing, System.Windows.Forms
    Add-Type @"
using System; using System.Runtime.InteropServices;
public static class N801 {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, UIntPtr e);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, System.Text.StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
}
"@
    [N801]::SetProcessDPIAware() | Out-Null
    $AE = [System.Windows.Automation.AutomationElement]
    $walker = [System.Windows.Automation.TreeWalker]::RawViewWalker

    function Foreground {
        $h = [N801]::GetForegroundWindow()
        $sb = New-Object System.Text.StringBuilder 512
        [N801]::GetWindowText($h, $sb, 512) | Out-Null
        $procId = 0
        [N801]::GetWindowThreadProcessId($h, [ref]$procId) | Out-Null
        $name = (Get-Process -Id $procId -ErrorAction SilentlyContinue).ProcessName
        "{0} (pid {1}, {2})" -f $sb.ToString(), $procId, $name
    }
    function Describe($el, $depth, $list) {
        if ($depth -gt 12 -or $list.Count -gt 400) { return }
        $c = $el.Current
        $r = $c.BoundingRectangle
        $list.Add(('{0}{1} name="{2}" id="{3}" class="{4}" rect={5},{6},{7},{8}' -f ('  ' * $depth), $c.ControlType.ProgrammaticName.Replace('ControlType.', ''), $c.Name, $c.AutomationId, $c.ClassName, [int]$r.X, [int]$r.Y, [int]$r.Width, [int]$r.Height)) | Out-Null
        $child = $walker.GetFirstChild($el)
        while ($child) { Describe $child ($depth + 1) $list; $child = $walker.GetNextSibling($child) }
    }
    function Find-All($el, $list) {
        $list.Add($el) | Out-Null
        $child = $walker.GetFirstChild($el)
        while ($child) { Find-All $child $list; $child = $walker.GetNextSibling($child) }
    }
    function Shot($rect, $path) {
        $w = [int]$rect.Width; $h = [int]$rect.Height
        if ($w -le 0 -or $h -le 0) { return }
        $bmp = New-Object System.Drawing.Bitmap $w, $h
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        $g.CopyFromScreen([int]$rect.X, [int]$rect.Y, 0, 0, $bmp.Size)
        $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
        $g.Dispose(); $bmp.Dispose()
    }
    function Core-Windows {
        $cond = New-Object System.Windows.Automation.PropertyCondition($AE::ClassNameProperty, 'Windows.UI.Core.CoreWindow')
        $AE::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children, $cond)
    }

    $result.foregroundBefore = Foreground
    if ($Target -eq 'center') {
        # Win+N opens the notification center.
        [N801]::keybd_event(0x5B, 0, 0, [UIntPtr]::Zero); [N801]::keybd_event(0x4E, 0, 0, [UIntPtr]::Zero)
        [N801]::keybd_event(0x4E, 0, 2, [UIntPtr]::Zero); [N801]::keybd_event(0x5B, 0, 2, [UIntPtr]::Zero)
    }
    $wanted = if ($Target -eq 'banner') { 'New notification' } else { 'Notification Center' }
    $found = $null
    $deadline = (Get-Date).AddSeconds($Wait)
    $seen = @()
    while ((Get-Date) -lt $deadline -and -not $found) {
        foreach ($w in (Core-Windows)) {
            $n = $w.Current.Name
            if ($seen -notcontains $n) { $seen += $n }
            if ($n -eq $wanted) { $found = $w }
        }
        if (-not $found) { Start-Sleep -Milliseconds 150 }
    }
    $result.coreWindowsSeen = $seen
    $result.found = [bool]$found
    if ($found) {
        Start-Sleep -Milliseconds 900   # let it finish sliding in
        $lines = New-Object System.Collections.Generic.List[string]
        Describe $found 0 $lines
        $result.tree = $lines
        $rect = $found.Current.BoundingRectangle
        Shot $rect (Join-Path $Out "$Label.png")
        if ($Action -ne 'none') {
            $all = New-Object System.Collections.Generic.List[object]
            Find-All $found $all
            $pick = $null
            foreach ($el in $all) {
                $c = $el.Current
                if ($Match -and ($c.Name -notlike "*$Match*")) { continue }
                if (-not $Match -and $el -eq $found) { continue }
                $pattern = $null
                if ($Action -eq 'invoke' -and -not $el.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$pattern)) { continue }
                $pick = $el; break
            }
            if ($pick) {
                $c = $pick.Current
                $result.acted = ('{0} name="{1}" id="{2}"' -f $c.ControlType.ProgrammaticName, $c.Name, $c.AutomationId)
                if ($Action -eq 'invoke') {
                    $pattern = $pick.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
                    $pattern.Invoke()
                } else {
                    $r = $c.BoundingRectangle
                    $x = [int]($r.X + $r.Width / 2); $y = [int]($r.Y + $r.Height / 2)
                    $result.clickedAt = "$x,$y"
                    [N801]::SetCursorPos($x, $y) | Out-Null
                    Start-Sleep -Milliseconds 150
                    [N801]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero); [N801]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero)
                }
                Start-Sleep -Seconds $After
                $result.foregroundAfter = Foreground
            } else {
                $result.acted = 'nothing matched'
            }
        }
    }
    if ($Target -eq 'center' -and $Action -eq 'none') {
        [N801]::keybd_event(0x1B, 0, 0, [UIntPtr]::Zero); [N801]::keybd_event(0x1B, 0, 2, [UIntPtr]::Zero)
    }
} catch {
    $result.error = $_.ToString() + ' @ ' + $_.InvocationInfo.PositionMessage
}
$result.finished = (Get-Date).ToString('o')
$result | ConvertTo-Json -Depth 5 | Set-Content -Encoding UTF8 (Join-Path $Out "$Label.json")
