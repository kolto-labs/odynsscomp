//! Expands `#include` and `#define` before lexing.
//!
//! # Status
//!
//! Scaffold. What the original compiler does, and what an implementation has to
//! match:
//!
//! - `#include "name"` pulls in `name.nss`. The extension is implied and the
//!   quotes are required. Includes are resolved against the search path in
//!   order, and a file already included is skipped rather than included twice.
//! - `#define NAME value` is a plain object-like text macro. Function-like
//!   macros are not supported.
//! - There is no `#if`, `#ifdef`, or `#pragma`. Do not add any.
//! - Line numbers must survive expansion, or diagnostics will point at the
//!   wrong place in the wrong file. Keep a source map.

use std::path::{Path, PathBuf};

use crate::error::CompileError;

/// Where the preprocessor looks for included files.
///
/// [`crate::nwscript`] and the compiler entry point both feed this from the
/// caller's options. The order matters: first match wins.
#[derive(Debug, Clone, Default)]
pub struct IncludeResolver {
    search_paths: Vec<PathBuf>,
}

impl IncludeResolver {
    /// Builds a resolver with no search paths.
    pub fn new() -> Self {
        IncludeResolver::default()
    }

    /// Adds a directory to the end of the search path.
    pub fn push_path(&mut self, directory: impl Into<PathBuf>) {
        self.search_paths.push(directory.into());
    }

    /// Returns the search path in order.
    pub fn search_paths(&self) -> &[PathBuf] {
        &self.search_paths
    }

    /// Finds `name` on the search path, adding the `.nss` extension if absent.
    ///
    /// Returns `None` when no directory holds it. Matching is whatever the
    /// host filesystem does, which means include names that differ only in case
    /// resolve on Windows and fail on Linux. Report that as an error rather
    /// than guessing.
    pub fn resolve(&self, name: &str) -> Option<PathBuf> {
        let file_name = if Path::new(name).extension().is_some() {
            name.to_string()
        } else {
            format!("{name}.nss")
        };

        self.search_paths
            .iter()
            .map(|directory| directory.join(&file_name))
            .find(|candidate| candidate.is_file())
    }
}

/// One expanded translation unit plus the map back to the files it came from.
#[derive(Debug, Clone, Default)]
pub struct Expansion {
    /// The expanded source text, ready for the lexer.
    pub source: String,
    /// Every file that contributed, in the order it was pulled in.
    pub included_files: Vec<PathBuf>,
}

/// Expands `source` into a single translation unit.
///
/// `origin` names the file the text came from and is used both for diagnostics
/// and as the first place to look for a relative include.
///
/// # Errors
///
/// Returns an error when an include cannot be found, when includes form a
/// cycle, or when a directive is malformed.
pub fn expand(
    source: &str,
    origin: &Path,
    resolver: &IncludeResolver,
) -> Result<Expansion, CompileError> {
    let _ = (source, origin, resolver);
    // TODO(preprocessor): implement include expansion and object-like macros,
    // keeping a source map so diagnostics point back at the original file.
    Err(CompileError::NotImplemented("the preprocessor"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_resolver_has_no_search_path() {
        let resolver = IncludeResolver::new();
        assert!(resolver.search_paths().is_empty());
        assert!(resolver.resolve("k_inc_generic").is_none());
    }

    #[test]
    fn search_paths_keep_the_order_they_were_added() {
        let mut resolver = IncludeResolver::new();
        resolver.push_path("first");
        resolver.push_path("second");
        assert_eq!(resolver.search_paths().len(), 2);
        assert!(resolver.search_paths()[0].ends_with("first"));
    }

    #[test]
    fn expand_reports_that_it_is_unwritten_instead_of_panicking() {
        let error = expand(
            "void main() {}",
            Path::new("k_test.nss"),
            &IncludeResolver::new(),
        )
        .expect_err("the preprocessor is a stub");
        assert!(error.is_not_implemented());
    }
}
