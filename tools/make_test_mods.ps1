# Builds a few fake mod archives with valid TES4 plugin headers for testing.
param([string]$Out = "$env:TEMP\smm-test-mods")
New-Item -ItemType Directory -Force $Out | Out-Null

function Sub([string]$type, [byte[]]$data) {
  $b = [System.Text.Encoding]::ASCII.GetBytes($type)
  $len = [BitConverter]::GetBytes([uint16]$data.Length)
  return $b + $len + $data
}
function ZStr([string]$s) { return [System.Text.Encoding]::ASCII.GetBytes($s) + [byte[]]@(0) }

function Plugin([string]$path, [uint32]$flags, [string[]]$masters) {
  $body = @()
  $body += Sub "HEDR" ([BitConverter]::GetBytes([single]1.7) + [BitConverter]::GetBytes([uint32]1) + [BitConverter]::GetBytes([uint32]2048))
  $body += Sub "CNAM" (ZStr "Test")
  foreach ($m in $masters) {
    $body += Sub "MAST" (ZStr $m)
    $body += Sub "DATA" ([byte[]]::new(8))
  }
  $body += Sub "INTV" ([BitConverter]::GetBytes([uint32]1))
  $hdr = [System.Text.Encoding]::ASCII.GetBytes("TES4") + [BitConverter]::GetBytes([uint32]$body.Length) + [BitConverter]::GetBytes($flags) + [byte[]]::new(8) + [BitConverter]::GetBytes([uint16]44) + [byte[]]::new(2)
  [System.IO.File]::WriteAllBytes($path, [byte[]]($hdr + $body))
}

function Build([string]$name, [scriptblock]$fill) {
  $work = Join-Path $Out "work-$name"
  Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
  New-Item -ItemType Directory -Force $work | Out-Null
  & $fill $work
  $zip = Join-Path $Out "$name.zip"
  Remove-Item $zip -ErrorAction SilentlyContinue
  Compress-Archive -Path "$work\*" -DestinationPath $zip
  Remove-Item -Recurse -Force $work
  Write-Output $zip
}

# 1. Nexus-style name, wrapped in a Data folder, ESP with one master
Build "Immersive Citizens - AI Overhaul-8429-0-4-0-3" {
  param($d)
  New-Item -ItemType Directory -Force "$d\Data\meshes\ic" | Out-Null
  New-Item -ItemType Directory -Force "$d\Data\textures\ic" | Out-Null
  Set-Content "$d\Data\meshes\ic\house.nif" "nif"
  Set-Content "$d\Data\textures\ic\wall.dds" ("x" * 5000)
  Plugin "$d\Data\Immersive Citizens - AI Overhaul.esp" 0 @("Skyrim.esm", "Update.esm")
}

# 2. Plain layout, ESL-flagged plugin
Build "Better_Dialogue_Controls_v1.2" {
  param($d)
  New-Item -ItemType Directory -Force "$d\interface" | Out-Null
  Set-Content "$d\interface\dialoguemenu.swf" "swf"
  Plugin "$d\BetterDialogueControls.esp" 0x200 @("Skyrim.esm")
}

# 3. Missing master + a texture that overrides mod 1
Build "SkyUI_5_2_SE-12604-5-2SE" {
  param($d)
  New-Item -ItemType Directory -Force "$d\textures\ic" | Out-Null
  Set-Content "$d\textures\ic\wall.dds" "override"
  Plugin "$d\SkyUI_SE.esp" 0 @("Skyrim.esm", "Nonexistent.esm")
}

# 4. Nested wrapper folder, no plugin at all (texture pack)
Build "Noble Skyrim Textures-1234-2-0" {
  param($d)
  New-Item -ItemType Directory -Force "$d\Noble Skyrim\textures\architecture" | Out-Null
  Set-Content "$d\Noble Skyrim\textures\architecture\stone.dds" ("s" * 20000)
}
