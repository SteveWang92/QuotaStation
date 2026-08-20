param(
    [Parameter(Mandatory = $true)]
    [string]$Source,
    [Parameter(Mandatory = $true)]
    [string]$Output,
    [Parameter(Mandatory = $true)]
    [string]$Sizes
)

Add-Type -AssemblyName System.Drawing
New-Item -ItemType Directory -Force -Path $Output | Out-Null

$sourceImage = [System.Drawing.Image]::FromFile($Source)
try {
    foreach ($sizeText in $Sizes.Split(',')) {
        $size = [int]$sizeText
        $bitmap = New-Object System.Drawing.Bitmap($size, $size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
        $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
        try {
            $graphics.Clear([System.Drawing.Color]::Transparent)
            $graphics.CompositingMode = [System.Drawing.Drawing2D.CompositingMode]::SourceCopy
            $graphics.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
            $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
            $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
            $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
            $graphics.DrawImage($sourceImage, [System.Drawing.Rectangle]::new(0, 0, $size, $size))
            $bitmap.Save(
                (Join-Path $Output "$size.png"),
                [System.Drawing.Imaging.ImageFormat]::Png
            )
        }
        finally {
            $graphics.Dispose()
            $bitmap.Dispose()
        }
    }
}
finally {
    $sourceImage.Dispose()
}
