# bionsscomp

Compiles NWScript source (`.nss`) into compiled scripts (`.ncs`) for
Star Wars: Knights of the Old Republic I and II.

It replaces `nwnnsscomp.exe`. That binary is Windows-only, so Linux and macOS
modders run it under wine, and a patcher that shells out to it has to ship it,
find it, and hope wine is installed. This is a Rust library instead. A patcher
links it and compiles in process: no subprocess, no temporary files, no wine.

## Status

It compiles. Every one of the 2340 shipped script pairs that `nwnnsscomp.exe`
itself builds, this compiler builds to identical bytes, across both games and
all four difficulty tiers. The 30 it declines to build, this one declines too.

Byte equality is the acceptance test, not merely running the same. That is why
this compiler does no optimization at all: the original gates every
optimization it has behind a flag the KOTOR fork left permanently off, so
matching its output means matching that too. `2 + 3` really does emit two
pushes and an add.

| Part | State |
| --- | --- |
| Preprocessor, lexer, parser | done |
| Name resolution, layout, reachability | done |
| Code generation and `.ncs` emission | done |
| `nwscript.nss` routine table parsing | done |
| Command line, argv-compatible with the original | done |
| Decompiling | not supported, and not planned |

Two things the original rejects and this compiler accepts: a definition whose
parameter list disagrees with an earlier prototype, and an argument whose type
does not match the parameter for a handful of engine routines. Seven corpus
scripts are in that gap. Nothing in it changes the bytes of a script that
compiles both ways.

## Use it as a library

```rust
use bionsscomp::{compile, CompileOptions, Game, NwScript};

let nwscript = NwScript::load(Game::K1, "/path/to/kotor/nwscript.nss")?;
let options = CompileOptions::new(&nwscript)
    .with_include_path("/path/to/mod/scripts");

let ncs_bytes = compile(source, &options)?;
```

`compile` returns the bytes. Writing them somewhere is the caller's decision,
which is what lets a patcher compile straight into an archive.

## Use it from the command line

The argument grammar of `nwnnsscomp.exe` is accepted as-is, so existing scripts
and tools keep working:

```sh
bionsscomp -c k_test.nss -o k_test.ncs
```

Longer options cover what the original guessed at:

```sh
bionsscomp --game 2 --nwscript /path/to/tsl/nwscript.nss \
           --include ./scripts k_test.nss -o k_test.ncs
```

| Option | Meaning |
| --- | --- |
| `-c` | Compile. This is the default. |
| `-d` | Decompile. Not supported. |
| `-o <file>` | Where to write the compiled script. |
| `-g`, `--game <1\|2>` | Which game to compile for. Defaults to 1. |
| `-n`, `--nwscript <file>` | Path to that game's `nwscript.nss`. |
| `-i`, `--include <dir>` | Directory to search for included scripts. Repeat to add more. |
| `-O`, `--optimize` | Fold constants and drop unreachable code. |

With no `--nwscript`, the CLI looks for `nwscript.nss` next to its own binary,
the way the original compiler did.

## Why you have to supply `nwscript.nss`

An engine routine is called by index, not by name. `ActionAttack` compiles to
"call routine 37", and 37 is that prototype's position in `nwscript.nss`.

KOTOR II added routines and did not preserve every KOTOR I index. Compile a
KOTOR II script against the KOTOR I table and you get a file that loads, runs,
and calls the wrong engine function. There is no error to catch.

So the routine table is data you supply per game. It is also game data, which
is why this repository ships none of it. Take `nwscript.nss` from your own
installation, or from a modding tool that extracted it.

## Depend on it from BioPatcher

The repository is private, so `crates.io` is not involved. Use a git dependency
pinned to a tag:

```toml
[dependencies]
bionsscomp = { git = "https://github.com/bodencrouch/bionsscomp.git", tag = "v1.0.0" }
```

Cargo has to authenticate to reach a private repository. Over HTTPS that means
a git credential helper:

```sh
gh auth setup-git          # once, on an account with access to the repo
cargo build
```

Over SSH instead, if the account has a key loaded:

```toml
[dependencies]
bionsscomp = { git = "ssh://git@github.com/bodencrouch/bionsscomp.git", tag = "v1.0.0" }
```

If Cargo's built-in SSH client cannot authenticate but `git clone` can, hand the
fetch to the git binary:

```toml
# .cargo/config.toml
[net]
git-fetch-with-cli = true
```

For local development against both trees at once, point at the path and skip
the network:

```toml
[dependencies]
bionsscomp = { path = "../bionsscomp/crates/bionsscomp" }
```

A `[patch]` entry keeps the git dependency in `Cargo.toml` while building
against a local checkout:

```toml
[patch."ssh://git@github.com/bodencrouch/bionsscomp.git"]
bionsscomp = { path = "../bionsscomp/crates/bionsscomp" }
```

The library pulls in no third-party crates, so depending on it adds one entry
to your lock file and nothing else.

## Build and test

```sh
cargo build --all-targets
cargo test --all
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
```

Minimum supported Rust version is **1.77**, matching BioPatcher. That is the
last toolchain that targeted Windows 7 directly, and the crate stays on it by
taking no dependencies. CI builds on 1.77.2 on every push, so a change that
raises the floor fails there.

## The golden corpus

The acceptance test compiles a `.nss` and compares the bytes to what
`nwnnsscomp.exe` produced for the same input. Byte equality is the bar;
anything less means a script that loads but behaves differently.

The pairs are game data and are not in this repository. Point the tests at a
corpus:

```sh
BIONSSCOMP_CORPUS=/path/to/corpus cargo test --all
```

The corpus layout:

```text
corpus/
  pairs.jsonl                one JSON object per line
  nwscript/k1/nwscript.nss
  nwscript/k2/nwscript.nss
```

Each line of `pairs.jsonl` names a source, its expected output, and a game:

```json
{"game": "k1", "nss": "nss/k_test.nss", "ncs": "ncs/k_test.ncs"}
```

Keys `nss_path`/`source` and `ncs_path`/`expected` are accepted too. Relative
paths resolve against the corpus root. Without a corpus the test prints what it
skipped and passes, so a fresh checkout is green.

## Layout

```text
crates/bionsscomp/       the library
  src/error.rs           diagnostics, spans, line and column
  src/preprocessor.rs    #include and #define
  src/lexer.rs           text to tokens
  src/parser.rs          tokens to a syntax tree
  src/ast.rs             the syntax tree
  src/sema.rs            names, types, stack slots
  src/codegen.rs         tree to instructions
  src/ncs.rs             the compiled script container
  src/nwscript.rs        per-game routine tables
  tests/pipeline.rs      end to end checks on committed fixtures
  tests/frontend.rs      preprocessor, lexer and parser on the fixtures
  tests/frontend_corpus.rs  parses every script the corpus lists
  tests/golden.rs        byte comparison against the corpus
  tests/fixtures/spec_gaps/  one file per open question, for the Wine oracle
crates/bionsscomp-cli/   the binary
```

## Credit

The compiler this one replaces is Torlack's NWScript compiler, written by
Edward T. Smith and later contributed to the Open Knights Consortium under the
BSD 3-Clause license, then ported to KOTOR by Hazard and by tk102 for stoffe.

bionsscomp is an independent Rust reimplementation. It copies no code. It does
copy behaviour on purpose, down to the diagnostic strings, because mod source
and mod tooling were written against that behaviour. See [NOTICE](NOTICE) for
the full attribution.

## License

MIT. See [LICENSE](LICENSE).

Not affiliated with BioWare, Obsidian Entertainment, LucasArts, or Electronic
Arts. Ships no game data.
