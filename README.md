# odynsscomp

Compiles NWScript source (`.nss`) into compiled scripts (`.ncs`) for
Star Wars: Knights of the Old Republic I and II.

It replaces `nwnnsscomp.exe`. That binary is Windows-only, so Linux and macOS
modders run it under wine, and a patcher that shells out to it has to ship it,
find it, and hope wine is installed. This is a Rust library instead. A patcher
links it and compiles in process: no subprocess, no temporary files, no wine.

## Status

Scaffold. Version 1.0.0 fixes the public API, the diagnostics, and the compiled
script container. The compiler stages return `CompileError::NotImplemented`
until they land. Nothing panics and nothing writes to disk unless you ask it to.

| Part | State |
| --- | --- |
| Public API, `CompileOptions`, `Game` | done |
| Diagnostics with line and column | done |
| `.ncs` header and byte writer | done |
| Command line argument handling | done |
| Preprocessor, lexer, parser, sema, codegen | stubs |
| `nwscript.nss` routine table parsing | stub |

## Use it as a library

```rust
use odynsscomp::{compile, CompileOptions, Game, NwScript};

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
odynsscomp -c k_test.nss -o k_test.ncs
```

Longer options cover what the original guessed at:

```sh
odynsscomp --game 2 --nwscript /path/to/tsl/nwscript.nss \
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

## Depend on it from OdyPatcher

The repository is private, so `crates.io` is not involved. Use a git dependency
pinned to a tag:

```toml
[dependencies]
odynsscomp = { git = "ssh://git@github.com/bodencrouch/odynsscomp.git", tag = "v1.0.0" }
```

That needs an SSH key on the account with access. Over HTTPS instead, with a
credential helper or a token in the environment:

```toml
[dependencies]
odynsscomp = { git = "https://github.com/bodencrouch/odynsscomp.git", tag = "v1.0.0" }
```

```sh
gh auth setup-git          # once, to teach git the credential helper
cargo build
```

For local development against both trees at once, point at the path and skip
the network:

```toml
[dependencies]
odynsscomp = { path = "../odynsscomp/crates/odynsscomp" }
```

A `[patch]` entry keeps the git dependency in `Cargo.toml` while building
against a local checkout:

```toml
[patch."ssh://git@github.com/bodencrouch/odynsscomp.git"]
odynsscomp = { path = "../odynsscomp/crates/odynsscomp" }
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

Minimum supported Rust version is **1.77**, matching OdyPatcher. That is the
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
ODYNSSCOMP_CORPUS=/path/to/corpus cargo test --all
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
crates/odynsscomp/       the library
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
  tests/golden.rs        byte comparison against the corpus
crates/odynsscomp-cli/   the binary
```

## License

MIT. See [LICENSE](LICENSE).

Not affiliated with BioWare, Obsidian Entertainment, LucasArts, or Electronic
Arts. Ships no game data.
