{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    naersk.url = "github:nix-community/naersk";
  };

  outputs =
    inputs@{
      flake-parts,
      naersk,
      ...
    }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];

      perSystem =
        { pkgs, ... }:
        let
          llvmPackages = pkgs.rustc.llvmPackages;

          commonArgs = {
            buildInputs = [
              pkgs.nix.dev
              llvmPackages.libclang.lib
            ];

            nativeBuildInputs = [
              pkgs.pkg-config
              llvmPackages.bintools
            ];
          };

          naerskLib = pkgs.callPackage naersk { };
        in
        {
          packages.default = naerskLib.buildPackage (
            commonArgs
            // {
              src = ./.;
              LIBCLANG_PATH = "${llvmPackages.libclang.lib}/lib";
              preBuild = ''
                export LIBCLANG_PATH=${llvmPackages.libclang.lib}/lib
              '';
            }
          );

          devShells.default = pkgs.mkShell {
            inherit (commonArgs) buildInputs;

            nativeBuildInputs =
              commonArgs.nativeBuildInputs
              ++ (with pkgs; [
                cargo
                rustc
                rustfmt
                clippy
                rust-analyzer
              ]);

            env = {
              RUST_SRC_PATH = "${pkgs.rust.packages.stable.rustPlatform.rustLibSrc}";
              PKG_CONFIG_PATH = "${pkgs.nix.dev}/lib/pkgconfig";
              LIBCLANG_PATH = "${llvmPackages.libclang.lib}/lib";
            };
          };
        };
    };
}
