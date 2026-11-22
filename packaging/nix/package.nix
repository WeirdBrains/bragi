# Bragi from the GitHub release tarball (prebuilt, no Rust), patched for Nix.
# `desktop = true` keeps the desktop entry and icons; `false` is the standalone form with only bin/bragi.
{ lib, stdenv, fetchurl, autoPatchelfHook, makeWrapper
, libxkbcommon, libxcb, libX11, wayland, vulkan-loader, fontconfig, freetype, zlib, desktop ? true }:

let
  release = lib.importJSON ./release.json;
  src = release.${stdenv.hostPlatform.system} or (throw "bragi: no release for ${stdenv.hostPlatform.system}");
in
stdenv.mkDerivation {
  pname = if desktop then "bragi" else "bragi-standalone";
  inherit (release) version;

  src = fetchurl { inherit (src) url sha256; };

  nativeBuildInputs = [ autoPatchelfHook makeWrapper ];
  buildInputs = [ libxkbcommon libxcb libX11 wayland fontconfig freetype zlib stdenv.cc.cc.lib ];

  dontConfigure = true;
  dontBuild = true;

  installPhase = ''
    runHook preInstall
    install -Dm755 bin/bragi $out/bin/bragi
    # The terminal app (from 1.0.0): no GUI libraries, so not wrapped.
    if [ -e bin/bragi-tui ]; then install -Dm755 bin/bragi-tui $out/bin/bragi-tui; fi
    install -Dm644 LICENSE $out/share/licenses/bragi/LICENSE
  '' + lib.optionalString desktop ''
    cp -a share/applications share/icons $out/share/
  '' + ''
    runHook postInstall
  '';

  # GPUI loads Vulkan, Wayland and xkbcommon at run time (dlopen).
  postFixup = ''
    wrapProgram $out/bin/bragi --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath [ vulkan-loader wayland libxkbcommon libX11 libxcb ]}
  '';

  meta = {
    description = "Chat with your private AI cloud (Ymir) over Weirdnet, desktop app";
    homepage = "https://bragi.weirdbrains.com";
    license = lib.licenses.bsd3;
    platforms = [ "x86_64-linux" "aarch64-linux" ];
    mainProgram = "bragi";
    sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
  };
}
