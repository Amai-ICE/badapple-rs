{
  description = "Rust env";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = inputs@{ self, ...}: 
  inputs.flake-utils.lib.eachDefaultSystem (
      system: let
        overlays = [ (import inputs.rust-overlay) ];
        pkgs = import (inputs.nixpkgs) { inherit system overlays; };

        nativeBuildInputs = with pkgs; [
          ffmpeg.dev
          pkg-config

          rustPlatform.bindgenHook
          makeWrapper
        ];

        buildInputs = with pkgs; [
          pulseaudio
        ];

        rustPlatform = pkgs.makeRustPlatform {
          cargo = pkgs.rust-bin.stable.latest.minimal;
          rustc = pkgs.rust-bin.stable.latest.minimal;
        };

        FFMPEG_PATH = with pkgs.ffmpeg.dev;
        "${dev.out}/includes";

      in {
        packages.default = rustPlatform.buildRustPackage rec {
          inherit buildInputs nativeBuildInputs;

          name = "badapple";
          src = ./.;
          version = "0.0.1";
          meta.mainProgram = name;

          cargoLock = {
            lockFile = ./Cargo.lock;
            allowBuiltinFetchGit = true;
          };

          postFixup = ''
            wrapProgram $out/bin/${name}
          '';
        };

        devShells.default = pkgs.mkShell {
          inherit nativeBuildInputs;
         
          buildInputs = 
            buildInputs
            ++ (with pkgs.rust-bin; [
              (stable.latest.minimal.override {
                extensions = [
                  "clippy"
                  "rust-src"
                ];
              })
              nightly.latest.rustfmt
              nightly.latest.rust-analyzer
            ]);
          
          RUST_BACKTRACE = 1;
          PKG_CONFIG_PATH = FFMPEG_PATH;
        };
      }
    );
}
