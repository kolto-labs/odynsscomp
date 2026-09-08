//! Turns tokens into a [`Script`].
//!
//! # Status
//!
//! Scaffold. Notes for whoever writes it:
//!
//! - A hand-written recursive descent parser is the right shape here. The
//!   grammar is small, and no dependency is worth taking for it.
//! - Precedence follows C, with `>>>` sitting alongside `>>`.
//! - Declarations and statements are ambiguous on their first token: `int x;`
//!   at file scope is a global, `int f();` is a prototype. Look ahead past the
//!   name for `(`.
//! - Keep going after an error. Recover at the next `;` or `}` and report the
//!   whole batch through [`crate::error::DiagnosticSink`].

use crate::ast::Script;
use crate::error::CompileError;
use crate::lexer::Token;

/// Builds a [`Script`] from a token stream.
#[derive(Debug, Clone)]
pub struct Parser {
    tokens: Vec<Token>,
    position: usize,
}

impl Parser {
    /// Builds a parser over `tokens`.
    pub fn new(tokens: Vec<Token>) -> Self {
        Parser {
            tokens,
            position: 0,
        }
    }

    /// Returns the tokens the parser was given.
    pub fn tokens(&self) -> &[Token] {
        &self.tokens
    }

    /// Returns the index of the next token to read.
    pub fn position(&self) -> usize {
        self.position
    }

    /// Returns the next token without consuming it.
    pub fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.position)
    }

    /// Parses the whole token stream.
    ///
    /// # Errors
    ///
    /// Returns every syntax error found, not just the first.
    pub fn parse(&mut self) -> Result<Script, CompileError> {
        // TODO(parser): implement recursive descent over the grammar.
        Err(CompileError::NotImplemented("the parser"))
    }
}

/// Parses a token stream in one call.
///
/// # Errors
///
/// Returns every syntax error found, not just the first.
pub fn parse(tokens: Vec<Token>) -> Result<Script, CompileError> {
    Parser::new(tokens).parse()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_parser_is_at_the_start() {
        let parser = Parser::new(Vec::new());
        assert_eq!(parser.position(), 0);
        assert!(parser.peek().is_none());
    }

    #[test]
    fn parse_reports_that_it_is_unwritten_instead_of_panicking() {
        let error = parse(Vec::new()).expect_err("the parser is a stub");
        assert!(error.is_not_implemented());
    }
}
