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
//! # What loading a table does
//!
//! `nwscript.nss` is compiled with two permissions no other file has: it may
//! name the `action` type, and a body-less function in it is an engine routine
//! rather than a forward declaration. Its file scope variables are not
//! variables either — every one of them is a constant, and a script that names
//! `TRUE` gets that constant's initializer emitted inline instead of a read of
//! a global. That is why [`Constant`] keeps the initializer expression.

use std::fmt;
use std::path::Path;

use crate::ast::{simplify_constant, Declaration, Expr, Type};
use crate::error::{CompileError, Diagnostic, Span, Stage};
use crate::preprocessor::EngineStructures;

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
            .filter(|parameter| parameter.default.is_none())
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
    /// The default value, if the prototype gives one.
    ///
    /// A call that leaves the argument out gets this expression emitted at the
    /// call site, exactly as if it had been written there. So the expression
    /// has to survive, not just the fact that there was one.
    pub default: Option<Expr>,
}

impl RoutineParameter {
    /// Returns whether the prototype gives the parameter a default value.
    pub fn has_default(&self) -> bool {
        self.default.is_some()
    }
}

/// A file scope variable of `nwscript.nss`, which is a constant.
#[derive(Debug, Clone, PartialEq)]
pub struct Constant {
    /// The constant name, such as `TRUE`.
    pub name: String,
    /// Its type.
    pub ty: Type,
    /// The initializer, which is what a use of the name emits.
    pub value: Expr,
}

/// A loaded, game-specific routine table.
#[derive(Debug, Clone)]
pub struct NwScript {
    game: Game,
    source: String,
    routines: Vec<Routine>,
    constants: Vec<Constant>,
    engine_structures: EngineStructures,
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
        let source = source.into();
        let origin = Path::new("nwscript.nss");
        let resolver = crate::preprocessor::IncludeResolver::new();
        let expansion = crate::preprocessor::expand(&source, origin, &resolver)?;

        let lexed = crate::lexer::scan(&expansion.source, &expansion.engine_structures);
        if lexed.has_errors() {
            return Err(CompileError::Diagnostics(
                expansion.localize_all(lexed.diagnostics),
            ));
        }
        let script = crate::parser::parse(lexed.tokens)
            .map_err(|error| relabel(error, &expansion, origin))?;

        let mut routines = Vec::new();
        let mut constants = Vec::new();
        for declaration in &script.declarations {
            match declaration {
                // A body-less function in this file is an engine routine, and
                // its position among them is the id the byte stream calls it
                // by. Nothing else assigns those numbers.
                Declaration::Function(function) if function.body.is_none() => {
                    let id = u16::try_from(routines.len()).map_err(|_| {
                        CompileError::from_diagnostic(Diagnostic::error(
                            Stage::NwScript,
                            function.span,
                            "nwscript.nss declares more than 65536 engine routines",
                        ))
                    })?;
                    routines.push(Routine {
                        id,
                        name: function.name.clone(),
                        return_type: function.return_type.clone(),
                        parameters: function
                            .parameters
                            .iter()
                            .map(|parameter| RoutineParameter {
                                ty: parameter.ty.clone(),
                                name: parameter.name.clone(),
                                default: parameter.default.as_ref().map(simplify_constant),
                            })
                            .collect(),
                        span: function.span,
                    });
                }
                Declaration::Global(variable) => {
                    let Some(value) = variable.init.clone() else {
                        return Err(CompileError::from_diagnostic(Diagnostic::error(
                            Stage::NwScript,
                            variable.span,
                            format!("Required value for constant \"{}\" missing.", variable.name),
                        )));
                    };
                    constants.push(Constant {
                        name: variable.name.clone(),
                        ty: variable.ty.clone(),
                        value: simplify_constant(&value),
                    });
                }
                _ => {}
            }
        }

        Ok(NwScript {
            game,
            source,
            routines,
            constants,
            engine_structures: expansion.engine_structures,
        })
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
            engine_structures: EngineStructures::new(),
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

    /// Looks a constant up by name. Names are case sensitive.
    pub fn constant(&self, name: &str) -> Option<&Constant> {
        self.constants.iter().find(|constant| constant.name == name)
    }

    /// Returns the engine structure types this table's `#define` lines declared.
    ///
    /// A script never declares its own; it inherits these, which is why the
    /// name `effect` resolves in a file that never mentions `nwscript.nss`.
    pub fn engine_structures(&self) -> &EngineStructures {
        &self.engine_structures
    }
}

/// Rewrites a compile error to blame `nwscript.nss` rather than the buffer.
fn relabel(
    error: CompileError,
    expansion: &crate::preprocessor::Expansion,
    origin: &Path,
) -> CompileError {
    match error {
        CompileError::Diagnostics(list) => {
            let mut list = expansion.localize_all(list);
            let label = origin.display().to_string();
            for diagnostic in &mut list {
                diagnostic.stage = Stage::NwScript;
                if diagnostic.file.is_empty() {
                    diagnostic.file = label.clone();
                }
            }
            CompileError::Diagnostics(list)
        }
        other => other,
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
    fn a_prototype_becomes_a_routine_numbered_by_its_position() {
        let table = NwScript::from_source(
            Game::K1,
            "void Random(int n);\nvoid PrintString(string s);\n",
        )
        .expect("the table loads");
        assert_eq!(table.routine("Random").expect("Random is there").id, 0);
        assert_eq!(
            table
                .routine("PrintString")
                .expect("PrintString is there")
                .id,
            1
        );
    }

    #[test]
    fn a_file_scope_variable_becomes_a_constant_that_keeps_its_value() {
        let table =
            NwScript::from_source(Game::K1, "int TRUE = 1;\nfloat PI = 3.14;\n").expect("loads");
        let constant = table.constant("TRUE").expect("TRUE is there");
        assert_eq!(constant.ty, Type::Int);
        assert!(matches!(constant.value, Expr::IntLit { value: 1, .. }));
        assert!(table.routines().is_empty());
    }

    #[test]
    fn engine_structure_defines_come_along_with_the_table() {
        let table = NwScript::from_source(Game::K1, "#define ENGINE_STRUCTURE_0 effect\n")
            .expect("the table loads");
        assert_eq!(table.engine_structures().lookup("effect"), Some(0));
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
                    default: None,
                },
                RoutineParameter {
                    ty: Type::Int,
                    name: "nSeed".to_string(),
                    default: Some(Expr::IntLit {
                        value: 0,
                        span: Span::default(),
                    }),
                },
            ],
            span: Span::default(),
        };
        assert_eq!(routine.required_argument_count(), 1);
    }
}
