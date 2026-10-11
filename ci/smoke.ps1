# CI smoke test: launch one mode on the GitHub runner, screenshot it, collect the logs. Functional evidence only:
# the runner is a VM with no real GPU and a virtual display; it says nothing about parity (frame pacing, DPI, etc.).
param([string]$Mode, [string]$Extra = "", [string]$Exe = "pkg\spike719\spike719.exe", [string]$Out = "smoke")
$ErrorActionPreference = "Continue"
$name = ($Mode + $Extra).Replace("--", "_").Replace("=", "-").Replace(" ", "")
$dir = Join-Path $Out $name
New-Item -ItemType Directory -Force $dir | Out-Null
$prof = Join-Path $env:RUNNER_TEMP ("spike719-" + $name)
Remove-Item $prof -Recurse -Force -ErrorAction SilentlyContinue
$env:SPIKE719_CI = "1"
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
function Shot($file) {
  try {
    $b = [System.Windows.Forms.SystemInformation]::VirtualScreen
    $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($b.Left, $b.Top, 0, 0, $bmp.Size)
    $bmp.Save($file, [System.Drawing.Imaging.ImageFormat]::Png)
    "screen $($b.Width)x$($b.Height) -> $file" | Add-Content (Join-Path $dir "notes.txt")
  } catch { "screenshot failed: $_" | Add-Content (Join-Path $dir "notes.txt") }
}
$args = @("--mode=$Mode", "--profile=$prof", "--size=960x640")
if ($Extra) { $args += $Extra.Split(" ") }
$p = Start-Process -FilePath $Exe -ArgumentList $args -PassThru
"started pid $($p.Id): $Exe $($args -join ' ')" | Add-Content (Join-Path $dir "notes.txt")
Start-Sleep -Seconds 10;  Shot (Join-Path $dir "1-plain.png")
Start-Sleep -Seconds 4;   Shot (Join-Path $dir "2-modal.png")
Start-Sleep -Seconds 5;   Shot (Join-Path $dir "3-menu.png")
Start-Sleep -Seconds 6;   Shot (Join-Path $dir "4-after.png")
"alive at end: $(-not (Get-Process -Id $p.Id -ErrorAction SilentlyContinue).HasExited)" | Add-Content (Join-Path $dir "notes.txt")
Get-Process | Where-Object { $_.ProcessName -like "spike719*" } | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 2
foreach ($f in "env.json", "spike.log", "events.csv", "frames.csv", "cpu.csv", "procs.csv", "cef.log") {
  $src = Join-Path $prof $f
  if (Test-Path $src) { Copy-Item $src $dir }
}
