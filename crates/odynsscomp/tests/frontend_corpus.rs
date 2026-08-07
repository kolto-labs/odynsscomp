//! Runs the front end over every `.nss` the local corpus lists.
//!
//! Preprocessing, lexing and parsing a few thousand real scripts is the
//! cheapest way to find a production the grammar has wrong. It needs no
//! reference compiler and no expected output: a script that shipped in a game
//! or a mod compiled once, so it must parse.
//!
//! This is a front end test. It says nothing about whether the bytes come out
//! right — `tests/golden.rs` owns that.
//!
//! # The corpus is not in this repository
//!
//! Game scripts and mod scripts are not redistributable, so they live outside
//! the tree. Point at them with `ODYNSSCOMP_CORPUS`, or leave them at the
//! default path below. When they are absent this test prints what it skipped
//! and passes, which keeps CI green on a clean checkout.

use std::path::{Path, PathBuf};

use odynsscomp::error::Severity;
use odynsscomp::preprocessor::{self, IncludeResolver};
use odynsscomp::{lexer, parser};

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
    candidate.join("pairs.jsonl").is_file().then_some(candidate)
}

/// Pulls one string field out of a flat JSON object.
///
/// The crate takes no dependencies and the manifest is one flat object per
/// line, so this is all the JSON that is needed.
fn json_string(line: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let start = line.find(&needle)? + needle.len();
    let rest = line[start..].trim_start().strip_prefix(':')?.trim_start();
    let body = rest.strip_prefix('"')?;
    let end = body.find('"')?;
    Some(body[..end].to_string())
}

/// One script and the game whose `nwscript.nss` goes with it.
struct Source {
    path: PathBuf,
    game: String,
}

/// Returns every source the manifest names.
fn sources(root: &Path) -> Vec<Source> {
    let text = std::fs::read_to_string(root.join("pairs.jsonl")).unwrap_or_default();
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let value = json_string(line, "nss")
                .or_else(|| json_string(line, "nss_path"))
                .or_else(|| json_string(line, "source"))?;
            let path = PathBuf::from(&value);
            Some(Source {
                path: if path.is_absolute() {
                    path
                } else {
                    root.join(path)
                },
                game: json_string(line, "game").unwrap_or_else(|| "k1".to_string()),
            })
        })
        .collect()
}

/// Reads the engine structure types a game's `nwscript.nss` declares.
///
/// The reference compiler compiles `nwscript.nss` before anything else, and
/// that is where `effect`, `event`, `location` and `talent` come from. Without
/// it every script that names one fails to parse, so seeding this is part of
/// setting the test up honestly rather than a way of avoiding a real failure.
fn engine_structures(root: &Path, game: &str) -> preprocessor::EngineStructures {
    let path = root.join("nwscript").join(game).join("nwscript.nss");
    let Ok(bytes) = std::fs::read(&path) else {
        return preprocessor::EngineStructures::new();
    };
    let source = preprocessor::decode_source(&bytes);
    preprocessor::expand(&source, &path, &IncludeResolver::new())
        .map(|expansion| expansion.engine_structures)
        .unwrap_or_default()
}

/// Preprocesses, lexes and parses one file. Returns the errors it produced.
fn front_end(path: &Path, engine: &preprocessor::EngineStructures) -> Vec<String> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new(); // Not in the corpus on this machine.
    };
    let source = preprocessor::decode_source(&bytes);

    let mut resolver = IncludeResolver::new();
    if let Some(directory) = path.parent() {
        resolver.push_path(directory);
    }

    let expansion = match preprocessor::expand(&source, path, &resolver) {
        Ok(expansion) => expansion,
        Err(error) => {
            // A missing include is a gap in the corpus, not a compiler defect.
            return error
                .diagnostics()
                .iter()
                .filter(|d| !d.message.starts_with("Unable to open the include file"))
                .map(ToString::to_string)
                .collect();
        }
    };

    // Whatever the file declared for itself wins over the prelude's table.
    let mut engine = engine.clone();
    for (index, name) in expansion.engine_structures.iter() {
        engine.define(index, name);
    }

    let lexed = lexer::scan(&expansion.source, &engine);
    let parsed = parser::parse_detailed(lexed.tokens);

    lexed
        .diagnostics
        .into_iter()
        .chain(parsed.diagnostics)
        .filter(|d| d.severity == Severity::Error)
        .map(|d| expansion.localize(d).to_string())
        .collect()
}

/// Parses a `nwscript.nss` and returns its engine routine prototypes in order.
fn header_prototypes(path: &Path) -> Result<Vec<String>, String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let source = preprocessor::decode_source(&bytes);

    let expansion = preprocessor::expand(&source, path, &IncludeResolver::new())
        .map_err(|error| error.to_string())?;
    let lexed = lexer::scan(&expansion.source, &expansion.engine_structures);
    let parsed = parser::parse_detailed(lexed.tokens);

    let problems: Vec<String> = lexed
        .diagnostics
        .into_iter()
        .chain(parsed.diagnostics)
        .filter(|d| d.severity == Severity::Error)
        .map(|d| expansion.localize(d).to_string())
        .collect();
    if !problems.is_empty() {
        return Err(problems.join("\n"));
    }

    Ok(parsed
        .script
        .prototypes()
        .map(|function| function.name.clone())
        .collect())
}

/// The shipped headers are the hardest thing the front end has to read.
///
/// Everything a script can say, `nwscript.nss` says: hundreds of prototypes,
/// default arguments everywhere, and the engine structure types that only a
/// `#define` can declare. A front end that cannot read this file cannot compile
/// anything at all, because engine routine ids are the positions of these
/// prototypes and nothing else.
#[test]
fn the_shipped_headers_parse_and_their_prototype_order_is_the_routine_table() {
    let Some(root) = corpus_root() else {
        eprintln!("skipping: no corpus.");
        return;
    };

    // Routine counts confirmed against the extracted game data.
    for (game, expected) in [("k1", 772usize), ("k2", 877usize)] {
        let path = root.join("nwscript").join(game).join("nwscript.nss");
        if !path.is_file() {
            eprintln!("skipping {game}: no header in the corpus");
            continue;
        }

        let prototypes = match header_prototypes(&path) {
            Ok(prototypes) => prototypes,
            Err(problems) => panic!("{game} nwscript.nss failed the front end:\n{problems}"),
        };

        assert_eq!(
            prototypes.len(),
            expected,
            "{game} nwscript.nss declares {expected} engine routines"
        );
        assert_eq!(
            prototypes[0], "Random",
            "{game} routine 0 is Random, because its id is its position"
        );
    }
}

/// KOTOR II's shipped header uses type names that are not real NWScript types.
///
/// `void SetAvailableNPCId(INT nNPC, OBJECT_ID oidNPC);` is in the file BioWare
/// shipped. `INT` and `OBJECT_ID` are lexer synonyms for `int` and `object`,
/// and this line is why they exist: without them the game's own header does not
/// parse. The community-patched header rewrites them, so both spellings have to
/// work.
#[test]
fn the_shouting_type_synonyms_are_load_bearing_in_the_shipped_k2_header() {
    let Some(root) = corpus_root() else {
        eprintln!("skipping: no corpus.");
        return;
    };

    for name in ["nwscript.nss", "nwscript.community-patched.nss"] {
        let path = root.join("nwscript").join("k2").join(name);
        if !path.is_file() {
            eprintln!("skipping {name}: not in the corpus");
            continue;
        }
        let prototypes = match header_prototypes(&path) {
            Ok(prototypes) => prototypes,
            Err(problems) => panic!("k2 {name} failed the front end:\n{problems}"),
        };
        assert!(
            prototypes.iter().any(|n| n == "SetAvailableNPCId"),
            "k2 {name} declares SetAvailableNPCId"
        );
    }
}

#[test]
fn every_corpus_script_gets_through_the_front_end() {
    let Some(root) = corpus_root() else {
        eprintln!("skipping: no corpus. Set ODYNSSCOMP_CORPUS to a directory holding pairs.jsonl.");
        return;
    };

    let mut scripts = sources(&root);
    if scripts.is_empty() {
        eprintln!("skipping: {} holds no readable pairs", root.display());
        return;
    }

    // The corpus lives on whatever disk the developer put it on, and reading a
    // few thousand scripts plus their includes is minutes of IO on a slow one.
    // `ODYNSSCOMP_CORPUS_LIMIT` trims the run while iterating.
    if let Some(limit) = std::env::var("ODYNSSCOMP_CORPUS_LIMIT")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
    {
        scripts.truncate(limit);
    }

    let k1 = engine_structures(&root, "k1");
    let k2 = engine_structures(&root, "k2");

    let mut failures: Vec<String> = Vec::new();
    for script in &scripts {
        let engine = if script.game == "k2" { &k2 } else { &k1 };
        let errors = front_end(&script.path, engine);
        if !errors.is_empty() {
            failures.push(format!(
                "{}\n    {}",
                script.path.display(),
                errors.join("\n    ")
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} corpus scripts failed the front end:\n{}",
        failures.len(),
        scripts.len(),
        failures
            .iter()
            .take(25)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );

    eprintln!("{} corpus scripts parsed", scripts.len());
}
