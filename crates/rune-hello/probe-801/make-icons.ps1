# Draws two plain icons that cannot be mistaken for each other: an orange PNG for a registry key's IconUri and a blue
# ICO for a shortcut, so a toast's picture says which one Windows took. Issue #801.
param([Parameter(Mandatory)][string]$Out)
Add-Type -AssemblyName System.Drawing
function Draw($color, $letter, $path) {
    $bmp = New-Object System.Drawing.Bitmap 256, 256
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.Clear($color)
    $font = New-Object System.Drawing.Font 'Segoe UI', 120, ([System.Drawing.FontStyle]::Bold)
    $g.DrawString($letter, $font, [System.Drawing.Brushes]::White, 40, 20)
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
}
Draw ([System.Drawing.Color]::FromArgb(230, 120, 20)) 'R' (Join-Path $Out 'orange.png')
Draw ([System.Drawing.Color]::FromArgb(30, 90, 220)) 'S' (Join-Path $Out 'blue.png')
# An .ico holding the blue PNG as its one 256-pixel image.
$png = [System.IO.File]::ReadAllBytes((Join-Path $Out 'blue.png'))
$ms = New-Object System.IO.MemoryStream
$w = New-Object System.IO.BinaryWriter $ms
$w.Write([uint16]0); $w.Write([uint16]1); $w.Write([uint16]1)
$w.Write([byte]0); $w.Write([byte]0); $w.Write([byte]0); $w.Write([byte]0)
$w.Write([uint16]1); $w.Write([uint16]32); $w.Write([uint32]$png.Length); $w.Write([uint32]22)
$w.Write($png); $w.Flush()
[System.IO.File]::WriteAllBytes((Join-Path $Out 'blue.ico'), $ms.ToArray())
