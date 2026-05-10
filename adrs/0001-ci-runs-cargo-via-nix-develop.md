# ADR 0001: CI runs cargo inside `nix develop`

## Status

Accepted (2026-05-09). Supersedes the prior version of this ADR which had CI run
`nix build .#checks.<system>.<check>` directly.

## Context

The repo ships a `flake.nix` with `crane`-based checks (`workspace-build`,
`workspace-test`, `workspace-clippy`, `pre-commit`). The flake is the single
source of truth for system dependencies: anything the workspace links against
(libdbus, openssl, pkg-config, ...) is declared in `buildInputs`, so local
Darwin dev, Linux dev shell, and CI all see the same pinned versions and a new
contributor running `nix develop` gets a working build with no out-of-band
package-manager steps. CI must respect that invariant.

The prior incarnation of this ADR ran each crane check via
`nix build --accept-flake-config .#checks.x86_64-linux.<check>` and then
`nix log` on failure. In practice that buried errors:

- `nix build` emits a `building '/nix/store/<hash>-cargo-src-<name>.drv'...`
  line for every cargo dep crane materialises as a separate derivation.
  Workspaces with hundreds of deps produce tens of thousands of progress lines
  before the actual error.
- GitHub Actions truncates step output past a size threshold. The truncation
  hits before any failed-derivation stderr, so the "real" error never reaches
  the log even when the build does fail later.
- The recovery `nix log .#checks.x86_64-linux.<check>` step retrieves the
  _top-level_ check derivation's log. If a nested cargo-src dep fails, the
  top-level log just records the wrapper failure - the dep's stderr lives in a
  _different_ nix log path. So the recovery step pulls a useless summary.

Net effect: a real cargo or clippy error is invisible. The "lessons" section of
the prior ADR even called out "peel off tooling layers until the actual error is
visible" - but the chosen tooling (crane + `nix build` + `nix log`) hid errors
behind two layers of indirection.

## Decision

CI runs `cargo` directly inside a pure `nix develop .#ci` shell:

- `flake.nix` exposes `devShells.<system>.ci`: a plain `pkgs.mkShell` (no
  devenv, no `--impure`) with the fenix-pinned toolchain plus the same
  `nativeBuildInputs` / `buildInputs` the crane checks use.
- The CI workflow runs each cargo command (`cargo build`, `cargo test`,
  `cargo clippy`) as `nix develop --accept-flake-config .#ci -c <command>` in a
  matrix job on `ubuntu-latest`.
- Pre-commit hooks still run via `nix build .#checks.x86_64-linux.pre-commit` in
  a separate job, because that derivation produces a small, focused log that
  doesn't blow up.
- A GitHub-Actions-side `actions/cache` step caches `~/.cargo/...` and `target/`
  keyed on `Cargo.lock` + `flake.lock` so re-runs with unchanged inputs are
  fast.
- No `apt install`, no `dtolnay/rust-toolchain`, no `rustup`. The flake still
  owns all system deps; new ones go into `buildInputs` in `flake.nix`, never
  into the workflow.

## Consequences

Positive:

- Cargo's stderr streams straight to the Actions log. The actual compile error
  is visible, immediately, without indirection.
- One source of truth for system deps: `flake.nix`.
  `nix develop .#ci -c
  cargo <cmd>` reproduces CI on any machine with Nix.
- Cargo incremental + GH cache keep re-runs fast even though crane's per-dep nix
  derivations are no longer reused across check kinds.

Negative:

- Crane's per-dep deps cache is no longer reused across CI jobs (each cargo
  matrix entry maintains its own GH-Actions cache). The trade-off: cargo's
  incremental compilation + GH cache is plenty fast in practice, and the
  observability win dominates.
- Pure-flake hermeticity is partially relaxed for the cargo jobs: the cargo
  invocation inside `nix develop` writes to `target/` outside the nix store.
  System deps are still pinned by the flake, so this is a build-cache concern,
  not a reproducibility one.

## Prior iterations (do not repeat)

Two earlier setups failed and should not be revived:

1. `cargo` via `dtolnay/rust-toolchain` + `apt install libdbus-1-dev pkg-config`
   on the runner. Violated the "flake owns system deps" invariant: the runner's
   apt versions could drift from the flake, splitting the build surface in two
   and breaking reproducibility for `nix build` users.
2. `nix build .#checks.x86_64-linux.<check>` + `nix log` on failure. Buried the
   cargo error under tens of thousands of `building '...drv'` lines that GitHub
   then truncated; the recovery `nix log` pulled the top-level derivation, not
   the failing nested dep, so the actual error stayed hidden.

The current setup preserves the upside of (1) - cargo errors are visible - while
keeping the upside of (2) - flake owns system deps. If a build-script-heavy
crate fails to link a system lib in CI, fix `buildInputs` in `flake.nix`. Do not
reach for apt.

## Lessons (for the next time CI gets weird)

1. **Peel off tooling layers until the actual error is visible.** Layers of
   indirection (nix, crane, sandbox, magic-nix-cache, action runners) each eat
   error messages in their own way. The layer closest to the compiler is the
   only one that gives you the truth - get cargo's stderr first, theorise
   second.

2. **A failure that doesn't reproduce on Darwin is a Linux-specific failure.**
   Reproduce on Linux directly (Docker, devcontainer, Actions runner) before
   theorising about caching, fingerprints, or proc-macro lifecycle.

3. **`E0463 "can't find crate"` for a proc-macro means the `.so` isn't
   loadable.** This can be:
   - the artifact is missing entirely (cache hole, build skip),
   - the artifact exists but is corrupt (sed pass over binary, strip removing
     required sections), or
   - the artifact exists but links against unresolved symbols (missing system
     lib at compile time, NOT at runtime).

4. **For build-script-heavy crates like `keyring` / `secretspec`, CI failures
   are usually system libraries, not Rust toolchain or nix tooling.** Check
   `flake.nix`'s `buildInputs` first; if a system lib is missing, add it there.
