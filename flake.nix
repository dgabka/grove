{
  description = "grove development shell";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-26.05-darwin";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, rust-overlay, ... }:
    let
      systems = [ "aarch64-darwin" "aarch64-linux" "x86_64-darwin" "x86_64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
      pkgsFor = system: import nixpkgs {
        inherit system;
        overlays = [ rust-overlay.overlays.default ];
      };
      version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
    in
    {
      packages = forAllSystems (system:
        let pkgs = pkgsFor system;
        in rec {
          grove = pkgs.rustPlatform.buildRustPackage {
            pname = "grove";
            inherit version;
            src = self;
            cargoLock.lockFile = ./Cargo.lock;
            nativeBuildInputs = [ pkgs.makeWrapper ];
            nativeCheckInputs = [ pkgs.git ];
            postInstall = ''
              wrapProgram $out/bin/grove --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.fzf pkgs.git pkgs.tmux ]}
            '';
            meta.mainProgram = "grove";
          };
          default = grove;
        });

      devShells = forAllSystems (system:
        let pkgs = pkgsFor system;
        in {
          default = pkgs.mkShell {
            packages = with pkgs; [
              rust-bin.beta.latest.default
              rust-analyzer
              cargo-watch
            ];
          };
        });
    };
}
