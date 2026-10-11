# CI smoke test: launch one mode on the GitHub runner, drive it with real mouse/keyboard input, screenshot it,
# collect the logs. Functional evidence only: the runner is a VM with no real GPU and a virtual display; it says
# nothing about parity (frame pacing, DPI, tearing, GPU texture import).
param([string]$Mode, [string]$Extra = "", [string]$Exe = "pkg\spike719\spike719.exe", [string]$Out = "smoke")
$ErrorActionPreference = "Continue"
$name = ($Mode + $Extra).Replace("--", "_").Replace("=", "-").Replace(" ", "")
$dir = Join-Path $Out $name
New-Item -ItemType Directory -Force $dir | Out-Null
$notes = Join-Path $dir "notes.txt"
$prof = Join-Path $env:RUNNER_TEMP ("spike719-" + $name)
Remove-Item $prof -Recurse -Force -ErrorAction SilentlyContinue
$env:SPIKE719_CI = "1"
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type @"
using System; using System.Runtime.InteropServices;
public class W {
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X; public int Y; }
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, int data, UIntPtr extra);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
}
"@
function Shot($file) {
  try {
    $b = [System.Windows.Forms.SystemInformation]::VirtualScreen
    $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($b.Left, $b.Top, 0, 0, $bmp.Size)
    $bmp.Save($file, [System.Drawing.Imaging.ImageFormat]::Png)
  } catch { "screenshot failed: $_" | Add-Content $notes }
}
# SetCursorPos does not move the cursor in the runner's session (clicks landed at screen centre), so use absolute
# mouse_event moves (0x8001 = MOVE|ABSOLUTE, coordinates normalised to 0..65535), plus a 1px jiggle for hover.
function MoveMouse($x, $y) {
  $b = [System.Windows.Forms.SystemInformation]::VirtualScreen
  $nx = [int](([double]$x * 65535.0) / ($b.Width - 1)); $ny = [int](([double]$y * 65535.0) / ($b.Height - 1))
  [W]::mouse_event(0x8001, [uint32]$nx, [uint32]$ny, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 150
  [W]::mouse_event(0x8001, [uint32]([Math]::Min(65535, $nx + 40)), [uint32]$ny, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 100
  [W]::mouse_event(0x8001, [uint32]$nx, [uint32]$ny, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 300
  $pos = [System.Windows.Forms.Cursor]::Position
  "move -> wanted $([int]$x),$([int]$y) got $($pos.X),$($pos.Y)" | Add-Content $notes
}
function Click($x, $y, [string]$btn = "left") {
  MoveMouse $x $y
  if ($btn -eq "left") { [W]::mouse_event(0x2,0,0,0,[UIntPtr]::Zero); [W]::mouse_event(0x4,0,0,0,[UIntPtr]::Zero) }
  else { [W]::mouse_event(0x8,0,0,0,[UIntPtr]::Zero); [W]::mouse_event(0x10,0,0,0,[UIntPtr]::Zero) }
  Start-Sleep -Milliseconds 400
}
# Move once, then two quick clicks (the move helper takes ~0.5 s, which exceeds the 500 ms double-click window).
function DblClick($x, $y) {
  MoveMouse $x $y
  for ($i = 0; $i -lt 2; $i++) { [W]::mouse_event(0x2,0,0,0,[UIntPtr]::Zero); [W]::mouse_event(0x4,0,0,0,[UIntPtr]::Zero); Start-Sleep -Milliseconds 80 }
  Start-Sleep -Milliseconds 300
}

$args = @("--mode=$Mode", "--profile=$prof", "--size=960x640")
if ($Extra) { $args += $Extra.Split(" ") }
$p = Start-Process -FilePath $Exe -ArgumentList $args -PassThru
"started pid $($p.Id): $Exe $($args -join ' ')" | Add-Content $notes

# --- passive part: plain, modal (t=12..16 s), menu (t=18..22 s)
Start-Sleep -Seconds 10;  Shot (Join-Path $dir "1-plain.png")
Start-Sleep -Seconds 4;   Shot (Join-Path $dir "2-modal.png")
Start-Sleep -Seconds 5;   Shot (Join-Path $dir "3-menu.png")
Start-Sleep -Seconds 6;   Shot (Join-Path $dir "4-after.png")

# --- active part: real input
$rects = $null
$ev = Join-Path $prof "events.csv"
if (Test-Path $ev) {
  $line = Get-Content $ev | Where-Object { $_ -match "rects:" } | Select-Object -First 1
  if ($line) { $json = ($line -replace '^[^"]*"rects:', '' -replace '"$', '') -replace "'", '"'; try { $rects = $json | ConvertFrom-Json } catch { "rects parse failed: $json" | Add-Content $notes } }
}
$p.Refresh()
$h = $p.MainWindowHandle
"main window handle: $h  title: $($p.MainWindowTitle)" | Add-Content $notes
if ($rects -and $h -ne [IntPtr]::Zero) {
  [W]::SetForegroundWindow($h) | Out-Null; Start-Sleep -Milliseconds 500
  $o = New-Object W+POINT; [W]::ClientToScreen($h, [ref]$o) | Out-Null
  "client origin on screen: $($o.X),$($o.Y)  rects: $json" | Add-Content $notes
  $cx = { param($r, $dx, $dy) $o.X + $r.x + $dx }
  # 1. tooltip hanging off a toolbar button over the panes
  MoveMouse ($o.X + $rects.tip.x + $rects.tip.w/2) ($o.Y + $rects.tip.y + $rects.tip.h/2); Start-Sleep -Seconds 2
  Shot (Join-Path $dir "5-tooltip.png")
  # 2. click the first input in pane 0 and type
  Click ($o.X + $rects.s0.x + 60) ($o.Y + $rects.s0.y + 42)
  [System.Windows.Forms.SendKeys]::SendWait("hello 123"); Start-Sleep -Seconds 1
  Shot (Join-Path $dir "6-typed.png")
  # 3. select a word in pane 0 and open the context menu on it
  DblClick ($o.X + $rects.s0.x + 70) ($o.Y + $rects.s0.y + 82)
  Click ($o.X + $rects.s0.x + 70) ($o.Y + $rects.s0.y + 82) "right"; Start-Sleep -Seconds 1
  Shot (Join-Path $dir "7-contextmenu.png")
  # choose "Copy" (first item, just below-right of the click point) and read the clipboard back
  [System.Windows.Forms.Clipboard]::Clear()
  Click ($o.X + $rects.s0.x + 70 + 30) ($o.Y + $rects.s0.y + 82 + 12); Start-Sleep -Seconds 1
  try { "clipboard after context-menu Copy: [$([System.Windows.Forms.Clipboard]::GetText())]" | Add-Content $notes } catch { "clipboard read failed: $_" | Add-Content $notes }
  Shot (Join-Path $dir "7b-after-copy.png")
  [System.Windows.Forms.SendKeys]::SendWait("{ESC}"); Start-Sleep -Milliseconds 500
  # 4. wheel scroll inside pane 1
  MoveMouse ($o.X + $rects.s1.x + $rects.s1.w/2) ($o.Y + $rects.s1.y + $rects.s1.h/2)
  [W]::mouse_event(0x800,0,0,-360,[UIntPtr]::Zero); Start-Sleep -Milliseconds 300
  [W]::mouse_event(0x800,0,0,-360,[UIntPtr]::Zero); Start-Sleep -Seconds 1
  Shot (Join-Path $dir "8-scrolled.png")
  # 5. host-drawn popup: click the fixed "open popup" button in pane 0 (real user gesture), screenshot, then close it
  Click ($o.X + $rects.s0.x + 50) ($o.Y + $rects.s0.y + 74); Start-Sleep -Seconds 4
  Shot (Join-Path $dir "9-popup.png")
} else { "input phase skipped (rects or window handle missing)" | Add-Content $notes }

"alive at end: $(-not (Get-Process -Id $p.Id -ErrorAction SilentlyContinue).HasExited)" | Add-Content $notes
Get-Process | Where-Object { $_.ProcessName -like "spike719*" } | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 2
foreach ($f in "env.json", "spike.log", "events.csv", "frames.csv", "cpu.csv", "procs.csv", "cef.log") {
  $src = Join-Path $prof $f
  if (Test-Path $src) { Copy-Item $src $dir }
}
