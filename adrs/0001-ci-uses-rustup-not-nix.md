# ADR 0001: CI runs cargo via rustup, not nix flake checks

## Status

Accepted (2026-05-07).

## Context

The repo ships a `flake.nix` with `crane`-based checks (`workspace-build`,
`workspace-test`, `workspace-clippy`, `pre-commit`) that work locally on Darwin
via `nix flake check`. CI was originally wired the same way: GitHub Actions ran
`nix build .#checks.x86_64-linux.<check>` for each matrix entry.

When the `dex_arb` example started pulling `secretspec` (typed config-file
codegen via `secretspec_derive::declare_secrets!`), every Linux CI run failed
with:

```
error[E0463]: can't find crate for `secretspec_derive`
  --> examples/dex_arb/src/main.rs:36:1
```

Darwin-local `nix build` of the same checks succeeded.

## What was actually wrong

`secretspec` -> `keyring` -> `dbus-secret-service` -> `libdbus-sys`. The
`libdbus-sys` build script invokes `pkg-config --libs --cflags dbus-1`. On a
Linux box without `libdbus-1-dev` + `pkg-config`, that build script panics:

```
The system library `dbus-1` required by crate `libdbus-sys` was not found.
```

That panic was visible immediately when CI ran `cargo build` directly. It was
NOT visible through `nix build .#checks...workspace-test` - the nix sandbox
either swallowed the build-script panic and produced an unloadable proc-macro
`.so` for `secretspec_derive` (linked against unresolved D-Bus symbols), or the
dep-graph was such that `libdbus-sys` was simply skipped. Either way, the
downstream symptom was rustc failing to dlopen the proc-macro at consumer
compile time, which surfaces as `E0463 "can't find crate"` - a misleading error
message that points at the proc-macro dep, not the missing system lib.

The flake had `dbus` and `openssl` in `buildInputs` for Linux:

```nix
buildInputs = lib.optionals stdenv.isLinux [ dbus openssl ];
```

That should have been enough, but with `strictDeps = true` and crane's
`buildDepsOnly` stubbing of workspace member sources, the propagation broke
somewhere we never fully diagnosed. Spent eight CI iterations chasing the wrong
layer (crane's `dummySrc` stubbing, `removeReferencesTo*` post-install hooks,
`dontStrip` / `dontPatchELF` on the target tarball, deps cache corruption
hypotheses). None of those were the issue.

## Decision

CI runs `cargo build / test / clippy` directly via `dtolnay/rust-toolchain` on a
plain Ubuntu runner, with `apt install libdbus-1-dev pkg-config` as a preceding
step. The flake's `pre-commit` check still runs through nix (formatting / lint
hooks only - no Rust compilation involved).

The flake's `workspace-build` / `workspace-test` / `workspace-clippy` checks
remain wired in `flake.nix` for local development on Darwin where they work
fine. They are no longer the CI source of truth.

`.github/workflows/ci.yaml` ships three matrix entries (`build`, `test`,
`clippy`) plus `pre-commit`, all parallel. Total wall-time is ~3 minutes vs. the
~12-15 minutes the previous nix-based CI took for the deps phase alone.

## Consequences

Positive:

- Failures surface the real cargo / build-script error instead of a misleading
  `E0463` masked by nix sandbox + crane caching layers.
- ~4x faster CI.
- One less moving part: no `magic-nix-cache-action`, no `flake-checker`, no
  `crane.cargoArtifacts` invalidation matrix.
- Plain cargo error messages link directly to upstream documentation
  (`apt install libdbus-1-dev`).

Negative:

- CI no longer validates the nix flake's `workspace-*` checks. Local
  `nix flake check` on Darwin remains the gate for "the flake still works for
  nix users".
- Two ways to build (cargo and nix). The cargo path is canonical for CI; the nix
  path is canonical for reproducible local dev. Versions of system libs (e.g.
  `libdbus`) that ship on the GitHub runner can drift from what nix pins.

## Lessons (for the next time CI gets weird)

1. **Peel off tooling layers until the actual error is visible.** Eight failed
   CI runs would have been zero if I had run `cargo build` on a plain Linux
   container in iteration 1. Layers of indirection (nix, crane, sandbox,
   magic-nix-cache, action runners) each have their own way of eating error
   messages, and the one closest to the actual compiler is the only one that
   gives you the truth.

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

   For us it was the third case. Build-script failures during proc-macro
   compilation manifest as load failures at consumer compile time - several
   layers of indirection from the actual cause.

4. **For build-script-heavy crates like `keyring` / `secretspec`, the reasons CI
   fails are usually system libraries, not Rust toolchain or nix tooling.**
   Check `apt list --installed | grep dev` first.
