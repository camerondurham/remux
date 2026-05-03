{
  description = "A local-first CLI/TUI for finding, inspecting, and attaching to tmux panes across local and SSH hosts";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "aarch64-darwin" "x86_64-darwin" "aarch64-linux" "x86_64-linux" ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems f;
      version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
    in
    {
      packages = forAllSystems (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          remux = pkgs.rustPlatform.buildRustPackage {
            pname = "remux";
            inherit version;
            src = self;
            cargoLock.lockFile = ./Cargo.lock;
            # e2e tests spawn fake ssh/tmux binaries and require a writable
            # home directory; they are incompatible with the Nix build sandbox.
            doCheck = false;
            meta = {
              description = "A local-first CLI/TUI for finding, inspecting, and attaching to tmux panes across local and SSH hosts";
              mainProgram = "remux";
              platforms = pkgs.lib.platforms.unix;
            };
          };
        in
        { default = remux; inherit remux; }
      );

      apps = forAllSystems (system: {
        default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/remux";
        };
      });

      devShells = forAllSystems (system:
        let pkgs = nixpkgs.legacyPackages.${system};
        in {
          default = pkgs.mkShell {
            packages = with pkgs; [ rustc cargo rustfmt clippy ];
          };
        }
      );

      overlays.default = final: _prev: {
        remux = self.packages.${final.system}.default;
      };
    };
}
