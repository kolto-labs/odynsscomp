//! Golden file tests: compile a `.nss` and compare the bytes to a `.ncs` that
//! `nwnnsscomp.exe` produced.
//!
//! Byte equality with the original compiler is the acceptance bar. Anything
//! less means a script that loads but behaves differently, which is worse than
//! a script that fails to build.
//!
//! # The corpus is not in this repository
//!
//! The pairs are game data and compiler output, neither of which is
//! redistributable, so they live outside the tree. Point the tests at them
//! with `ODYNSSCOMP_CORPUS`, or leave them at the default path below.
//!
//! ```text
//! corpus/
//!   pairs.jsonl          one JSON object per line, described in Pair::parse
//!   nwscript/k1/nwscript.nss
//!   nwscript/k2/nwscript.nss
//! ```
//!
//! When the corpus is absent these tests print what they skipped and pass.
//! That keeps CI green on a checkout that has no game files.

use std::path::{Path, PathBuf};

use odynsscomp::{compile_file, CompileOptions, Game, NwScript};

/// Returns the corpus root, or `None` when it is not on this machine.
fn corpus_root() -> Option<PathBuf> {
    let candidate = match std::env::var_os("ODYNSSCOMP_CORPUS") {
        Some(value) => PathBuf::from(value),
        None => {
            let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
            PathBuf::from(home)
                .join("Workspaces")
                .join("odynsscomp_research")
                .join("corpus")
        }
    };

    if candidate.join("pairs.jsonl").is_file() {
        Some(candidate)
    } else {
        None
    }
}

/// Returns the `nwscript.nss` for `game`, or `None` when the corpus lacks it.
fn corpus_nwscript(root: &Path, game: Game) -> Option<PathBuf> {
    let path = root
        .join("nwscript")
        .join(game.short_name())
        .join("nwscript.nss");
    path.is_file().then_some(path)
}

/// One source and its expected output.
#[derive(Debug)]
struct Pair {
    game: Game,
    nss: PathBuf,
    ncs: PathBuf,
}

impl Pair {
    /// Reads one line of `pairs.jsonl`.
    ///
    /// The line is a flat JSON object. Recognized keys, first match wins:
    ///
    /// - source: `nss`, `nss_path`, or `source`
    /// - expected output: `ncs`, `ncs_path`, or `expected`
    /// - game: `game`, accepting `1`, `2`, `k1`, `k2`, `tsl`
    ///
    /// Relative paths resolve against the corpus root.
    fn parse(line: &str, root: &Path) -> Option<Pair> {
        let nss = json_string(line, "nss")
            .or_else(|| json_string(line, "nss_path"))
            .or_else(|| json_string(line, "source"))?;
        let ncs = json_string(line, "ncs")
            .or_else(|| json_string(line, "ncs_path"))
            .or_else(|| json_string(line, "expected"))?;
        let game = json_string(line, "game")
            .and_then(|value| Game::parse(&value))
            .unwrap_or_default();

        Some(Pair {
            game,
            nss: resolve(root, &nss),
            ncs: resolve(root, &ncs),
        })
    }
}

fn resolve(root: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

/// Pulls a string or number field out of a flat JSON object.
///
/// This is deliberately small. The crate takes no dependencies, and the corpus
/// manifest is one flat object per line. Replace it if the format grows.
fn json_string(line: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let start = line.find(&needle)? + needle.len();
    let rest = line[start..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();

    if let Some(body) = rest.strip_prefix('"') {
        let mut out = String::new();
        let mut chars = body.chars();
        while let Some(ch) = chars.next() {
            match ch {
                '"' => return Some(out),
                '\\' => match chars.next()? {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'r' => out.push('\r'),
                    other => out.push(other),
                },
                other => out.push(other),
            }
        }
        None
    } else {
        let end = rest.find([',', '}']).unwrap_or(rest.len());
        let value = rest[..end].trim();
        (!value.is_empty()).then(|| value.to_string())
    }
}

/// Loads the corpus manifest.
fn load_pairs(root: &Path) -> Vec<Pair> {
    let text = std::fs::read_to_string(root.join("pairs.jsonl")).unwrap_or_default();
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| Pair::parse(line, root))
        .collect()
}

/// Loads a game's routine table once, and hands it out for every pair.
///
/// Reading and parsing a 2300-line `nwscript.nss` for each of a few thousand
/// pairs is most of the run time, and the answer never changes.
fn load_tables(root: &Path) -> Result<(NwScript, NwScript), String> {
    let mut tables = Vec::new();
    for game in [Game::K1, Game::K2] {
        let path = corpus_nwscript(root, game)
            .ok_or_else(|| format!("the corpus has no nwscript.nss for {game}"))?;
        let table =
            NwScript::load(game, &path).map_err(|error| format!("{}: {error}", path.display()))?;
        tables.push(table);
    }
    let k2 = tables.pop().expect("both tables loaded");
    let k1 = tables.pop().expect("both tables loaded");
    Ok((k1, k2))
}

/// Compiles the whole corpus and requires the bytes to match what shipped.
///
/// Ignored by default, for two reasons. It is minutes of IO over a corpus that
/// is not in this repository, and the shipped bytes are not a clean oracle:
/// roughly one pair in fifteen was built from a revision of the `.nss` that is
/// not the one sitting beside it, so even a perfect compiler misses those. The
/// bar that has no such slack is `differential.rs`, which asks the original
/// compiler directly.
///
/// ```sh
/// cargo test --test golden -- --ignored --nocapture
/// ```
#[test]
#[ignore = "needs the corpus, and shipped bytes carry source drift; see the doc comment"]
fn compiled_output_matches_the_original_compiler() {
    let Some(root) = corpus_root() else {
        eprintln!("skipping: no corpus. Set ODYNSSCOMP_CORPUS to a directory holding pairs.jsonl.");
        return;
    };

    let mut pairs = load_pairs(&root);
    if pairs.is_empty() {
        eprintln!("skipping: {} holds no readable pairs", root.display());
        return;
    }
    if let Some(limit) = std::env::var("ODYNSSCOMP_CORPUS_LIMIT")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
    {
        pairs.truncate(limit);
    }

    let (k1, k2) = match load_tables(&root) {
        Ok(tables) => tables,
        Err(reason) => {
            eprintln!("skipping: {reason}");
            return;
        }
    };

    let mut checked = 0usize;
    let mut failures: Vec<String> = Vec::new();

    for pair in &pairs {
        let nwscript = match pair.game {
            Game::K1 => &k1,
            Game::K2 => &k2,
        };

        let options = CompileOptions::new(nwscript);
        let actual = match compile_file(&pair.nss, &options) {
            Ok(bytes) => bytes,
            Err(error) => {
                failures.push(format!("{}: {error}", pair.nss.display()));
                continue;
            }
        };

        let expected = match std::fs::read(&pair.ncs) {
            Ok(bytes) => bytes,
            Err(error) => {
                failures.push(format!("{}: {error}", pair.ncs.display()));
                continue;
            }
        };

        checked += 1;
        if actual != expected {
            failures.push(describe_mismatch(pair, &actual, &expected));
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} pairs failed:\n{}",
        failures.len(),
        pairs.len(),
        failures.join("\n")
    );

    eprintln!("{checked} of {} pairs matched byte for byte", pairs.len());
}

/// Describes the first byte that differs, so a failure says something useful.
fn describe_mismatch(pair: &Pair, actual: &[u8], expected: &[u8]) -> String {
    let first_difference = actual
        .iter()
        .zip(expected.iter())
        .position(|(left, right)| left != right);

    match first_difference {
        Some(offset) => format!(
            "{}: byte {offset} is {:#04x}, expected {:#04x} ({} bytes vs {})",
            pair.nss.display(),
            actual[offset],
            expected[offset],
            actual.len(),
            expected.len()
        ),
        None => format!(
            "{}: output is {} bytes, expected {}",
            pair.nss.display(),
            actual.len(),
            expected.len()
        ),
    }
}

#[test]
fn the_manifest_parser_reads_the_documented_shape() {
    let root = Path::new("/corpus");
    let line = r#"{"game": "k2", "nss": "nss/k_test.nss", "ncs": "ncs/k_test.ncs"}"#;
    let pair = Pair::parse(line, root).expect("the line parses");

    assert_eq!(pair.game, Game::K2);
    assert_eq!(pair.nss, PathBuf::from("/corpus/nss/k_test.nss"));
    assert_eq!(pair.ncs, PathBuf::from("/corpus/ncs/k_test.ncs"));
}

#[test]
fn the_manifest_parser_accepts_the_alternate_key_names() {
    let root = Path::new("/corpus");
    let line = r#"{"game": 1, "source": "a.nss", "expected": "a.ncs", "note": "vanilla"}"#;
    let pair = Pair::parse(line, root).expect("the line parses");

    assert_eq!(pair.game, Game::K1);
    assert!(pair.nss.ends_with("a.nss"));
}

#[test]
fn the_manifest_parser_keeps_absolute_paths() {
    let root = Path::new("/corpus");
    let line = r#"{"nss": "/elsewhere/a.nss", "ncs": "/elsewhere/a.ncs"}"#;
    let pair = Pair::parse(line, root).expect("the line parses");

    assert_eq!(pair.nss, PathBuf::from("/elsewhere/a.nss"));
    assert_eq!(pair.game, Game::K1, "an absent game falls back to k1");
}

#[test]
fn a_line_missing_a_path_is_dropped() {
    assert!(Pair::parse(r#"{"nss": "a.nss"}"#, Path::new("/corpus")).is_none());
}
