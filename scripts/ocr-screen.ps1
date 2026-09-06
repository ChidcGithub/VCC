param([int]$X, [int]$Y, [int]$W, [int]$H)
$ErrorActionPreference = 'Stop'
# KEEP THIS FILE PURE ASCII (no Chinese comments!).
# PS5.1 reads BOM-less files as GBK: a Chinese char at end-of-line can be a GBK
# lead byte that swallows the following LF, merging the next line into the
# comment (the Add-Type below silently vanished this way once).
# Make the whole process DPI aware: powershell.exe is DPI-unaware by default,
# so on scaled displays (125%/150%) CopyFromScreen coords get virtualized.
# X/Y/W/H come from the PMv2-aware host and are physical pixels.
Add-Type -Namespace Native -Name Dpi -MemberDefinition '[DllImport("user32.dll")] public static extern bool SetProcessDPIAware();'
[void][Native.Dpi]::SetProcessDPIAware()
# Redirected stdout defaults to OEM codepage (GBK): Chinese OCR text would be
# garbled by the host's from_utf8_lossy. Force UTF-8 on the output pipes.
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
Add-Type -AssemblyName System.Runtime.WindowsRuntime
Add-Type -AssemblyName System.Windows.Forms, System.Drawing

# WinRT IAsyncOperation -> .NET Task bridge
$asTaskGeneric = ([System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object {
    $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and
    $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1' })[0]
function Await($Op, [Type]$ResultType) {
    $asTask = $asTaskGeneric.MakeGenericMethod($ResultType)
    $netTask = $asTask.Invoke($null, @($Op))
    try {
        $netTask.Wait(-1) | Out-Null
    } catch {
        # Unwrap AggregateException so the real WinRT error reaches stderr
        throw $_.Exception.InnerException
    }
    $netTask.Result
}

[void][Windows.Media.Ocr.OcrEngine, Windows.Foundation, ContentType = WindowsRuntime]
[void][Windows.Globalization.Language, Windows.Foundation, ContentType = WindowsRuntime]
[void][Windows.Graphics.Imaging.BitmapDecoder, Windows.Foundation, ContentType = WindowsRuntime]
[void][Windows.Storage.StorageFile, Windows.Foundation, ContentType = WindowsRuntime]
[void][Windows.Storage.Streams.IRandomAccessStream, Windows.Foundation, ContentType = WindowsRuntime]

# Partial screen capture
$b = New-Object System.Drawing.Bitmap $W, $H
$g = [System.Drawing.Graphics]::FromImage($b)
$g.CopyFromScreen($X, $Y, 0, 0, (New-Object System.Drawing.Size($W, $H)))
$g.Dispose()
# PID-unique capture path: concurrent runs (tests or rapid tool calls) must not
# clobber each other's PNG (one process' finally-delete would break the other)
$tmp = Join-Path $env:TEMP ("vcc-ocr-capture-{0}.png" -f $PID)
$b.Save($tmp, [System.Drawing.Imaging.ImageFormat]::Png)
$b.Dispose()

try {
    $file = Await ([Windows.Storage.StorageFile]::GetFileFromPathAsync($tmp)) ([Windows.Storage.StorageFile])
    $stream = Await ($file.OpenAsync([Windows.Storage.FileAccessMode]::Read)) ([Windows.Storage.Streams.IRandomAccessStream])
    $decoder = Await ([Windows.Graphics.Imaging.BitmapDecoder]::CreateAsync($stream)) ([Windows.Graphics.Imaging.BitmapDecoder])
    $bmp = Await ($decoder.GetSoftwareBitmapAsync()) ([Windows.Graphics.Imaging.SoftwareBitmap])

    # Prefer a Chinese language pack, then user default languages
    $engine = $null
    foreach ($l in [Windows.Media.Ocr.OcrEngine]::AvailableRecognizerLanguages) {
        if ($l.LanguageTag -like 'zh*') { $engine = [Windows.Media.Ocr.OcrEngine]::TryCreateFromLanguage($l); break }
    }
    if (-not $engine) { $engine = [Windows.Media.Ocr.OcrEngine]::TryCreateFromUserProfileLanguages() }
    if (-not $engine) { Write-Output '__NO_OCR__'; exit 2 }

    $result = Await ($engine.RecognizeAsync($bmp)) ([Windows.Media.Ocr.OcrResult])
    Write-Output $result.Text
    exit 0
} finally {
    Remove-Item $tmp -ErrorAction SilentlyContinue
}
