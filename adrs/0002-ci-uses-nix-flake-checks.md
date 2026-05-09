# ADR 0002: CI runs the nix flake checks, not cargo via rustup

## Status

Accepted (2026-05-09). Supersedes [ADR 0001](0001-ci-uses-rustup-not-nix.md).

## Context

ADR 0001 switched CI from `nix build .#checks...` to `cargo build/test/clippy`
on a plain Ubuntu runner with `apt install libdbus-1-dev pkg-config` to make
`secretspec_derive` proc-macro loading reproduce. That was a workaround for a
Linux-only crane / sandbox interaction we never fully diagnosed.

The workaround violates a load-bearing repo invariant: the flake owns all system
dependencies. Any library the workspace links against (libdbus, openssl, ...) is
declared in `flake.nix` `buildInputs` so:

- Local Darwin dev, Linux dev shell, and CI all see the same pinned versions.
- A new contributor running `nix develop` or `nix build` gets a working build
  with no out-of-band package-manager steps.
- There is exactly one place to add a system dep when a new crate needs one.

`apt install` in CI splits that surface in two and lets the runner's package
catalogue drift from the flake. It also means a contributor who only uses
`nix build` cannot reproduce a CI failure without first replicating the apt
state, which inverts the value of having a hermetic flake at all.

## Decision

CI runs `nix build --accept-flake-config .#checks.x86_64-linux.<check>` for each
of `workspace-build`, `workspace-test`, `workspace-clippy`, `pre-commit`, in a
single matrix job. No `apt install` step. No rustup. No `Swatinem/rust-cache`.
The flake's `buildInputs` carry pkg-config, dbus, and openssl on Linux; Darwin
gets `libiconv`.

On failure, the workflow runs `nix log .#checks.x86_64-linux.<check>` so the
actual cargo / build-script stderr shows up in the Actions log instead of the
truncated summary `nix build` prints by default. This addresses ADR 0001's
correct point that nix had been hiding the real error - the fix is to unmask it,
not to abandon nix.

## Consequences

Positive:

- One source of truth for system deps: `flake.nix`.
- `nix build .#checks...` reproduces CI state exactly on any machine with Nix -
  no "but did you `apt install ...`" debugging.
- Crane's `cargoArtifacts` deps cache is reused across all four check
  derivations; the binary cache makes re-runs with unchanged inputs near
  instant.
- One workflow file, one matrix, one set of steps - simpler than the
  cargo-checks + pre-commit split ADR 0001 introduced.

Negative:

- If the original Linux-only `secretspec_derive` proc-macro `.so` failure
  recurs, we have to actually diagnose it instead of routing around it with apt.
  ADR 0001's "Lessons" section is preserved as a guide for that diagnosis: peel
  off layers until the real error is visible, reproduce on Linux directly, treat
  `E0463` on a proc-macro as either missing artifact, corrupt artifact, or
  unresolved-symbol artifact.

## Open work

If the proc-macro load failure resurfaces, the suspect surface is crane's
`buildDepsOnly` interaction with `strictDeps = true` and the secretspec
build-script chain. The flake already has `dbus` + `openssl` in `buildInputs`
for Linux, which is what the build script needs. Anything beyond that is a real
bug in the dep graph, not an apt-installable workaround.
