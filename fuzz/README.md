# vf-fuzz

cargo-fuzz harness for `vf-core::scope` (VFL-145, T2 addendum to C2 / VFL-15).
Own Cargo workspace (`Cargo.toml`'s empty `[workspace]`), pinned to its own
nightly toolchain (`rust-toolchain.toml`) separately from the repository's
production `1.99.0` pin. Test-lane (`ci/lane-gate.sh:38`): authored and owned
by Halsey (Test Writer), not by the coders who own `crates/vf-core/src`.

## Running the targets

From this directory, **without** a `+nightly` override — the override
bypasses `rust-toolchain.toml`'s pin and resolves whatever toolchain happens
to be installed instead of the one this harness is reviewed against (see
[VFL-202](/VFL/issues/VFL-202#document-ruling), closing F6):

```
cd fuzz
cargo fuzz run canonical_host -- -max_total_time=600
cargo fuzz run scope_match -- -max_total_time=600
```

Each run is evidence for C2's (VFL-15) acceptance criterion: 10 minutes
locally without panic. Requires the toolchain VFL-201 provisions: the
`nightly-2026-09-01` pin with `rust-src`, `cargo-fuzz` 0.13, and a C++
compiler (`libfuzzer-sys` compiles bundled C++ in its build script, so even
`cargo check` needs one).

Type-checking only, without linking libFuzzer or actually fuzzing, is
`just fuzz-check` from the repository root — wired into `just gate` and
`.github/workflows/rust-check.yml`. It still resolves this directory's
`rust-toolchain.toml`, so it runs on the nightly pin, not the production
`1.99.0` one; it needs the same toolchain (plus a C++ compiler) as the real
runs above, just not libFuzzer's linking step.

## Crash reproducers

`artifacts/` and `corpus/` are gitignored. A crash found by a run does not
stay a local file: it becomes a named regression test in
`crates/vf-core/tests/`, authored by Halsey, with the raw reproducer artefact
attached to the Test Runner run report that found it.
