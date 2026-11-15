# Types text into the app window. Refuses to send keystrokes unless the app is the
# foreground window, so stray input can never land in another program.
param([Parameter(Mandatory)][string]$Text, [string]$Title = "Skyrim Mod Manager")
Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System; using System.Runtime.InteropServices;
public class T {
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
}
"@
if (Get-Process -Name SkyrimSE, skse64_loader -ErrorAction SilentlyContinue) {
  Write-Error "Skyrim is running; refusing to inject keyboard input"; exit 1
}
$p = Get-Process | Where-Object { $_.MainWindowTitle -eq $Title } | Select-Object -First 1
if (-not $p) { Write-Error "window not found"; exit 1 }
$h = $p.MainWindowHandle
[T]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 300
if ([T]::GetForegroundWindow() -ne $h) { Write-Error "app is not the foreground window; not typing"; exit 1 }
# Escape SendKeys metacharacters so the text is sent literally.
$escaped = ($Text.ToCharArray() | ForEach-Object { if ('+^%~(){}[]' -contains $_) { "{$_}" } else { "$_" } }) -join ''
[System.Windows.Forms.SendKeys]::SendWait($escaped)
Start-Sleep -Milliseconds 200
