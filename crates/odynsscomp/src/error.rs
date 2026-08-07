//! Diagnostics: source positions, severities, and the public error type.
//!
//! Every stage of the pipeline reports problems through [`Diagnostic`]. A
//! diagnostic always carries a [`Span`] so callers can point at the offending
//! text, and `nwnnsscomp.exe`-style one-line messages come out of the
//! [`std::fmt::Display`] implementations.
//!
//! This module is fully implemented. The compiler stages are not; see the
//! module docs in [`crate::lexer`] and friends.

use std::error::Error;
use std::fmt;

/// A byte offset plus the 1-based line and column it lands on.
///
/// Columns count characters, not bytes, so a tab is one column. `nwnnsscomp`
/// reports 1-based line numbers and no columns; we keep columns because they
/// cost nothing and make editor integration possible later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Position {
    /// Byte offset from the start of the source text.
    pub offset: usize,
    /// 1-based line number.
    pub line: u32,
    /// 1-based column number, counted in characters.
    pub column: u32,
}

impl Position {
    /// The very start of a source file.
    pub const START: Position = Position {
        offset: 0,
        line: 1,
        column: 1,
    };

    /// Builds a position from its parts.
    pub const fn new(offset: usize, line: u32, column: u32) -> Self {
        Position {
            offset,
            line,
            column,
        }
    }

    /// Computes the position of `offset` inside `source`.
    ///
    /// Offsets past the end of `source` clamp to the end. Offsets that land in
    /// the middle of a multi-byte character round down to that character.
    pub fn from_offset(source: &str, offset: usize) -> Self {
        let offset = offset.min(source.len());
        let mut line = 1u32;
        let mut column = 1u32;
        let mut last = 0usize;

        for (index, ch) in source.char_indices() {
            if index >= offset {
                return Position::new(index, line, column);
            }
            last = index + ch.len_utf8();
            if ch == '\n' {
                line += 1;
                column = 1;
            } else {
                column += 1;
            }
        }

        let _ = last;
        Position::new(offset, line, column)
    }
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

/// A half-open range of source text, `start..end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    /// First position covered by the span.
    pub start: Position,
    /// One past the last position covered by the span.
    pub end: Position,
}

impl Span {
    /// Builds a span from two positions.
    pub const fn new(start: Position, end: Position) -> Self {
        Span { start, end }
    }

    /// Builds an empty span at `position`.
    pub const fn point(position: Position) -> Self {
        Span {
            start: position,
            end: position,
        }
    }

    /// Returns the smallest span covering both inputs.
    pub fn join(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    /// Returns the source text this span covers, or `None` if it does not fit.
    pub fn text<'a>(&self, source: &'a str) -> Option<&'a str> {
        source.get(self.start.offset..self.end.offset)
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.start)
    }
}

/// How bad a diagnostic is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// The compile fails.
    Error,
    /// The compile continues.
    Warning,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Severity::Error => f.write_str("Error"),
            Severity::Warning => f.write_str("Warning"),
        }
    }
}

/// Which stage produced a diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// `#include` and `#define` expansion.
    Preprocessor,
    /// Turning text into tokens.
    Lexer,
    /// Turning tokens into a syntax tree.
    Parser,
    /// Name resolution and type checking.
    Sema,
    /// Lowering the tree to stack machine instructions.
    Codegen,
    /// Writing the `.ncs` byte stream.
    Ncs,
    /// Loading a per-game `nwscript.nss` routine table.
    NwScript,
    /// Reading or writing files.
    Io,
}

impl fmt::Display for Stage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Stage::Preprocessor => "preprocessor",
            Stage::Lexer => "lexer",
            Stage::Parser => "parser",
            Stage::Sema => "sema",
            Stage::Codegen => "codegen",
            Stage::Ncs => "ncs",
            Stage::NwScript => "nwscript",
            Stage::Io => "io",
        };
        f.write_str(name)
    }
}

/// One problem found in one source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Whether this stops the compile.
    pub severity: Severity,
    /// The stage that raised it.
    pub stage: Stage,
    /// Where it happened.
    pub span: Span,
    /// The file it happened in, as written by the caller. Empty when unknown.
    pub file: String,
    /// One line, no trailing period, no leading severity word.
    pub message: String,
}

impl Diagnostic {
    /// Builds an error diagnostic.
    pub fn error(stage: Stage, span: Span, message: impl Into<String>) -> Self {
        Diagnostic {
            severity: Severity::Error,
            stage,
            span,
            file: String::new(),
            message: message.into(),
        }
    }

    /// Builds a warning diagnostic.
    pub fn warning(stage: Stage, span: Span, message: impl Into<String>) -> Self {
        Diagnostic {
            severity: Severity::Warning,
            stage,
            span,
            file: String::new(),
            message: message.into(),
        }
    }

    /// Attaches a file name and returns the diagnostic.
    pub fn with_file(mut self, file: impl Into<String>) -> Self {
        self.file = file.into();
        self
    }
}

impl fmt::Display for Diagnostic {
    /// Formats as `file(line): Error: message`, matching what mod authors are
    /// used to reading out of `nwnnsscomp` logs.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.file.is_empty() {
            write!(
                f,
                "({}): {}: {}",
                self.span.start.line, self.severity, self.message
            )
        } else {
            write!(
                f,
                "{}({}): {}: {}",
                self.file, self.span.start.line, self.severity, self.message
            )
        }
    }
}

/// Why a compile failed.
///
/// A failed compile carries every diagnostic the pipeline collected, not just
/// the first one, so a caller can show the whole list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompileError {
    /// The source was rejected. Contains at least one error diagnostic.
    Diagnostics(Vec<Diagnostic>),
    /// A file could not be read or written.
    Io {
        /// The path that failed.
        path: String,
        /// The operating system's explanation.
        message: String,
    },
    /// The requested feature exists in the design but not yet in the code.
    ///
    /// The scaffold returns this from every entry point. It disappears as the
    /// stages land.
    NotImplemented(&'static str),
}

impl CompileError {
    /// Wraps a single diagnostic.
    pub fn from_diagnostic(diagnostic: Diagnostic) -> Self {
        CompileError::Diagnostics(vec![diagnostic])
    }

    /// Returns the diagnostics, or an empty slice for other error kinds.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        match self {
            CompileError::Diagnostics(list) => list,
            _ => &[],
        }
    }

    /// Returns true when this is the scaffold's placeholder error.
    pub fn is_not_implemented(&self) -> bool {
        matches!(self, CompileError::NotImplemented(_))
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompileError::Diagnostics(list) => {
                for (index, diagnostic) in list.iter().enumerate() {
                    if index > 0 {
                        writeln!(f)?;
                    }
                    write!(f, "{diagnostic}")?;
                }
                Ok(())
            }
            CompileError::Io { path, message } => {
                write!(f, "Error: unable to open \"{path}\": {message}")
            }
            CompileError::NotImplemented(what) => {
                write!(f, "Error: {what} is not implemented yet")
            }
        }
    }
}

impl Error for CompileError {}

/// Collects diagnostics across stages.
///
/// The compiler keeps going after an error so one run reports many problems,
/// the way `nwnnsscomp` does up to its 100 error limit.
#[derive(Debug, Clone, Default)]
pub struct DiagnosticSink {
    diagnostics: Vec<Diagnostic>,
    error_count: usize,
}

impl DiagnosticSink {
    /// The error count at which the original compiler gives up.
    pub const ERROR_LIMIT: usize = 100;

    /// Builds an empty sink.
    pub fn new() -> Self {
        DiagnosticSink::default()
    }

    /// Records a diagnostic.
    pub fn push(&mut self, diagnostic: Diagnostic) {
        if diagnostic.severity == Severity::Error {
            self.error_count += 1;
        }
        self.diagnostics.push(diagnostic);
    }

    /// Returns every diagnostic recorded so far.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Returns every diagnostic recorded so far, consuming the sink.
    pub fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }

    /// Returns how many errors were recorded.
    pub fn error_count(&self) -> usize {
        self.error_count
    }

    /// Returns true when at least one error was recorded.
    pub fn has_errors(&self) -> bool {
        self.error_count > 0
    }

    /// Returns true once the error limit is reached and the run should stop.
    pub fn is_exhausted(&self) -> bool {
        self.error_count >= Self::ERROR_LIMIT
    }

    /// Turns the sink into an error when it holds errors, otherwise `Ok`.
    pub fn into_result(self) -> Result<Vec<Diagnostic>, CompileError> {
        if self.has_errors() {
            Err(CompileError::Diagnostics(self.diagnostics))
        } else {
            Ok(self.diagnostics)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_tracks_lines_and_columns() {
        let source = "void main()\n{\n    int x;\n}\n";
        let offset = source.find("int").expect("fixture contains int");
        let position = Position::from_offset(source, offset);
        assert_eq!(position.line, 3);
        assert_eq!(position.column, 5);
        assert_eq!(position.offset, offset);
    }

    #[test]
    fn position_clamps_past_the_end() {
        let source = "int x;";
        let position = Position::from_offset(source, 9_999);
        assert_eq!(position.line, 1);
        assert_eq!(position.offset, source.len());
    }

    #[test]
    fn span_join_covers_both_ends() {
        let source = "alpha beta";
        let left = Span::new(
            Position::from_offset(source, 0),
            Position::from_offset(source, 5),
        );
        let right = Span::new(
            Position::from_offset(source, 6),
            Position::from_offset(source, 10),
        );
        assert_eq!(left.join(right).text(source), Some("alpha beta"));
    }

    #[test]
    fn diagnostics_format_like_the_original_compiler() {
        let diagnostic = Diagnostic::error(
            Stage::Parser,
            Span::point(Position::new(12, 3, 5)),
            "expected \";\"",
        )
        .with_file("k_test.nss");
        assert_eq!(
            diagnostic.to_string(),
            "k_test.nss(3): Error: expected \";\""
        );
    }

    #[test]
    fn sink_reports_errors_but_not_warnings() {
        let mut sink = DiagnosticSink::new();
        sink.push(Diagnostic::warning(Stage::Sema, Span::default(), "unused"));
        assert!(!sink.has_errors());
        sink.push(Diagnostic::error(Stage::Sema, Span::default(), "no main"));
        assert!(sink.has_errors());
        assert_eq!(sink.error_count(), 1);
        assert!(sink.into_result().is_err());
    }
}
