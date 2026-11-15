# Presses the mouse at one position in the app window, drags it to another in small steps,
# and lets go. -Shot saves a screenshot while the button is still held, so the drag
# feedback can be checked. Positions are physical pixels from the window's top-left.
#
# Same safety rules as click.ps1: refuses to run while the game is up, or unless the app
# really is the foreground window, since injected input lands wherever the cursor is.
param([int]$X, [int]$Y, [int]$X2, [int]$Y2, [string]$Shot = "", [int]$Steps = 20, [string]$Title = "Skyrim Mod Manager")
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System; using System.Runtime.InteropServices;
public class D {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, UIntPtr e);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, IntPtr pid);
  [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool attach);
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@
[D]::SetProcessDPIAware() | Out-Null
if (Get-Process -Name SkyrimSE, skse64_loader -ErrorAction SilentlyContinue) {
  Write-Error "Skyrim is running; refusing to inject mouse input"; exit 1
}
$p = Get-Process | Where-Object { $_.MainWindowTitle -eq $Title } | Select-Object -First 1
if (-not $p) { Write-Error "window not found"; exit 1 }
$h = $p.MainWindowHandle
[D]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 200
if ([D]::GetForegroundWindow() -ne $h) {
  # Windows only lets the thread that owns the foreground window hand it over (a freshly
  # launched app loses out to the taskbar); borrow that thread's input queue and ask again.
  $fg = [D]::GetWindowThreadProcessId([D]::GetForegroundWindow(), [IntPtr]::Zero)
  $me = [D]::GetCurrentThreadId()
  [D]::AttachThreadInput($fg, $me, $true) | Out-Null
  [D]::SetForegroundWindow($h) | Out-Null
  [D]::AttachThreadInput($fg, $me, $false) | Out-Null
  Start-Sleep -Milliseconds 300
}
if ([D]::GetForegroundWindow() -ne $h) { Write-Error "app is not the foreground window; not dragging"; exit 1 }
$r = New-Object D+RECT
[D]::GetWindowRect($h, [ref]$r) | Out-Null
[D]::SetCursorPos($r.L + $X, $r.T + $Y) | Out-Null
Start-Sleep -Milliseconds 100
[D]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero)
Start-Sleep -Milliseconds 80
for ($i = 1; $i -le $Steps; $i++) {
  $cx = [int]($X + ($X2 - $X) * $i / $Steps)
  $cy = [int]($Y + ($Y2 - $Y) * $i / $Steps)
  [D]::SetCursorPos($r.L + $cx, $r.T + $cy) | Out-Null
  Start-Sleep -Milliseconds 25
}
if ($Shot -ne "") {
  Start-Sleep -Milliseconds 250
  $w = $r.R - $r.L; $hh = $r.B - $r.T
  $bmp = New-Object System.Drawing.Bitmap $w, $hh
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $hdc = $g.GetHdc()
  [D]::PrintWindow($h, $hdc, 2) | Out-Null
  $g.ReleaseHdc($hdc)
  $bmp.Save($Shot, [System.Drawing.Imaging.ImageFormat]::Png)
  Write-Output "saved $Shot"
}
[D]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero)
Start-Sleep -Milliseconds 400
