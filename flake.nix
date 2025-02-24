{
  description = "Protofetch - A source dependency management tool for Protobuf files";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    crane.url = "github:ipetkov/crane";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      crane,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        inherit (pkgs) lib;
        craneLib = crane.mkLib pkgs;

        protofetch = craneLib.buildPackage {
          pname = "protofetch";
          src = lib.cleanSourceWith {
            src = ./.; # The original, unfiltered source
            filter = path: type: (lib.hasSuffix "\.proto" path) || (craneLib.filterCargoSources path type);
          };
          buildInputs = [
            pkgs.openssl
            pkgs.libgit2
            pkgs.pkg-config
          ] ++ lib.optionals pkgs.stdenv.isDarwin [ pkgs.darwin.apple_sdk.frameworks.Security ];
          preBuild = ''
            export HOME=$(mktemp -d)
          '';
        };
      in
      {
        packages = rec {
          inherit protofetch;
          default = protofetch;
        };
        checks = {
          # Build the crate as part of `nix flake check`
          inherit protofetch;
        };
      }
    );
}
