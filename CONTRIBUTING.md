# Contributing

## Before you push

```sh
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
```

CI runs the same three on Linux and Windows, plus a build on Rust 1.77.2. All
of them have to pass.

## Rules that are not negotiable

**No game data in the repository.** No `nwscript.nss`, no `.ncs` from either
game, no `nwnnsscomp.exe`. Tests reach for those through `ODYNSSCOMP_CORPUS`.
`.gitignore` blocks the obvious names; do not work around it.

**No new dependencies without a reason in the pull request.** The library is
std-only so it keeps building on Rust 1.77 and adds nothing to a consumer's
lock file. A parser framework is not a reason.

**Minimum supported Rust version is 1.77.** If you need something newer, say so
in the pull request and expect a conversation, not a merge.

**Byte equality with `nwnnsscomp.exe` decides arguments.** Where the original
compiler makes an arbitrary choice, copy it. Tidier output that differs by one
byte is a failure.

## Working on a stage

Each module in `crates/odynsscomp/src/` has a `# Status` section listing what an
implementation owes the next stage. Read it first. Replace the
`CompileError::NotImplemented` return with real work, delete the `TODO`, and
drop the matching skip arm in `tests/golden.rs` once the stage is real.

Keep errors flowing through `DiagnosticSink` so one run reports many problems.
Do not stop at the first error.

## Tests

Anything that needs game files goes in `tests/golden.rs` and skips when the
corpus is absent. Anything else goes in `tests/pipeline.rs` or a `mod tests` in
the module, and runs everywhere.

Name a test after the behavior it pins, not the function it calls.
