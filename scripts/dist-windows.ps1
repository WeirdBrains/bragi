# Release build of the Windows desktop shell and the terminal app into dist/ for the host architecture
# (x86_64 or aarch64):
#   bragi-VER-TRIPLE.exe        the standalone exe
#   bragi-tui-VER-TRIPLE.exe    the terminal app's standalone exe (a console program)
#   bragi-VER-TRIPLE.zip        both exes with LICENSE.txt and README.md
#   bragi-VER-TRIPLE.msi        per-user installer (WiX 7, packaging/windows/bragi.wxs)
#   bragi-VER-TRIPLE-setup.exe  the same installer as a WiX 7 Burn bundle (packaging/windows/bundle.wxs)
# Needs wix.exe 7.x on PATH with WixToolset.UI.wixext and WixToolset.BootstrapperApplications.wixext in the
# global cache. WiX 7 is used under the Open Source Maintenance Fee EULA v1.1, accepted by WeirdBrains on
# 2026-11-12 (docs/RELEASES.md), so every wix command passes -acceptEula wix7. Nothing is code-signed.
# With BRAGI_FFMPEG_DIR (ffmpeg.exe and ffprobe.exe from scripts/build-ffmpeg.sh, e.g. in MSYS2), both go
# beside bragi.exe in the zip and the installer, with their licence, build config and source offer.
# BRAGI_FEATURES (default none) adds Cargo features to both binaries, e.g. `on-device` (the llama.cpp engine;
# not turned on in the Release workflow for Windows yet).
param([string]$WixVersion = "7.0.1")
$ErrorActionPreference = "Stop"
$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $root

$meta = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
$ver = ($meta.packages | Where-Object { $_.name -eq "bragi-desktop" }).version
$targetDir = $meta.target_directory
switch ($env:PROCESSOR_ARCHITECTURE) {
  "AMD64" { $triple = "x86_64-pc-windows-msvc"; $wixArch = "x64" }
  "ARM64" { $triple = "aarch64-pc-windows-msvc"; $wixArch = "arm64" }
  default { throw "unknown architecture $env:PROCESSOR_ARCHITECTURE" }
}
$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE ".cargo" }
$env:RUSTFLAGS = "$env:RUSTFLAGS --remap-path-prefix=$root=C:\build\bragi --remap-path-prefix=$cargoHome=C:\cargo --remap-path-prefix=$env:USERPROFILE=C:\Users\user".Trim()
$featureArgs = @()
if ($env:BRAGI_FEATURES) {
  $featureArgs = @("--features", (($env:BRAGI_FEATURES -split "[ ,]+" | Where-Object { $_ } | ForEach-Object { "bragi-desktop/$_,bragi-tui/$_" }) -join ","))
}
cargo build --release --locked -p bragi-desktop -p bragi-tui @featureArgs
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$name = "bragi-$ver-$triple"
$dist = Join-Path $root "dist"
New-Item -ItemType Directory -Force -Path $dist | Out-Null
$stage = Join-Path ([IO.Path]::GetTempPath()) ("bragi-dist-" + [guid]::NewGuid())
$src = Join-Path $stage "src"
New-Item -ItemType Directory -Force -Path $src | Out-Null
try {
  Copy-Item (Join-Path $targetDir "release\bragi.exe") (Join-Path $src "bragi.exe")
  Copy-Item (Join-Path $targetDir "release\bragi-tui.exe") (Join-Path $src "bragi-tui.exe")
  Copy-Item (Join-Path $root "LICENSE") (Join-Path $src "LICENSE.txt")
  Copy-Item (Join-Path $root "README.md") (Join-Path $src "README.md")
  Copy-Item (Join-Path $root "brand\png\windows\bragi.ico") (Join-Path $src "bragi.ico")
  Copy-Item (Join-Path $root "THIRD_PARTY_NOTICES.md") (Join-Path $src "THIRD_PARTY_NOTICES.md")
  $ffmpeg = 0
  $zipFiles = @((Join-Path $src "bragi.exe"), (Join-Path $src "bragi-tui.exe"), (Join-Path $src "LICENSE.txt"), (Join-Path $src "README.md"), (Join-Path $src "THIRD_PARTY_NOTICES.md"))
  if ($env:BRAGI_FFMPEG_DIR) {
    foreach ($f in "bin\ffmpeg.exe", "bin\ffprobe.exe") { Copy-Item (Join-Path $env:BRAGI_FFMPEG_DIR $f) $src }
    Copy-Item (Join-Path $env:BRAGI_FFMPEG_DIR "LICENSE.md") (Join-Path $src "FFMPEG-LICENSE.md")
    Copy-Item (Join-Path $env:BRAGI_FFMPEG_DIR "CONFIG.txt") (Join-Path $src "FFMPEG-CONFIG.txt")
    Copy-Item (Join-Path $env:BRAGI_FFMPEG_DIR "SOURCE.txt") (Join-Path $src "FFMPEG-SOURCE.txt")
    $zipFiles += "ffmpeg.exe", "ffprobe.exe", "FFMPEG-LICENSE.md", "FFMPEG-CONFIG.txt", "FFMPEG-SOURCE.txt" | ForEach-Object { Join-Path $src $_ }
    $ffmpeg = 1
  }

  Copy-Item (Join-Path $src "bragi.exe") (Join-Path $dist "$name.exe") -Force
  Copy-Item (Join-Path $src "bragi-tui.exe") (Join-Path $dist "bragi-tui-$ver-$triple.exe") -Force
  $zipDir = Join-Path $stage $name
  New-Item -ItemType Directory -Force -Path $zipDir | Out-Null
  Copy-Item $zipFiles $zipDir
  Compress-Archive -Path $zipDir -DestinationPath (Join-Path $dist "$name.zip") -Force

  # The installer's license pane needs RTF; made from LICENSE so the two never drift.
  $lines = foreach ($line in ((Get-Content -Raw (Join-Path $root "LICENSE")) -replace "`r", "") -split "`n") {
    $line.Replace('\', '\\').Replace('{', '\{').Replace('}', '\}')
  }
  $rtf = Join-Path $stage "License.rtf"
  [IO.File]::WriteAllText($rtf, "{\rtf1\ansi\deff0{\fonttbl{\f0\fswiss Segoe UI;}}\f0\fs18 " + ($lines -join "\par`r`n") + "}", [Text.Encoding]::ASCII)

  $wix = (Get-Command wix.exe).Source
  $have = (& $wix --version).Trim()
  if (-not $have.StartsWith($WixVersion)) { throw "wix.exe is $have, expected $WixVersion" }
  $msi = Join-Path $dist "$name.msi"
  & $wix build -nologo -acceptEula wix7 -arch $wixArch -ext "WixToolset.UI.wixext/$WixVersion" `
    -d "Source=$src" -d "ProductVersion=$ver" -d "LicenseRtf=$rtf" -d "Ffmpeg=$ffmpeg" -culture en-US -pdbtype none `
    -o $msi (Join-Path $root "packaging\windows\bragi.wxs")
  if ($LASTEXITCODE -ne 0) { throw "wix build (msi) failed: $LASTEXITCODE" }
  & $wix build -nologo -acceptEula wix7 -arch $wixArch -ext "WixToolset.BootstrapperApplications.wixext/$WixVersion" `
    -d "Source=$src" -d "ProductVersion=$ver" -d "LicenseRtf=$rtf" -d "Msi=$msi" -d "Ffmpeg=$ffmpeg" -pdbtype none `
    -o (Join-Path $dist "$name-setup.exe") (Join-Path $root "packaging\windows\bundle.wxs")
  if ($LASTEXITCODE -ne 0) { throw "wix build (bundle) failed: $LASTEXITCODE" }
} finally {
  Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
}
Get-ChildItem $dist | Format-Table Name, Length
