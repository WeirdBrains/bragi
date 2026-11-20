{
  description = "Bragi: chat with your private AI cloud";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      # The prebuilt release (packaging/nix/release.json): `bragi` with the desktop entry and icons,
      # `bragi-standalone` with only the binary.
      packages = forAll (pkgs: rec {
        bragi = pkgs.callPackage ./packaging/nix/package.nix { };
        bragi-standalone = pkgs.callPackage ./packaging/nix/package.nix { desktop = false; };
        default = bragi;
      });

      apps = forAll (pkgs: {
        default = {
          type = "app";
          program = "${self.packages.${pkgs.stdenv.hostPlatform.system}.bragi}/bin/bragi";
          meta.description = "Bragi, the desktop app";
        };
      });
    };
}
