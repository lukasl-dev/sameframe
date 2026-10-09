{
  description = "Sameframe";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    crane.url = "github:ipetkov/crane";
  };

  outputs =
    inputs@{ crane, flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      flake.nixosModules = rec {
        default = sameframe;
        sameframe = import ./nix/module.nix { self = inputs.self; };
      };

      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];

      perSystem =
        { pkgs, system, ... }:
        let
          craneLib = crane.mkLib pkgs;

          topcoat-cli = craneLib.buildPackage {
            pname = "topcoat-cli";
            version = "0.10.0";
            src = pkgs.fetchzip {
              url = "https://static.crates.io/crates/topcoat-cli/topcoat-cli-0.10.0.crate";
              extension = "tar.gz";
              hash = "sha256-lk13D/RqrskzhXwLNz+KEbWowllQqGxoGU47T4ZdBvk=";
            };
            strictDeps = true;
            doCheck = false;
            meta.mainProgram = "topcoat";
          };

          commonArgs = {
            src = pkgs.lib.cleanSourceWith {
              src = ./.;
              filter =
                path: type:
                craneLib.filterCargoSources path type
                || pkgs.lib.hasPrefix "${toString ./.}/assets/" path
                || pkgs.lib.hasSuffix ".css" path;
            };
            strictDeps = true;
            nativeBuildInputs = [ pkgs.tailwindcss_4 ];
          };

          artifacts = craneLib.buildDepsOnly commonArgs;

          sameframe = craneLib.buildPackage (
            commonArgs
            // {
              cargoArtifacts = artifacts;
              nativeBuildInputs = commonArgs.nativeBuildInputs ++ [ topcoat-cli ];
              postBuild = ''
                topcoat asset bundle --release --out "$TMPDIR/topcoat-assets"
              '';
              postInstall = ''
                cp -r "$TMPDIR/topcoat-assets" "$out/bin/assets"
              '';
              meta.mainProgram = "sameframe";
            }
          );
        in
        {
          packages = {
            default = sameframe;
            inherit sameframe topcoat-cli;
          };

          devShells.default = craneLib.devShell {
            inputsFrom = [ sameframe ];
            packages = with pkgs; [
              rust-analyzer
              tailwindcss_4
              topcoat-cli
              nodejs
              jq
            ];
          };

          formatter = pkgs.nixfmt-tree;

          checks = {
            default = sameframe;
            client = pkgs.runCommand "sameframe-client-tests" { nativeBuildInputs = [ pkgs.nodejs ]; } ''
              cp -r ${./assets} assets
              cp -r ${./tests} tests
              node --test tests/client*.mjs
              touch "$out"
            '';
            clippy = craneLib.cargoClippy (
              commonArgs
              // {
                cargoArtifacts = artifacts;
                cargoClippyExtraArgs = "--all-targets -- -D warnings";
              }
            );
            fmt = craneLib.mkCargoDerivation (
              commonArgs
              // {
                cargoArtifacts = null;
                nativeBuildInputs = [
                  topcoat-cli
                  pkgs.rustfmt
                ];
                buildPhaseCargoCommand = "topcoat fmt --check --rustfmt src build.rs";
                doInstallCargoArtifacts = false;
              }
            );
          }
          // pkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
            module = import ./nix/tests/module.nix { inherit inputs pkgs system; };
            service = pkgs.testers.runNixOSTest (
              import ./nix/tests/service.nix {
                inherit inputs;
              }
            );
          };
        };
    };
}
