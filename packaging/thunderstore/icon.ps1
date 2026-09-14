<#
.SYNOPSIS
    Draw packaging/thunderstore/icon.png, the 256x256 Thunderstore requires.

.DESCRIPTION
    The same mark the two windows wear: Mannaz, struck in worn brass on night,
    with the left edge burning the way the launcher's background does. Drawn
    from the same geometry as valhsync-ui's theme::rune rather than exported
    from a drawing program, so the icon cannot drift away from the window.

    Run it only when the mark itself changes; the PNG is committed.
#>
[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing

$size = 256
$bmp = New-Object System.Drawing.Bitmap($size, $size)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias

# The palette, from docs/palette.md.
$night = [System.Drawing.Color]::FromArgb(0x0F, 0x0D, 0x0B)
$gold = [System.Drawing.Color]::FromArgb(0xC7, 0xA4, 0x55)
$goldLit = [System.Drawing.Color]::FromArgb(0xE8, 0xCD, 0x8B)
$edge = [System.Drawing.Color]::FromArgb(0x4A, 0x3C, 0x27)

$g.Clear($night)

# The burning edge: bands down the left side, brightest at the very edge and
# falling away quadratically, exactly as theme::burning_edge paints it.
for ($i = 0; $i -lt 24; $i++) {
    $x = $i * 2.2
    $fall = [math]::Pow(1.0 - ($i / 24.0), 2)
    $alpha = [int](70 * $fall)
    if ($alpha -le 0) { continue }
    $brush = New-Object System.Drawing.SolidBrush(
        [System.Drawing.Color]::FromArgb($alpha, 0xE8, 0x7A, 0x2A))
    $g.FillRectangle($brush, [float]$x, 0, 2.4, $size)
    $brush.Dispose()
}

# A few embers, hugging that edge.
$embers = @(@(14, 196, 3.0), @(26, 150, 2.2), @(9, 98, 2.6), @(34, 62, 1.8), @(20, 28, 2.0))
foreach ($e in $embers) {
    $brush = New-Object System.Drawing.SolidBrush(
        [System.Drawing.Color]::FromArgb(150, 0xE8, 0xA6, 0x55))
    $g.FillEllipse($brush, [float]($e[0] - $e[2]), [float]($e[1] - $e[2]),
        [float]($e[2] * 2), [float]($e[2] * 2))
    $brush.Dispose()
}

# A carved border, so the tile has an edge of its own in a grid of tiles.
$pen = New-Object System.Drawing.Pen($edge, 3)
$g.DrawRectangle($pen, 6, 6, $size - 13, $size - 13)
$pen.Dispose()

# Mannaz, from theme::mannaz_strokes: the same four segments, same ratios.
$cx = 132.0
$cy = 128.0
$r = 74.0
$left = -0.60
$right = 0.60
$top = -0.95
$bottom = 0.95
$waist = 0.18
function Point-At([double]$x, [double]$y) {
    New-Object System.Drawing.PointF([float]($cx + $x * $r), [float]($cy + $y * $r))
}
$strokes = @(
    @((Point-At $left $top), (Point-At $left $bottom)),
    @((Point-At $right $top), (Point-At $right $bottom)),
    @((Point-At $left $top), (Point-At $right $waist)),
    @((Point-At $right $top), (Point-At $left $waist))
)

# Drawn twice: a wide, dim pass for the glow the window paints behind it, then
# the mark itself.
$glow = New-Object System.Drawing.Pen(
    [System.Drawing.Color]::FromArgb(40, $gold.R, $gold.G, $gold.B), 22)
$glow.StartCap = [System.Drawing.Drawing2D.LineCap]::Round
$glow.EndCap = [System.Drawing.Drawing2D.LineCap]::Round
foreach ($s in $strokes) { $g.DrawLine($glow, $s[0], $s[1]) }
$glow.Dispose()

$mark = New-Object System.Drawing.Pen($goldLit, 9)
$mark.StartCap = [System.Drawing.Drawing2D.LineCap]::Round
$mark.EndCap = [System.Drawing.Drawing2D.LineCap]::Round
foreach ($s in $strokes) { $g.DrawLine($mark, $s[0], $s[1]) }
$mark.Dispose()

$out = Join-Path $PSScriptRoot "icon.png"
$g.Dispose()
$bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
Write-Host "wrote $out ($size x $size)"
