//! Turns NWScript source text into tokens.
//!
//! # Status
//!
//! Scaffold. The token vocabulary below is settled; [`Lexer::tokenize`] is not
//! written yet. Whoever implements it should keep these rules in mind:
//!
//! - NWScript is C-like: `//` line comments, `/* */` block comments that do not
//!   nest, and case-sensitive identifiers.
//! - String literals use double quotes and have no escape sequences in the
//!   original compiler. Do not "fix" that; scripts depend on it.
//! - `vector` literals are written `[1.0, 2.0, 3.0]`.
//! - Floats accept a trailing `f`, as in `1.0f`.
//! - Integer literals accept decimal, `0x` hex, and a leading `0` octal form.

use crate::error::{CompileError, Span};

/// What kind of token this is.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    /// An identifier or a keyword. Keyword classification happens in the parser.
    Identifier(String),
    /// An integer literal, already converted.
    Int(i32),
    /// A float literal, already converted.
    Float(f32),
    /// A string literal with the surrounding quotes removed.
    Str(String),
    /// Punctuation or an operator.
    Punct(Punct),
    /// The end of the token stream.
    Eof,
}

/// Punctuation and operators, in the set NWScript actually uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum Punct {
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Semicolon,
    Comma,
    Dot,
    Question,
    Colon,
    Assign,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    PlusAssign,
    MinusAssign,
    StarAssign,
    SlashAssign,
    PercentAssign,
    Increment,
    Decrement,
    Eq,
    NotEq,
    Less,
    LessEq,
    Greater,
    GreaterEq,
    AndAnd,
    OrOr,
    Not,
    Amp,
    Pipe,
    Caret,
    Tilde,
    Shl,
    Shr,
    /// `>>>`, the unsigned right shift NWScript inherits from Java-style syntax.
    UShr,
    AmpAssign,
    PipeAssign,
    CaretAssign,
    ShlAssign,
    ShrAssign,
    UShrAssign,
}

/// One token plus where it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    /// What the token is.
    pub kind: TokenKind,
    /// Where the token sits in the source text.
    pub span: Span,
}

impl Token {
    /// Builds a token.
    pub fn new(kind: TokenKind, span: Span) -> Self {
        Token { kind, span }
    }

    /// Returns true when this token ends the stream.
    pub fn is_eof(&self) -> bool {
        matches!(self.kind, TokenKind::Eof)
    }
}

/// Scans source text into [`Token`]s.
#[derive(Debug, Clone)]
pub struct Lexer<'a> {
    source: &'a str,
    offset: usize,
}

impl<'a> Lexer<'a> {
    /// Builds a lexer over `source`.
    pub fn new(source: &'a str) -> Self {
        Lexer { source, offset: 0 }
    }

    /// Returns the full source text.
    pub fn source(&self) -> &'a str {
        self.source
    }

    /// Returns the byte offset the lexer has reached.
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Returns the text the lexer has not consumed.
    pub fn remaining(&self) -> &'a str {
        &self.source[self.offset.min(self.source.len())..]
    }

    /// Scans the whole input.
    ///
    /// The returned vector always ends with a [`TokenKind::Eof`] token.
    ///
    /// # Errors
    ///
    /// Returns every lexing error found, not just the first.
    pub fn tokenize(&mut self) -> Result<Vec<Token>, CompileError> {
        // TODO(lexer): implement scanning. See the module docs for the rules
        // that differ from C.
        Err(CompileError::NotImplemented("the lexer"))
    }
}

/// Scans `source` in one call.
///
/// # Errors
///
/// Returns every lexing error found, not just the first.
pub fn tokenize(source: &str) -> Result<Vec<Token>, CompileError> {
    Lexer::new(source).tokenize()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_lexer_has_consumed_nothing() {
        let lexer = Lexer::new("void main() {}");
        assert_eq!(lexer.offset(), 0);
        assert_eq!(lexer.remaining(), "void main() {}");
    }

    #[test]
    fn tokenize_reports_that_it_is_unwritten_instead_of_panicking() {
        let error = tokenize("void main() {}").expect_err("the lexer is a stub");
        assert!(error.is_not_implemented());
    }
}
