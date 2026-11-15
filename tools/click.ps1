# Clicks at a position relative to the app window's top-left (in physical pixels).
# -Scroll N scrolls N wheel notches down (negative = up) at the position instead of clicking.
#
# Mouse input is global: if the app can't be brought to the front (a fullscreen game
# keeps focus), the clicks and wheel land in whatever *is* in front. So this refuses to
# run while the game is up, and again unless the app really is the foreground window.
param([int]$X, [int]$Y, [int]$Scroll = 0, [string]$Title = "Skyrim Mod Manager")
Add-Type @"
using System; using System.Runtime.InteropServices;
public class C {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, UIntPtr e);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@
[C]::SetProcessDPIAware() | Out-Null
if (Get-Process -Name SkyrimSE, skse64_loader -ErrorAction SilentlyContinue) {
  Write-Error "Skyrim is running; refusing to inject mouse input"; exit 1
}
$p = Get-Process | Where-Object { $_.MainWindowTitle -eq $Title } | Select-Object -First 1
if (-not $p) { Write-Error "window not found"; exit 1 }
$h = $p.MainWindowHandle
[C]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 200
if ([C]::GetForegroundWindow() -ne $h) { Write-Error "app is not the foreground window; not clicking"; exit 1 }
$r = New-Object C+RECT
[C]::GetWindowRect($h, [ref]$r) | Out-Null
[C]::SetCursorPos($r.L + $X, $r.T + $Y) | Out-Null
Start-Sleep -Milliseconds 100
if ($Scroll -ne 0) {
  for ($i = 0; $i -lt [Math]::Abs($Scroll); $i++) {
    $d = [BitConverter]::ToUInt32([BitConverter]::GetBytes([int](-120 * [Math]::Sign($Scroll))), 0)
    [C]::mouse_event(0x800, 0, 0, $d, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 40
  }
  Start-Sleep -Milliseconds 300
  exit 0
}
[C]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero)
Start-Sleep -Milliseconds 60
[C]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero)
Start-Sleep -Milliseconds 400
