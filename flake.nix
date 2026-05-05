{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
    flake-utils.url = "github:numtide/flake-utils";

    git-hooks.url = "github:cachix/git-hooks.nix";
    git-hooks.inputs.nixpkgs.follows = "nixpkgs";

    devenv.url = "github:cachix/devenv";
    devenv.inputs.git-hooks.follows = "git-hooks";

    fenix.url = "github:nix-community/fenix";
    fenix.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      devenv,
      git-hooks,
      fenix,
      ...
    }@inputs:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ fenix.overlays.default ];
        };
        toolchain = fenix.packages.${system}.default;

        hooks = {
          actionlint.enable = true;
          denofmt.enable = true;
          nixfmt.enable = true;

          taplo.enable = true;
          rustfmt = {
            enable = true;
            packageOverrides = { inherit (toolchain) cargo rustfmt; };
          };
        };

      in
      {
        devShells = {
          default = devenv.lib.mkShell {
            inherit inputs pkgs;
            modules = [
              {
                # https://devenv.sh/reference/options/
                packages = with pkgs; lib.optionals stdenv.isDarwin [ libiconv ];

                languages.rust = {
                  enable = true;
                  inherit toolchain;
                };

                difftastic.enable = true;
                git-hooks = { inherit hooks; };
              }
            ];
          };
        };

        packages = {
          devenv-up = self.devShells.${system}.default.config.procfileScript;
        };

        checks = {
          pre-commit = git-hooks.lib.${system}.run {
            src = ./.;
            inherit hooks;
          };
        };
      }
    );

  nixConfig = {
    extra-trusted-public-keys = "devenv.cachix.org-1:w1cLUi8dv3hnoSPGAuibQv+f9TZLr6cv/Hm9XgU50cw=";
    extra-substituters = "https://devenv.cachix.org";
  };
}
