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

Each module in `crates/odynsscomp/src/` opens with what it owes the next one,
and `codegen.rs` lists the three behaviors that look like bugs and are load
bearing. Read those before changing anything there.

Do not add an optimization. Not constant folding, not dead branch removal, not
jump shortening. The original has all of them and runs none of them, and every
one changes the bytes.

Keep errors flowing through `DiagnosticSink` so one run reports many problems.
Do not stop at the first error.

## Tests

Anything that needs game files skips when the corpus is absent, so a clean
checkout and CI stay green. Anything else goes in `tests/pipeline.rs` or a
`mod tests` in the module, and runs everywhere.

`tests/golden.rs` compiles the whole corpus and compares against the shipped
`.ncs`. It is ignored by default: it is minutes of IO, and roughly one shipped
file in fifteen was built from a different revision of the `.nss` beside it, so
even a perfect compiler misses those.

```sh
cargo test --test golden -- --ignored --nocapture
ODYNSSCOMP_CORPUS_LIMIT=200 cargo test --test golden -- --ignored
```

`tests/differential.rs` is the bar without that slack: it asks the original
compiler directly.

Name a test after the behavior it pins, not the function it calls.

## Differential testing against the original

`tests/differential.rs` compiles the same source with `nwnnsscomp.exe` under
Wine and with this crate, then requires the bytes to be equal. It is the only
test that can settle a question the specs leave open.

```sh
export ODYNSSCOMP_REF_DIR=~/Workspaces/odynsscomp_research/harness/ref
export WINEPREFIX=~/Workspaces/odynsscomp_research/wineprefix
cargo test --test differential -- --nocapture
```

`ODYNSSCOMP_REF_DIR` holds one subdirectory per game, each with a copy of
`nwnnsscomp.exe` and that game's `nwscript.nss`. The original picks its game
from the `nwscript.nss` beside the executable and nothing else, so those two
directories are the whole mechanism. Without the variable, or without Wine,
every test prints why it skipped and passes.

Two things about the original that any driver has to handle:

- **It always exits 0.** Errors, a missing input, a missing `nwscript.nss` -
  all still return 0. Read stdout, and check that a file starting with `NCS `
  appeared.
- **All diagnostics go to stdout.** stderr carries only Wine's noise.

Add a case with the `differential_test!` macro. On a mismatch the failure names
the first differing instruction with context on both sides, plus a hex window.

The wider harness lives outside this repository at
`~/Workspaces/odynsscomp_research/harness/`. It answers one-off questions
(`diffrun.py`), records what the original does (`probe_gaps.py`, which writes
`spec_gaps_resolved.md`), and scores the whole corpus (`corpus_score.py`, which
writes `corpus_scoreboard.md`). That scoreboard is how correctness gets
measured.
