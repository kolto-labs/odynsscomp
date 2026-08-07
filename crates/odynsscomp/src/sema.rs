//! Name resolution and type checking.
//!
//! # Status
//!
//! Scaffold. What this stage owes the code generator:
//!
//! - Every [`crate::ast::Expr::Name`] resolved to a local slot, a global slot,
//!   a user function, an engine routine id, or a constant.
//! - Every expression given a type, with the implicit `int` to `float`
//!   widening applied where NWScript allows it and rejected where it does not.
//! - Struct field offsets computed, in declaration order.
//! - Stack slot assignment for locals, since the target is a stack machine and
//!   codegen addresses locals by offset.
//! - A `void main()` or `int StartingConditional()` entry point confirmed to
//!   exist. Which one is required depends on what the script is used for, so
//!   accept either.

use std::collections::HashMap;

use crate::ast::{Script, Type};
use crate::error::CompileError;
use crate::nwscript::NwScript;

/// What a name refers to once resolved.
#[derive(Debug, Clone, PartialEq)]
pub enum Symbol {
    /// A local variable at a stack offset.
    Local {
        /// The variable's type.
        ty: Type,
        /// Its offset in the current frame.
        slot: i32,
    },
    /// A global variable at a stack offset.
    Global {
        /// The variable's type.
        ty: Type,
        /// Its offset in the global frame.
        slot: i32,
    },
    /// A function defined in this script.
    Function {
        /// Its return type.
        return_type: Type,
        /// Its parameter types, in order.
        parameters: Vec<Type>,
    },
    /// An engine routine from `nwscript.nss`.
    Routine {
        /// The index the byte stream calls it by.
        id: u16,
        /// Its return type.
        return_type: Type,
    },
    /// A named constant from `nwscript.nss` or a `const` declaration.
    Constant {
        /// Its type.
        ty: Type,
    },
}

/// A lexically scoped name to [`Symbol`] map.
#[derive(Debug, Clone, Default)]
pub struct SymbolTable {
    scopes: Vec<HashMap<String, Symbol>>,
}

impl SymbolTable {
    /// Builds a table with one open scope.
    pub fn new() -> Self {
        SymbolTable {
            scopes: vec![HashMap::new()],
        }
    }

    /// Opens a nested scope.
    pub fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    /// Closes the innermost scope. The outermost scope is never closed.
    pub fn pop_scope(&mut self) {
        if self.scopes.len() > 1 {
            self.scopes.pop();
        }
    }

    /// Returns how many scopes are open.
    pub fn depth(&self) -> usize {
        self.scopes.len()
    }

    /// Declares a name in the innermost scope.
    ///
    /// Returns the symbol it replaced, which the caller should report as a
    /// redeclaration when it is `Some`.
    pub fn declare(&mut self, name: impl Into<String>, symbol: Symbol) -> Option<Symbol> {
        let scope = self
            .scopes
            .last_mut()
            .expect("a symbol table always keeps one scope open");
        scope.insert(name.into(), symbol)
    }

    /// Looks a name up from the innermost scope outward.
    pub fn lookup(&self, name: &str) -> Option<&Symbol> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name))
    }

    /// Returns true when the innermost scope already declares `name`.
    pub fn is_declared_locally(&self, name: &str) -> bool {
        self.scopes
            .last()
            .map(|scope| scope.contains_key(name))
            .unwrap_or(false)
    }
}

/// A checked script, ready for code generation.
#[derive(Debug, Clone)]
pub struct AnalyzedScript {
    /// The tree that was checked.
    pub script: Script,
    /// The resolved file scope names.
    pub globals: SymbolTable,
}

/// Resolves names and checks types.
///
/// # Errors
///
/// Returns every semantic error found, not just the first.
pub fn analyze(script: Script, nwscript: &NwScript) -> Result<AnalyzedScript, CompileError> {
    let _ = (script, nwscript);
    // TODO(sema): seed the global scope from the routine table, walk the tree,
    // resolve names, check types, and assign stack slots.
    Err(CompileError::NotImplemented("semantic analysis"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inner_scopes_shadow_outer_ones() {
        let mut table = SymbolTable::new();
        table.declare(
            "nCount",
            Symbol::Global {
                ty: Type::Int,
                slot: 0,
            },
        );
        table.push_scope();
        table.declare(
            "nCount",
            Symbol::Local {
                ty: Type::Float,
                slot: 4,
            },
        );

        match table.lookup("nCount") {
            Some(Symbol::Local { ty, .. }) => assert_eq!(*ty, Type::Float),
            other => panic!("expected the local to shadow, got {other:?}"),
        }

        table.pop_scope();
        assert!(matches!(
            table.lookup("nCount"),
            Some(Symbol::Global { .. })
        ));
    }

    #[test]
    fn the_outermost_scope_never_closes() {
        let mut table = SymbolTable::new();
        table.pop_scope();
        table.pop_scope();
        assert_eq!(table.depth(), 1);
    }

    #[test]
    fn redeclaring_in_one_scope_reports_the_old_symbol() {
        let mut table = SymbolTable::new();
        let symbol = Symbol::Constant { ty: Type::Int };
        assert!(table.declare("TRUE", symbol.clone()).is_none());
        assert_eq!(table.declare("TRUE", symbol.clone()), Some(symbol));
    }

    #[test]
    fn locally_declared_ignores_outer_scopes() {
        let mut table = SymbolTable::new();
        table.declare("x", Symbol::Constant { ty: Type::Int });
        table.push_scope();
        assert!(!table.is_declared_locally("x"));
        assert!(table.lookup("x").is_some());
    }
}
