//! Per-game engine routine tables loaded from `nwscript.nss`.
//!
//! # Why this is not baked in
//!
//! An engine routine is called by index, not by name. `ActionAttack` is
//! "call routine 37" in the byte stream, and the index is the position of the
//! prototype in `nwscript.nss`. KOTOR II added routines and did not keep every
//! KOTOR I index, so a script compiled against the wrong table calls the wrong
//! engine function and the game misbehaves without erroring.
//!
//! So the table is data, supplied per game, not a constant in this crate. It
//! also keeps copyrighted game files out of the repository: the caller points
//! at the `nwscript.nss` from their own installation.
//!
//! # Status
//!
//! Scaffold. File loading and error reporting are real; parsing the prototypes
//! out of the source is not.

use std::fmt;
use std::path::Path;

use crate::ast::Type;
use crate::error::{CompileError, Span};

/// Which game a script is being compiled for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Game {
    /// Star Wars: Knights of the Old Republic.
    K1,
    /// Star Wars: Knights of the Old Republic II - The Sith Lords.
    K2,
}

impl Game {
    /// Parses a game from a command line argument.
    ///
    /// Accepts `1`, `k1`, `kotor1`, `kotor`, and `2`, `k2`, `kotor2`, `tsl`,
    /// in any case.
    pub fn parse(text: &str) -> Option<Game> {
        match text.trim().to_ascii_lowercase().as_str() {
            "1" | "k1" | "kotor1" | "kotor" => Some(Game::K1),
            "2" | "k2" | "kotor2" | "tsl" => Some(Game::K2),
            _ => None,
        }
    }

    /// Returns the short name used in paths and arguments.
    pub fn short_name(self) -> &'static str {
        match self {
            Game::K1 => "k1",
            Game::K2 => "k2",
        }
    }

    /// Returns the game's full title.
    pub fn title(self) -> &'static str {
        match self {
            Game::K1 => "Knights of the Old Republic",
            Game::K2 => "Knights of the Old Republic II - The Sith Lords",
        }
    }
}

impl Default for Game {
    /// KOTOR I, matching what the original compiler assumed.
    fn default() -> Self {
        Game::K1
    }
}

impl fmt::Display for Game {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.short_name())
    }
}

/// One engine routine prototype.
#[derive(Debug, Clone, PartialEq)]
pub struct Routine {
    /// The index the byte stream calls it by. This is its position in the file.
    pub id: u16,
    /// The routine name as written in `nwscript.nss`.
    pub name: String,
    /// What it returns.
    pub return_type: Type,
    /// Its parameters, in order.
    pub parameters: Vec<RoutineParameter>,
    /// Where the prototype sits in `nwscript.nss`.
    pub span: Span,
}

impl Routine {
    /// Returns how many parameters have no default and must be passed.
    pub fn required_argument_count(&self) -> usize {
        self.parameters
            .iter()
            .filter(|parameter| !parameter.has_default)
            .count()
    }
}

/// One parameter of an engine routine.
#[derive(Debug, Clone, PartialEq)]
pub struct RoutineParameter {
    /// The parameter type.
    pub ty: Type,
    /// The parameter name.
    pub name: String,
    /// Whether the prototype gives it a default value.
    pub has_default: bool,
}

/// A `const` declared at file scope in `nwscript.nss`.
#[derive(Debug, Clone, PartialEq)]
pub struct Constant {
    /// The constant name, such as `OBJECT_SELF`.
    pub name: String,
    /// Its type.
    pub ty: Type,
}

/// A loaded, game-specific routine table.
#[derive(Debug, Clone)]
pub struct NwScript {
    game: Game,
    source: String,
    routines: Vec<Routine>,
    constants: Vec<Constant>,
}

impl NwScript {
    /// Reads `nwscript.nss` from disk and parses it.
    ///
    /// # Errors
    ///
    /// Returns [`CompileError::Io`] when the file cannot be read, or parse
    /// diagnostics when its contents are rejected.
    pub fn load(game: Game, path: impl AsRef<Path>) -> Result<NwScript, CompileError> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|error| CompileError::Io {
            path: path.display().to_string(),
            message: error.to_string(),
        })?;
        // Game files are Windows-1252 bytes, not UTF-8.
        NwScript::from_source(game, crate::preprocessor::decode_source(&bytes))
    }

    /// Parses `nwscript.nss` contents already held in memory.
    ///
    /// # Errors
    ///
    /// Returns parse diagnostics when the source is rejected.
    pub fn from_source(game: Game, source: impl Into<String>) -> Result<NwScript, CompileError> {
        let mut table = NwScript::empty(game);
        table.source = source.into();
        // TODO(nwscript): parse prototypes in file order, assigning each one
        // the next routine id, and collect file scope constants. Reuse the
        // lexer and parser rather than writing a second grammar. Fill
        // `table.routines` and `table.constants`, then return `Ok(table)`.
        Err(CompileError::NotImplemented("loading nwscript.nss"))
    }

    /// Builds a table with no routines and no constants.
    ///
    /// A script that calls no engine function compiles against this. It is also
    /// what tests use to exercise the pipeline without a copy of the game.
    pub fn empty(game: Game) -> NwScript {
        NwScript {
            game,
            source: String::new(),
            routines: Vec::new(),
            constants: Vec::new(),
        }
    }

    /// Returns which game this table is for.
    pub fn game(&self) -> Game {
        self.game
    }

    /// Returns the `nwscript.nss` source this table came from.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Returns every routine, ordered by id.
    pub fn routines(&self) -> &[Routine] {
        &self.routines
    }

    /// Returns every file scope constant.
    pub fn constants(&self) -> &[Constant] {
        &self.constants
    }

    /// Looks a routine up by name. Names are case sensitive.
    pub fn routine(&self, name: &str) -> Option<&Routine> {
        self.routines.iter().find(|routine| routine.name == name)
    }

    /// Looks a routine up by the id the byte stream uses.
    pub fn routine_by_id(&self, id: u16) -> Option<&Routine> {
        self.routines.iter().find(|routine| routine.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn games_parse_from_the_names_people_type() {
        assert_eq!(Game::parse("1"), Some(Game::K1));
        assert_eq!(Game::parse("K1"), Some(Game::K1));
        assert_eq!(Game::parse("kotor2"), Some(Game::K2));
        assert_eq!(Game::parse(" TSL "), Some(Game::K2));
        assert_eq!(Game::parse("k3"), None);
    }

    #[test]
    fn games_print_their_short_name() {
        assert_eq!(Game::K1.to_string(), "k1");
        assert_eq!(Game::K2.to_string(), "k2");
    }

    #[test]
    fn a_missing_nwscript_reports_the_path() {
        let error = NwScript::load(Game::K1, "does/not/exist/nwscript.nss")
            .expect_err("the file is missing");
        match error {
            CompileError::Io { path, .. } => assert!(path.contains("nwscript.nss")),
            other => panic!("expected an io error, got {other:?}"),
        }
    }

    #[test]
    fn loading_reports_that_parsing_is_unwritten_instead_of_panicking() {
        let error = NwScript::from_source(Game::K1, "void ActionAttack(object oTarget);")
            .expect_err("the loader is a stub");
        assert!(error.is_not_implemented());
    }

    #[test]
    fn required_arguments_skip_the_ones_with_defaults() {
        let routine = Routine {
            id: 0,
            name: "Random".to_string(),
            return_type: Type::Int,
            parameters: vec![
                RoutineParameter {
                    ty: Type::Int,
                    name: "nMaxInteger".to_string(),
                    has_default: false,
                },
                RoutineParameter {
                    ty: Type::Int,
                    name: "nSeed".to_string(),
                    has_default: true,
                },
            ],
            span: Span::default(),
        };
        assert_eq!(routine.required_argument_count(), 1);
    }
}
