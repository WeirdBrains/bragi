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
# BRAGI_FEATURES (default `on-device`, the optional on-device model's llama.cpp engine, B4) is the Cargo
# features both binaries get; BRAGI_FEATURES=none builds without. llama.cpp is built for a generic CPU of the
# architecture (GGML_NATIVE=OFF; weirdnet-model refuses to load on an x86 CPU without what the build needs).
# x86_64 builds it with MSVC. ARM64 builds it with clang-cl and Ninja from Visual Studio (ggml's CPU backend
# needs GGML_NATIVE=ON under MSVC proper on ARM, which would tie the exe to the build machine's CPU), in a
# Visual Studio developer shell this script enters. Its __FILE__ paths are trimmed (MSVC: /d1trimfile;
# clang-cl: -fmacro-prefix-map) so no build machine path ends up in the exe.
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
$features = if ($env:BRAGI_FEATURES) { $env:BRAGI_FEATURES } else { "on-device" }
$features = @($features -split "[ ,]+" | Where-Object { $_ -and $_ -ne "none" })
$featureArgs = @()
if ($features.Count -gt 0) {
  $featureArgs = @("--features", (($features | ForEach-Object { "bragi-desktop/$_,bragi-tui/$_" }) -join ","))
}
if ($features -contains "on-device") {
  # llama.cpp's sources come from the cargo registry and its build from the target dir; both end up in
  # __FILE__ (ggml's asserts), so both are trimmed, with the checkout and the user profile.
  $trim = @($cargoHome, $targetDir, $root, $env:USERPROFILE) | ForEach-Object { $_.TrimEnd('\') + '\' }
  if ($triple -like "aarch64-*") {
    $vs = & "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe" -latest -products * -property installationPath
    if (-not $vs) { throw "Visual Studio not found (vswhere)" }
    Import-Module (Join-Path $vs "Common7\Tools\Microsoft.VisualStudio.DevShell.dll")
    Enter-VsDevShell -VsInstallPath $vs -DevCmdArguments "-arch=arm64 -host_arch=arm64" -SkipAutomaticLocation | Out-Null
    $clang = (Get-Command clang-cl.exe -ErrorAction SilentlyContinue).Source
    if (-not $clang) {
      $clang = @((Join-Path $env:ProgramFiles "LLVM\bin\clang-cl.exe"), (Join-Path $vs "VC\Tools\Llvm\ARM64\bin\clang-cl.exe")) |
        Where-Object { Test-Path $_ } | Select-Object -First 1
    }
    if (-not $clang) { throw "clang-cl not found (LLVM, or Visual Studio's C++ Clang tools)" }
    if (-not (Get-Command ninja.exe -ErrorAction SilentlyContinue)) { throw "ninja not found in the developer shell" }
    & $clang --version
    $env:CC_aarch64_pc_windows_msvc = $clang
    $env:CXX_aarch64_pc_windows_msvc = $clang
    $env:CMAKE_GENERATOR = "Ninja"
    $maps = ($trim | ForEach-Object { "-Xclang -fmacro-prefix-map=$_=" }) -join " "
  } else {
    $maps = ($trim | ForEach-Object { "/d1trimfile:$_" }) -join " "
  }
  $env:CFLAGS = "$env:CFLAGS $maps".Trim()
  $env:CXXFLAGS = "$env:CXXFLAGS $maps".Trim()
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
