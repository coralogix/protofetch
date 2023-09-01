{
    description = "Protofetch - A source dependency management tool for Protobuf files";

    inputs = {
        nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
        flake-utils.url = "github:numtide/flake-utils";
        crane = { url = "github:ipetkov/crane"; inputs.nixpkgs.follows = "nixpkgs"; };
    };

    outputs = { self, nixpkgs, flake-utils, crane }: flake-utils.lib.eachDefaultSystem (system:
    let
        pkgs = import nixpkgs { inherit system; };
        inherit (pkgs) lib;
        craneLib = crane.lib.${system};
    in {
        packages = rec {
            default = protofetch;
            protofetch = craneLib.buildPackage {
                pname = "protofetch";
                src = lib.cleanSourceWith {
                    src = ./.; # The original, unfiltered source
                    filter = path: type:
                        (lib.hasSuffix "\.proto" path) ||
                        (craneLib.filterCargoSources path type);
                };
                buildInputs = with pkgs; [
                    openssl
                    libgit2
                ] ++ (if stdenv.isDarwin then [darwin.apple_sdk.frameworks.Security] else []);
                preBuild = ''
                    export HOME=$(mktemp -d)
                '';
            };
        };
    });
}




