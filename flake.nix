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

    crane.url = "github:ipetkov/crane";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      devenv,
      git-hooks,
      fenix,
      crane,
      ...
    }@inputs:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ fenix.overlays.default ];
        };
        # `fenix.packages.<system>.default` is a *profile* (a set of
        # component derivations: cargo, rustc, rust-src, ...). Crane's
        # `overrideToolchain` and the dev shell each want a single
        # combined-derivation toolchain, which fenix exposes via the
        # profile's `.toolchain` attribute.
        fenixProfile = fenix.packages.${system}.default;
        toolchain = fenixProfile.toolchain;

        # Crane builds the workspace as a Nix derivation, sandboxed
        # against the Nix store. That gives us:
        #   - cargo dep cache (`buildDepsOnly`) reused across the
        #     check / build / clippy / test outputs, so each check
        #     only recompiles the workspace itself.
        #   - the `checks.<system>.*` outputs cache in any nix binary
        #     cache, so CI re-runs that produce the same inputs are
        #     instant.
        #   - no `--impure` (which disables nix substitution).
        #
        # `overrideToolchain` takes a function that returns the merged
        # toolchain derivation, so fenix's profile (which is itself a
        # set of components) can be wired in via `_: toolchain`.
        craneLib = (crane.mkLib pkgs).overrideToolchain (_: toolchain);

        # Filter what gets fed into the cargo build's `src`. The
        # default `cleanCargoSource` only keeps Cargo.toml + Cargo.lock
        # + Rust sources; we add `secretspec.toml` because the
        # `secretspec_derive::declare_secrets!` macro reads it at
        # compile time.
        src = pkgs.lib.cleanSourceWith {
          src = ./.;
          filter =
            path: type:
            (craneLib.filterCargoSources path type) || (builtins.baseNameOf path == "secretspec.toml");
        };

        # System libraries the workspace links against.
        # - pkg-config: build-script glue for libdbus / openssl probes.
        # - dbus + openssl: secretspec pulls in `keyring` which on
        #   Linux needs both via `dbus-secret-service` / `aws-sdk-*`.
        nativeBuildInputs = with pkgs; [ pkg-config ];
        buildInputs =
          with pkgs;
          lib.optionals stdenv.isDarwin [ libiconv ]
          ++ lib.optionals stdenv.isLinux [
            dbus
            openssl
          ];

        commonArgs = {
          inherit src nativeBuildInputs buildInputs;
          strictDeps = true;
          # Required by crane's derivation builders; the workspace
          # has many crates so we use a generic name.
          pname = "nuke-workspace";
          version = "0.1.0";
          # `secretspec` reads ETH_WS_RPC_URL via providers (keyring,
          # dotenv, etc.). The build-time macro just generates types
          # against the secretspec.toml schema; no provider lookup
          # happens until runtime, so the build is hermetic.
        };

        # Builds every workspace dep as a separate derivation that
        # downstream checks reuse (so `cargo clippy`, `cargo test`,
        # and `cargo build` don't each recompile the dep tree).
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        hooks = {
          actionlint.enable = true;
          denofmt.enable = true;
          nixfmt.enable = true;

          taplo.enable = true;
          rustfmt = {
            enable = true;
            # The hook reads `cargo` and `rustfmt` as separate
            # component derivations, which the fenix *profile*
            # exposes by name (the combined `toolchain` derivation
            # bundles them but doesn't expose them as attributes).
            packageOverrides = { inherit (fenixProfile) cargo rustfmt; };
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
                packages = nativeBuildInputs ++ buildInputs;

                # devenv's rust language module expects a *profile*
                # (component-set) so it can pick `cargo`, `rustc`,
                # `rust-src` etc. by attribute.
                languages.rust = {
                  enable = true;
                  toolchain = fenixProfile;
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

        # The full CI surface lives here. CI runs `nix flake check`
        # and is done; no `--impure`, no `nix develop -c cargo ...`,
        # no per-step cache layers.
        checks = {
          # Pre-commit hooks (rustfmt, nixfmt, taplo, denofmt,
          # actionlint).
          pre-commit = git-hooks.lib.${system}.run {
            src = ./.;
            inherit hooks;
          };

          # `cargo build --all-targets` over the whole workspace.
          # Crane passes `--locked` and `--release` automatically.
          workspace-build = craneLib.cargoBuild (
            commonArgs
            // {
              inherit cargoArtifacts;
              cargoExtraArgs = "--all-targets";
            }
          );

          # `cargo test --all-targets`.
          workspace-test = craneLib.cargoTest (
            commonArgs
            // {
              inherit cargoArtifacts;
              cargoExtraArgs = "--all-targets";
            }
          );

          # `cargo clippy --all-targets -- -D warnings`. Crane
          # already passes `--locked` and `--release` automatically.
          workspace-clippy = craneLib.cargoClippy (
            commonArgs
            // {
              inherit cargoArtifacts;
              cargoClippyExtraArgs = "--all-targets -- -D warnings";
            }
          );
        };
      }
    );

  nixConfig = {
    extra-trusted-public-keys = "devenv.cachix.org-1:w1cLUi8dv3hnoSPGAuibQv+f9TZLr6cv/Hm9XgU50cw=";
    extra-substituters = "https://devenv.cachix.org";
  };
}
