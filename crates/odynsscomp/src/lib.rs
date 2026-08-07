//! Compiles NWScript source (`.nss`) to compiled scripts (`.ncs`) for
//! Star Wars: Knights of the Old Republic I and II.
//!
//! This crate replaces `nwnnsscomp.exe`, a Windows-only binary that Linux and
//! macOS modders otherwise run under wine. It is a library first: a patcher
//! links it and compiles in process, with no subprocess, no temporary files,
//! and no wine.
//!
//! # Byte equality is the acceptance test
//!
//! The output has to be the same bytes `nwnnsscomp.exe` produces, not merely
//! code that runs the same. So this compiler does no optimization at all: the
//! reference gates every optimization it has on a flag the KOTOR fork left
//! permanently off, and matching it means matching that too. See
//! [`codegen`] for the three behaviors that look like bugs and are load
//! bearing.
//!
//! # Compiling from memory
//!
//! ```no_run
//! use odynsscomp::{compile, CompileOptions, Game, NwScript};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let nwscript = NwScript::load(Game::K1, "/path/to/kotor/nwscript.nss")?;
//! let options = CompileOptions::new(&nwscript);
//! let bytes = compile("void main() { }", &options)?;
//! std::fs::write("k_test.ncs", bytes)?;
//! # Ok(())
//! # }
//! ```
//!
//! # Why the game matters
//!
//! Engine functions are called by index, and the index is the function's
//! position in that game's `nwscript.nss`. KOTOR II's table is not KOTOR I's.
//! Compiling against the wrong one produces a file that loads and calls the
//! wrong engine function. So [`CompileOptions`] requires a loaded
//! [`NwScript`]; there is no default and no guess.
//!
//! # Where `nwscript.nss` comes from
//!
//! The player's own installation. This crate ships no game data.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(missing_debug_implementations)]

pub mod ast;
pub mod codegen;
pub mod error;
pub mod lexer;
pub mod ncs;
pub mod nwscript;
pub mod parser;
pub mod preprocessor;
pub mod sema;

use std::path::{Path, PathBuf};

pub use crate::codegen::OptimizationLevel;
pub use crate::error::{CompileError, Diagnostic, Position, Severity, Span, Stage};
pub use crate::nwscript::{Game, NwScript, Routine};

/// The version of this crate.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Everything a compile needs besides the source text.
///
/// Built from a loaded [`NwScript`], which fixes the game.
#[derive(Debug, Clone)]
pub struct CompileOptions<'a> {
    nwscript: &'a NwScript,
    include_paths: Vec<PathBuf>,
    optimization: OptimizationLevel,
    source_name: String,
}

impl<'a> CompileOptions<'a> {
    /// Builds options against a loaded routine table.
    pub fn new(nwscript: &'a NwScript) -> Self {
        CompileOptions {
            nwscript,
            include_paths: Vec::new(),
            optimization: OptimizationLevel::None,
            source_name: "<memory>".to_string(),
        }
    }

    /// Adds a directory to search for `#include` files.
    ///
    /// Directories are searched in the order they are added.
    pub fn with_include_path(mut self, directory: impl Into<PathBuf>) -> Self {
        self.include_paths.push(directory.into());
        self
    }

    /// Sets how hard code generation works at shrinking the output.
    pub fn with_optimization(mut self, optimization: OptimizationLevel) -> Self {
        self.optimization = optimization;
        self
    }

    /// Sets the file name diagnostics report.
    ///
    /// [`compile_file`] sets this for you.
    pub fn with_source_name(mut self, name: impl Into<String>) -> Self {
        self.source_name = name.into();
        self
    }

    /// Returns the routine table this compile uses.
    pub fn nwscript(&self) -> &NwScript {
        self.nwscript
    }

    /// Returns which game this compile targets.
    pub fn game(&self) -> Game {
        self.nwscript.game()
    }

    /// Returns the include search path in order.
    pub fn include_paths(&self) -> &[PathBuf] {
        &self.include_paths
    }

    /// Returns the optimization level.
    pub fn optimization(&self) -> OptimizationLevel {
        self.optimization
    }

    /// Returns the file name diagnostics report.
    pub fn source_name(&self) -> &str {
        &self.source_name
    }

    fn include_resolver(&self) -> preprocessor::IncludeResolver {
        let mut resolver = preprocessor::IncludeResolver::new();
        for directory in &self.include_paths {
            resolver.push_path(directory.clone());
        }
        resolver
    }
}

/// What a compile produced.
#[derive(Debug, Clone)]
pub struct Compilation {
    /// The compiled script, header included.
    pub ncs: Vec<u8>,
    /// Warnings raised along the way. Errors come back as [`CompileError`].
    pub warnings: Vec<Diagnostic>,
    /// Every file the preprocessor pulled in, in the order it did.
    pub included_files: Vec<PathBuf>,
}

/// Compiles source text and returns the compiled script bytes.
///
/// This is the call a patcher makes. The bytes are a complete `.ncs` file,
/// ready to write or to stuff into an archive.
///
/// # Errors
///
/// Returns the diagnostics that stopped the compile. Nothing panics, and no
/// stage writes to disk.
pub fn compile(source: &str, options: &CompileOptions<'_>) -> Result<Vec<u8>, CompileError> {
    compile_detailed(source, options).map(|compilation| compilation.ncs)
}

/// Compiles source text and returns the bytes plus warnings.
///
/// # Errors
///
/// Returns the diagnostics that stopped the compile.
pub fn compile_detailed(
    source: &str,
    options: &CompileOptions<'_>,
) -> Result<Compilation, CompileError> {
    let origin = PathBuf::from(options.source_name());
    let resolver = options.include_resolver();

    let mut expansion = preprocessor::expand(source, &origin, &resolver)?;
    let mut warnings = expansion.warnings.clone();

    // A script never includes `nwscript.nss`, so the engine structure names it
    // declares have to be handed over here. Without them `effect` is an
    // ordinary identifier and every declaration that uses one fails to parse.
    expansion
        .engine_structures
        .merge_defaults(options.nwscript().engine_structures());

    let lexed = lexer::scan(&expansion.source, &expansion.engine_structures);
    if lexed.has_errors() {
        return Err(CompileError::Diagnostics(
            expansion.localize_all(lexed.diagnostics),
        ));
    }
    warnings.extend(expansion.localize_all(lexed.diagnostics));

    let parsed = parser::parse_detailed(lexed.tokens);
    if parsed.has_errors() {
        return Err(CompileError::Diagnostics(
            expansion.localize_all(parsed.diagnostics),
        ));
    }
    warnings.extend(expansion.localize_all(parsed.diagnostics));

    // The back end works on the expanded buffer too, so its diagnostics need
    // the same rewrite to name the file and line the code really came from.
    let analyzed = sema::analyze(parsed.script, options.nwscript())
        .map_err(|error| localize(error, &expansion))?;
    let ncs = codegen::generate(&analyzed, options.optimization())
        .map_err(|error| localize(error, &expansion))?;

    Ok(Compilation {
        ncs,
        warnings,
        included_files: expansion.included_files,
    })
}

/// Points a failed compile's diagnostics at the files they came from.
fn localize(error: CompileError, expansion: &preprocessor::Expansion) -> CompileError {
    match error {
        CompileError::Diagnostics(list) => CompileError::Diagnostics(expansion.localize_all(list)),
        other => other,
    }
}

/// Compiles a `.nss` file from disk.
///
/// The file's own directory is searched for includes first, ahead of anything
/// in `options`. Diagnostics report the file's name.
///
/// # Errors
///
/// Returns [`CompileError::Io`] when the file cannot be read, or the
/// diagnostics that stopped the compile.
pub fn compile_file(
    path: impl AsRef<Path>,
    options: &CompileOptions<'_>,
) -> Result<Vec<u8>, CompileError> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|error| CompileError::Io {
        path: path.display().to_string(),
        message: error.to_string(),
    })?;
    let source = preprocessor::decode_source(&bytes);

    let mut options = options.clone();
    if let Some(directory) = path.parent() {
        options.include_paths.insert(0, directory.to_path_buf());
    }
    options.source_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());

    compile(&source, &options)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_crate_reports_version_one() {
        assert!(VERSION.starts_with("1."));
    }

    #[test]
    fn options_carry_the_game_from_the_routine_table() {
        let nwscript = NwScript::empty(Game::K2);
        let options = CompileOptions::new(&nwscript);
        assert_eq!(options.game(), Game::K2);
        assert_eq!(options.optimization(), OptimizationLevel::None);
        assert!(options.include_paths().is_empty());
    }

    #[test]
    fn include_paths_keep_the_order_they_were_added() {
        let nwscript = NwScript::empty(Game::K1);
        let options = CompileOptions::new(&nwscript)
            .with_include_path("first")
            .with_include_path("second")
            .with_source_name("k_test.nss");
        assert_eq!(options.include_paths().len(), 2);
        assert!(options.include_paths()[0].ends_with("first"));
        assert_eq!(options.source_name(), "k_test.nss");
    }

    #[test]
    fn a_missing_source_file_reports_the_path() {
        let nwscript = NwScript::empty(Game::K1);
        let options = CompileOptions::new(&nwscript);
        let error = compile_file("does/not/exist.nss", &options).expect_err("the file is missing");
        match error {
            CompileError::Io { path, .. } => assert!(path.contains("exist.nss")),
            other => panic!("expected an io error, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_main_compiles_to_the_smallest_script_there_is() {
        let nwscript = NwScript::empty(Game::K1);
        let options = CompileOptions::new(&nwscript);
        let bytes = compile("void main() { }", &options).expect("an empty main compiles");
        assert_eq!(bytes.len(), 23);
        assert_eq!(&bytes[0..8], b"NCS V1.0");
    }

    #[test]
    fn a_file_with_no_entry_point_is_refused_as_an_include() {
        let nwscript = NwScript::empty(Game::K1);
        let options = CompileOptions::new(&nwscript);
        let error = compile("void Helper() { }", &options).expect_err("there is no entry point");
        assert!(error.to_string().contains("include file"));
    }
}
