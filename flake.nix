{
  description = "Protofetch - A source dependency management tool for Protobuf files";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    crane.url = "github:ipetkov/crane";
  };

  outputs =
    {
      self,
      nixpkgs,
      crane,
    }:
    let
      systems = [
        "aarch64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;

      mkProtofetch =
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          inherit (pkgs) lib;
          craneLib = crane.mkLib pkgs;
        in
        craneLib.buildPackage {
          pname = "protofetch";
          src = lib.cleanSourceWith {
            src = ./.; # The original, unfiltered source
            filter = path: type: (lib.hasSuffix "\.proto" path) || (craneLib.filterCargoSources path type);
          };
          buildInputs = [
            pkgs.git
            pkgs.openssl
            pkgs.libgit2
            pkgs.pkg-config
          ];
          preBuild = ''
            export HOME=$(mktemp -d)
          '';
        };
    in
    {
      packages = forAllSystems (
        system:
        let
          protofetch = mkProtofetch system;
        in
        rec {
          inherit protofetch;
          default = protofetch;
        }
      );
      checks = forAllSystems (system: {
        # Build the crate as part of `nix flake check`
        protofetch = self.packages.${system}.protofetch;
      });
    };
}
