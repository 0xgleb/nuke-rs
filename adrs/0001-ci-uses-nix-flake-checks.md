# ADR 0001: CI runs the nix flake checks

## Status

Accepted (2026-05-09).

## Context

The repo ships a `flake.nix` with `crane`-based checks (`workspace-build`,
`workspace-test`, `workspace-clippy`, `pre-commit`) that work locally on Darwin
via `nix flake check`. The flake is the single source of truth for system
dependencies: anything the workspace links against (libdbus, openssl,
pkg-config, ...) is declared in `buildInputs`, so local Darwin dev, Linux dev
shell, and CI all see the same pinned versions and a new contributor running
`nix develop` or `nix build` gets a working build with no out-of-band
package-manager steps.

CI must respect that invariant. Splitting system deps between the flake and a
runner-side package manager (e.g. `apt install libdbus-1-dev pkg-config`)
defeats the point: the runner's package catalogue drifts from the flake, and a
contributor who only uses `nix build` cannot reproduce a CI failure without
first replicating the apt state.

## Decision

CI runs `nix build --accept-flake-config .#checks.x86_64-linux.<check>` for each
of `workspace-build`, `workspace-test`, `workspace-clippy`, and `pre-commit` in
a single matrix job on `ubuntu-latest`. No `apt install` step, no rustup, no
`Swatinem/rust-cache`. The flake's `buildInputs` carry `pkg-config`, `dbus`, and
`openssl` on Linux; Darwin gets `libiconv`. New system deps go into
`buildInputs` in `flake.nix`, never into the workflow.

On failure, the workflow runs `nix log .#checks.x86_64-linux.<check>` so the
actual cargo / build-script stderr shows up in the Actions log instead of the
truncated summary `nix build` prints by default. `--print-build-logs` is
deliberately omitted: streaming every compile line buries the real error when
something fails. Quiet on success, full failed-derivation log on failure.

## Consequences

Positive:

- One source of truth for system deps: `flake.nix`.
- `nix build .#checks...` reproduces CI state exactly on any machine with Nix.
  No "but did you `apt install ...`" debugging.
- Crane's `cargoArtifacts` deps cache is reused across all four check
  derivations; the binary cache makes re-runs with unchanged inputs near
  instant.

Negative:

- A Linux-only failure inside the nix sandbox (e.g. proc-macro `.so` unloadable,
  `E0463 "can't find crate"`) needs to be diagnosed against the crane / fenix /
  secretspec stack instead of routed around with apt. See "Lessons" below for
  the playbook.

## Prior iteration (do not repeat)

An earlier attempt switched CI to `cargo build/test/clippy` via
`dtolnay/rust-toolchain` with `apt install libdbus-1-dev pkg-config` to sidestep
a Linux-only `secretspec_derive` proc-macro load failure (`E0463`) inside the
nix sandbox. That decision was reverted: it violated the "flake owns system
deps" invariant, split the build surface in two, and let the runner's package
versions drift from the flake. If the proc-macro load failure resurfaces, fix it
in the dep graph - do not reach for apt.

## Lessons (for the next time CI gets weird)

1. **Peel off tooling layers until the actual error is visible.** Layers of
   indirection (nix, crane, sandbox, magic-nix-cache, action runners) each have
   their own way of eating error messages. The layer closest to the actual
   compiler is the only one that gives you the truth - get the failed
   derivation's stderr (`nix log`) before theorising.

2. **A failure that doesn't reproduce on Darwin is a Linux-specific failure.**
   Reproduce on Linux directly (Docker, devcontainer, or Actions runner) before
   theorising about caching, fingerprints, or proc-macro lifecycle.

3. **`E0463 "can't find crate"` for a proc-macro means the `.so` isn't
   loadable.** This can be:
   - the artifact is missing entirely (cache hole, build skip),
   - the artifact exists but is corrupt (sed pass over binary, strip removing
     required sections), or
   - the artifact exists but links against unresolved symbols (missing system
     lib at compile time, NOT at runtime).

   Build-script failures during proc-macro compilation manifest as load failures
   at consumer compile time - several layers of indirection from the actual
   cause.

4. **For build-script-heavy crates like `keyring` / `secretspec`, CI failures
   are usually system libraries, not Rust toolchain or nix tooling.** Check
   `flake.nix`'s `buildInputs` first; if a system lib is missing, add it there.
