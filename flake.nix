{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    treefmt-nix = {
      url = "github:numtide/treefmt-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    git-hooks = {
      url = "github:cachix/git-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    inputs@{
      nixpkgs,
      flake-parts,
      ...
    }:

    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];

      imports = with inputs; [
        git-hooks.flakeModule
        treefmt-nix.flakeModule
      ];

      perSystem =
        {
          config,
          pkgs,
          system,
          ...
        }:
        let
          toolchain = pkgs.rust-bin.stable.latest.default;
          rustPlatform = pkgs.makeRustPlatform {
            cargo = toolchain;
            rustc = toolchain;
          };

          joinbell = rustPlatform.buildRustPackage {
            pname = "joinbell";
            version = "0.1.0";

            src = ./.;

            cargoLock.lockFile = ./Cargo.lock;

            meta = {
              homepage = "https://github.com/yadokani389/joinbell";
              license = with pkgs.lib.licenses; [
                asl20
                mit
              ];
              mainProgram = "joinbell";
            };
          };
        in
        {
          _module.args.pkgs = import nixpkgs {
            inherit system;
            overlays = [ inputs.rust-overlay.overlays.default ];
          };

          packages.default = joinbell;

          devShells.default = pkgs.mkShell {
            inputsFrom = [
              config.pre-commit.devShell
              joinbell
            ];
          };

          treefmt = {
            projectRootFile = "flake.nix";
            programs = {
              nixfmt.enable = true;
              rustfmt.enable = true;
              taplo.enable = true;
            };

            settings.formatter = {
              taplo.options = [
                "fmt"
                "-o"
                "reorder_keys=true"
              ];
            };
          };

          pre-commit.settings = {
            hooks = {
              ripsecrets.enable = true;
              typos.enable = true;
              treefmt.enable = true;
              clippy = {
                enable = true;
                packageOverrides.cargo = toolchain;
                packageOverrides.clippy = toolchain;
              };
            };
          };
        };
    };
}
