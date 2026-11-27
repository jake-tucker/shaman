# Looks at the desktop from inside the signed-in session: waits for a toast banner (or opens the notification
# center), writes down what UI Automation reads of it, keeps a picture cropped to it, and can press it. Issue #801.
param(
    [Parameter(Mandatory)][string]$Out,
    [string]$Label = 'obs',
    [int]$Wait = 10,
    [ValidateSet('banner', 'center')][string]$Target = 'banner',
    [ValidateSet('none', 'invoke', 'click')][string]$Action = 'none',
    [string]$Match = '',
    [string]$Kind = '',
    [string]$Minimize = '',
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
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowEx(IntPtr parent, IntPtr after, string cls, string title);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
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
        # An element can leave while the walk is on it (a list still sliding in): skip it and go on.
        try {
            $c = $el.Current
            $r = $c.BoundingRectangle
            $where = if ($r.IsEmpty) { 'off screen' } else { '{0},{1},{2},{3}' -f [int]$r.X, [int]$r.Y, [int]$r.Width, [int]$r.Height }
            $list.Add(('{0}{1} name="{2}" id="{3}" class="{4}" rect={5}' -f ('  ' * $depth), $c.ControlType.ProgrammaticName.Replace('ControlType.', ''), $c.Name, $c.AutomationId, $c.ClassName, $where)) | Out-Null
            $child = $walker.GetFirstChild($el)
        } catch { return }
        while ($child) { Describe $child ($depth + 1) $list; try { $child = $walker.GetNextSibling($child) } catch { $child = $null } }
    }
    function Find-All($el, $list) {
        $list.Add($el) | Out-Null
        try { $child = $walker.GetFirstChild($el) } catch { return }
        while ($child) { Find-All $child $list; try { $child = $walker.GetNextSibling($child) } catch { $child = $null } }
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
        # A banner and the notification center stand in a band of windows UI Automation's root does not list, so they
        # are found by walking the top-level windows with FindWindowEx and asked for by handle.
        $h = [IntPtr]::Zero
        while ($true) {
            $h = [N801]::FindWindowEx([IntPtr]::Zero, $h, 'Windows.UI.Core.CoreWindow', [NullString]::Value)
            if ($h -eq [IntPtr]::Zero) { break }
            if (-not [N801]::IsWindowVisible($h)) { continue }
            try { $AE::FromHandle($h) } catch {}
        }
    }

    if ($Minimize) {
        # Take the app's window out of the foreground first, so that what a press does to it can be told.
        foreach ($p in (Get-Process -Name $Minimize -ErrorAction SilentlyContinue)) {
            if ($p.MainWindowHandle -ne [IntPtr]::Zero) { [N801]::ShowWindowAsync($p.MainWindowHandle, 6) | Out-Null }
        }
        Start-Sleep -Milliseconds 700
    }
    function App-Windows {
        if (-not $Minimize) { return $null }
        @(Get-Process -Name $Minimize -ErrorAction SilentlyContinue | ForEach-Object {
            'pid {0} window {1} minimized {2}' -f $_.Id, $_.MainWindowHandle, [N801]::IsIconic($_.MainWindowHandle) })
    }
    $result.foregroundBefore = Foreground
    $result.appBefore = App-Windows
    function Center-Open {
        # The center's window keeps its name for a moment after it shuts, so it is open only when it also holds a list.
        foreach ($w in (Core-Windows)) {
            if ($w.Current.Name -like 'Notification Cent*') {
                $l = New-Object System.Collections.Generic.List[string]
                Describe $w 0 $l
                if ($l.Count -gt 20) { return $true }
            }
        }
        return $false
    }
    if ($Target -eq 'center') {
        for ($try = 0; $try -lt 3 -and -not (Center-Open); $try++) {
            # Win+N opens the notification center, and closes one that is open.
            [N801]::keybd_event(0x5B, 0, 0, [UIntPtr]::Zero); [N801]::keybd_event(0x4E, 0, 0, [UIntPtr]::Zero)
            [N801]::keybd_event(0x4E, 0, 2, [UIntPtr]::Zero); [N801]::keybd_event(0x5B, 0, 2, [UIntPtr]::Zero)
            Start-Sleep -Milliseconds 2500
        }
    }
    if ($Target -eq 'banner') {
        # A banner from an earlier case may still be up: wait it out, then say the watch has begun.
        $clear = (Get-Date).AddSeconds(15)
        while ((Get-Date) -lt $clear -and (Core-Windows | Where-Object { $_.Current.Name -eq 'New notification' })) { Start-Sleep -Milliseconds 300 }
    }
    Set-Content -Path (Join-Path $Out "$Label.watching") -Value 'watching'
    $wanted = if ($Target -eq 'banner') { 'New notification' } else { 'Notification Cent*' }
    $found = $null
    $deadline = (Get-Date).AddSeconds($Wait)
    $seen = @()
    while ((Get-Date) -lt $deadline -and -not $found) {
        foreach ($w in (Core-Windows)) {
            $n = $w.Current.Name
            if ($seen -notcontains $n) { $seen += $n }
            if ($n -like $wanted) { $found = $w }
        }
        if (-not $found) { Start-Sleep -Milliseconds 150 }
    }
    $result.coreWindowsSeen = $seen
    $result.found = [bool]$found
    if ($found) {
        Start-Sleep -Milliseconds $(if ($Target -eq 'center') { 2000 } else { 900 })   # let it finish sliding in
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
                try { $c = $el.Current; [void]$c.Name } catch { continue }
                if ($Match -and ($c.Name -notlike "*$Match*")) { continue }
                if ($c.BoundingRectangle.IsEmpty) { continue }
                if ($Kind -and ($c.ControlType.ProgrammaticName -ne "ControlType.$Kind")) { continue }
                if (-not $Match -and $el -eq $found) { continue }
                $pattern = $null
                $pick = $el; break
            }
            if ($pick) {
                $c = $pick.Current
                $result.acted = ('{0} name="{1}" id="{2}"' -f $c.ControlType.ProgrammaticName, $c.Name, $c.AutomationId)
                $result.patterns = @($pick.GetSupportedPatterns() | ForEach-Object { $_.ProgrammaticName })
                $pattern = $null
                if ($Action -eq 'invoke' -and $pick.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$pattern)) {
                    $result.how = 'invoked'
                    $pattern.Invoke()
                } else {
                    $result.how = 'pressed with the pointer'
                    $r = $c.BoundingRectangle
                    $x = [int]($r.X + $r.Width / 2); $y = [int]($r.Y + $r.Height / 2)
                    $result.clickedAt = "$x,$y"
                    [N801]::SetCursorPos($x, $y) | Out-Null
                    Start-Sleep -Milliseconds 150
                    [N801]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero); [N801]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero)
                }
                Start-Sleep -Seconds $After
                $result.foregroundAfter = Foreground
                $result.centerOpenAfter = [bool](Core-Windows | Where-Object { $_.Current.Name -like 'Notification Cent*' })
                $result.appAfter = App-Windows
            } else {
                $result.acted = 'nothing matched'
            }
        }
    }
    if ($Target -eq 'center' -and (Center-Open)) {
        [N801]::keybd_event(0x5B, 0, 0, [UIntPtr]::Zero); [N801]::keybd_event(0x4E, 0, 0, [UIntPtr]::Zero)
        [N801]::keybd_event(0x4E, 0, 2, [UIntPtr]::Zero); [N801]::keybd_event(0x5B, 0, 2, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 1200
        $result.centerShutAtEnd = -not (Center-Open)
    }
} catch {
    $result.error = $_.ToString() + ' @ ' + $_.InvocationInfo.PositionMessage
}
$result.finished = (Get-Date).ToString('o')
$result | ConvertTo-Json -Depth 5 | Set-Content -Encoding UTF8 (Join-Path $Out "$Label.json")
