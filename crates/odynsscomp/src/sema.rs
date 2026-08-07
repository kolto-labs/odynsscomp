//! Name resolution, layout, and reachability.
//!
//! # What this stage decides
//!
//! - **Struct layout.** Fields sit in declaration order with no padding, and a
//!   slot is four bytes. Everything downstream addresses a member by its slot
//!   index, so the layout has to be settled before a single byte is emitted.
//! - **Global slots.** Globals get slots in declaration order, counting from
//!   the bottom of the global frame.
//! - **Which functions are emitted, and in what order.** The reference
//!   compiler walks from the entry point and emits only what it reaches. That
//!   walk is not an optimization — it runs whether or not `-o` was passed — so
//!   it is here, in [`AnalyzedScript::emit_order`].
//! - **Whether the script gets a `#globals` routine.** One global variable is
//!   enough, and so is one `struct` declaration, even when no variable uses it.
//!
//! # What it does not decide
//!
//! Expression types. The code generator has to know the type of every operand
//! anyway — the opcode qualifier byte *is* the operand type pair — so typing
//! happens there, once, and type errors are reported with [`Stage::Codegen`].
//! Splitting it would mean writing the same inference twice and keeping the two
//! in step.

use crate::ast::{
    simplify_constant, Block, Declaration, EntryPoint, Expr, FunctionDecl, Parameter, Script, Stmt,
    Type, VarDecl,
};
use crate::error::{CompileError, Diagnostic, Span, Stage};
use crate::nwscript::NwScript;

/// One `struct` type, laid out.
#[derive(Debug, Clone, PartialEq)]
pub struct StructLayout {
    /// The type name.
    pub name: String,
    /// The fields, in declaration order.
    pub fields: Vec<FieldLayout>,
    /// How many four-byte slots the whole struct takes.
    pub size: i32,
}

impl StructLayout {
    /// Returns the field named `name`.
    pub fn field(&self, name: &str) -> Option<&FieldLayout> {
        self.fields.iter().find(|field| field.name == name)
    }
}

/// One field of a struct, with the slot it starts at.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldLayout {
    /// The field name.
    pub name: String,
    /// The field type.
    pub ty: Type,
    /// Its slot index from the start of the struct.
    pub slot: i32,
}

/// A function this script defines.
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionInfo {
    /// The function name.
    pub name: String,
    /// What it returns.
    pub return_type: Type,
    /// Its parameters, taken from whichever declaration gave defaults.
    pub parameters: Vec<Parameter>,
    /// Index into [`AnalyzedScript::script`]'s declarations of the definition.
    pub declaration: usize,
}

/// A global variable, with the slot it occupies.
#[derive(Debug, Clone, PartialEq)]
pub struct GlobalInfo {
    /// The variable name.
    pub name: String,
    /// Its type.
    pub ty: Type,
    /// Its slot index from the bottom of the global frame.
    pub slot: i32,
    /// Its initializer, if it has one.
    pub init: Option<Expr>,
    /// Where it was declared.
    pub span: Span,
}

/// A name that stands for an expression rather than for storage.
///
/// Every file scope variable of `nwscript.nss` is one of these, and so is any
/// `const` declaration. A use of the name emits [`ConstantInfo::value`] inline;
/// there is no storage to read.
#[derive(Debug, Clone, PartialEq)]
pub struct ConstantInfo {
    /// The constant name.
    pub name: String,
    /// Its declared type.
    pub ty: Type,
    /// What a use of the name emits.
    pub value: Expr,
}

/// A checked script, ready for code generation.
#[derive(Debug, Clone)]
pub struct AnalyzedScript<'a> {
    /// The tree that was checked.
    pub script: Script,
    /// The routine table this script compiles against.
    pub nwscript: &'a NwScript,
    /// Every `struct` type, laid out, in declaration order.
    pub structs: Vec<StructLayout>,
    /// Every function with a body, in declaration order.
    pub functions: Vec<FunctionInfo>,
    /// Every global variable, in declaration order, with slots assigned.
    pub globals: Vec<GlobalInfo>,
    /// The script's own constants. `nwscript.nss`'s are reached separately.
    pub constants: Vec<ConstantInfo>,
    /// Which entry point the script uses.
    pub entry: EntryPoint,
    /// Indices into [`AnalyzedScript::functions`], in emission order.
    ///
    /// The entry point comes first, then each function in the order the walk
    /// from the entry point first reached it. A function nothing reaches is
    /// absent, and is never emitted.
    pub emit_order: Vec<usize>,
    /// Whether the script gets a `#globals` routine.
    pub needs_globals: bool,
}

impl AnalyzedScript<'_> {
    /// Returns how many four-byte slots a value of `ty` takes.
    ///
    /// `void` and `action` take none: an `action` argument is a jumped-over
    /// subroutine, not a pushed value, even though it still counts toward the
    /// argument count byte of an `ACTION` instruction.
    pub fn type_size(&self, ty: &Type) -> i32 {
        match ty {
            Type::Void | Type::Action => 0,
            Type::Int | Type::Float | Type::Str | Type::Object | Type::Engine(_) => 1,
            Type::Vector => 3,
            Type::Struct(name) => self.struct_layout(name).map(|s| s.size).unwrap_or(0),
        }
    }

    /// Returns the layout of the struct type named `name`.
    pub fn struct_layout(&self, name: &str) -> Option<&StructLayout> {
        self.structs.iter().find(|layout| layout.name == name)
    }

    /// Returns the function named `name`, if this script defines one.
    pub fn function(&self, name: &str) -> Option<(usize, &FunctionInfo)> {
        self.functions
            .iter()
            .enumerate()
            .find(|(_, function)| function.name == name)
    }

    /// Returns the global variable named `name`.
    pub fn global(&self, name: &str) -> Option<&GlobalInfo> {
        self.globals.iter().find(|global| global.name == name)
    }

    /// Returns the constant named `name`, preferring the script's own.
    pub fn constant(&self, name: &str) -> Option<(&Type, &Expr)> {
        if let Some(constant) = self.constants.iter().find(|c| c.name == name) {
            return Some((&constant.ty, &constant.value));
        }
        self.nwscript
            .constant(name)
            .map(|constant| (&constant.ty, &constant.value))
    }

    /// Returns the body of the function at `index`, if it has one.
    pub fn body(&self, index: usize) -> Option<&Block> {
        let declaration = self.functions.get(index)?.declaration;
        match self.script.declarations.get(declaration)? {
            Declaration::Function(function) => function.body.as_ref(),
            _ => None,
        }
    }
}

/// Resolves names, lays out types, and works out what gets emitted.
///
/// # Errors
///
/// Returns every semantic error found, not just the first.
pub fn analyze<'a>(
    script: Script,
    nwscript: &'a NwScript,
) -> Result<AnalyzedScript<'a>, CompileError> {
    let mut diagnostics = Vec::new();

    let structs = lay_out_structs(&script, &mut diagnostics);
    let functions = collect_functions(&script, &mut diagnostics);
    let (globals, constants) = collect_globals(&script, &structs, &mut diagnostics);

    let entry = match script.entry_point() {
        Some((entry, _)) => entry,
        None => {
            diagnostics.push(Diagnostic::error(
                Stage::Sema,
                Span::default(),
                "File is an include file, ignored",
            ));
            EntryPoint::Main
        }
    };

    for declaration in &script.declarations {
        if let Declaration::Function(function) = declaration {
            check_function_shape(function, &mut diagnostics);
        }
    }

    let mut analyzed = AnalyzedScript {
        script,
        nwscript,
        structs,
        functions,
        globals,
        constants,
        entry,
        emit_order: Vec::new(),
        needs_globals: false,
    };
    analyzed.needs_globals = !analyzed.globals.is_empty() || !analyzed.structs.is_empty();
    analyzed.emit_order = gather_used(&analyzed);

    if diagnostics.iter().any(Diagnostic::is_error) {
        return Err(CompileError::Diagnostics(diagnostics));
    }
    Ok(analyzed)
}

fn lay_out_structs(script: &Script, diagnostics: &mut Vec<Diagnostic>) -> Vec<StructLayout> {
    let mut layouts: Vec<StructLayout> = Vec::new();

    for declaration in &script.declarations {
        let Declaration::Struct(declaration) = declaration else {
            continue;
        };
        if layouts.iter().any(|layout| layout.name == declaration.name) {
            diagnostics.push(Diagnostic::error(
                Stage::Sema,
                declaration.span,
                format!("Structure \"{}\" already defined", declaration.name),
            ));
            continue;
        }

        let mut fields = Vec::new();
        let mut slot = 0;
        for field in &declaration.fields {
            if field.is_const {
                diagnostics.push(Diagnostic::error(
                    Stage::Sema,
                    field.span,
                    "\"const\" qualifier not allowed in structure definition",
                ));
            }
            // A struct may hold another struct, and only an already-declared
            // one: the reference compiler has no forward declarations, so the
            // sizes it needs are always to hand.
            let size = match &field.ty {
                Type::Void => {
                    diagnostics.push(Diagnostic::error(
                        Stage::Sema,
                        field.span,
                        format!("Structure member \"{}\" cannot be void", field.name),
                    ));
                    0
                }
                Type::Vector => 3,
                Type::Struct(name) => match layouts.iter().find(|layout| layout.name == *name) {
                    Some(layout) => layout.size,
                    None => {
                        diagnostics.push(Diagnostic::error(
                            Stage::Sema,
                            field.span,
                            format!("Undeclared identifier \"{name}\""),
                        ));
                        0
                    }
                },
                _ => 1,
            };
            fields.push(FieldLayout {
                name: field.name.clone(),
                ty: field.ty.clone(),
                slot,
            });
            slot += size;
        }

        layouts.push(StructLayout {
            name: declaration.name.clone(),
            fields,
            size: slot,
        });
    }

    layouts
}

fn collect_functions(script: &Script, diagnostics: &mut Vec<Diagnostic>) -> Vec<FunctionInfo> {
    let mut functions: Vec<FunctionInfo> = Vec::new();

    for (index, declaration) in script.declarations.iter().enumerate() {
        let Declaration::Function(function) = declaration else {
            continue;
        };
        if function.body.is_none() {
            continue;
        }
        if functions.iter().any(|other| other.name == function.name) {
            diagnostics.push(Diagnostic::error(
                Stage::Sema,
                function.span,
                format!("Function \"{}\" already defined", function.name),
            ));
            continue;
        }
        functions.push(FunctionInfo {
            name: function.name.clone(),
            return_type: function.return_type.clone(),
            parameters: merge_defaults(script, function),
            declaration: index,
        });
    }

    functions
}

/// Takes the parameter list from the definition, and defaults from wherever.
///
/// A prototype may carry the default values and the definition repeat the
/// parameters without them. Calls need the values, so pull them across.
fn merge_defaults(script: &Script, definition: &FunctionDecl) -> Vec<Parameter> {
    let mut parameters = definition.parameters.clone();
    for parameter in &mut parameters {
        parameter.default = parameter.default.as_ref().map(simplify_constant);
    }
    let prototype = script
        .declarations
        .iter()
        .find_map(|declaration| match declaration {
            Declaration::Function(function)
                if function.name == definition.name && function.body.is_none() =>
            {
                Some(function)
            }
            _ => None,
        });
    if let Some(prototype) = prototype {
        for (parameter, declared) in parameters.iter_mut().zip(&prototype.parameters) {
            if parameter.default.is_none() {
                parameter.default = declared.default.as_ref().map(simplify_constant);
            }
        }
    }
    parameters
}

fn collect_globals(
    script: &Script,
    structs: &[StructLayout],
    diagnostics: &mut Vec<Diagnostic>,
) -> (Vec<GlobalInfo>, Vec<ConstantInfo>) {
    let mut globals = Vec::new();
    let mut constants = Vec::new();
    let mut slot = 0;

    for declaration in &script.declarations {
        let Declaration::Global(variable) = declaration else {
            continue;
        };

        if variable.is_const {
            match &variable.init {
                Some(value) => constants.push(ConstantInfo {
                    name: variable.name.clone(),
                    ty: variable.ty.clone(),
                    value: simplify_constant(value),
                }),
                None => diagnostics.push(Diagnostic::error(
                    Stage::Sema,
                    variable.span,
                    format!("Required value for constant \"{}\" missing.", variable.name),
                )),
            }
            continue;
        }

        let size = size_of(&variable.ty, structs);
        globals.push(GlobalInfo {
            name: variable.name.clone(),
            ty: variable.ty.clone(),
            slot,
            init: variable.init.clone(),
            span: variable.span,
        });
        slot += size;
    }

    (globals, constants)
}

fn size_of(ty: &Type, structs: &[StructLayout]) -> i32 {
    match ty {
        Type::Void | Type::Action => 0,
        Type::Vector => 3,
        Type::Struct(name) => structs
            .iter()
            .find(|layout| layout.name == *name)
            .map(|layout| layout.size)
            .unwrap_or(0),
        _ => 1,
    }
}

fn check_function_shape(function: &FunctionDecl, diagnostics: &mut Vec<Diagnostic>) {
    if function.is_const {
        diagnostics.push(Diagnostic::error(
            Stage::Sema,
            function.span,
            "\"const\" qualifier not allowed on function return type",
        ));
    }
    let mut seen_default = false;
    for parameter in &function.parameters {
        if parameter.is_const {
            diagnostics.push(Diagnostic::error(
                Stage::Sema,
                parameter.span,
                "\"const\" qualifier not allowed in function prototype",
            ));
        }
        if parameter.default.is_some() {
            seen_default = true;
        } else if seen_default {
            diagnostics.push(Diagnostic::error(
                Stage::Sema,
                parameter.span,
                format!("Parameter \"{}\" must have a default value", parameter.name),
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// reachability
// ---------------------------------------------------------------------------

/// Returns the functions to emit, in the order to emit them.
///
/// Global initializers are walked first, so a function only ever called from
/// one lands ahead of the entry point. Then the entry point, then depth first
/// from there, arguments before the call itself.
fn gather_used(analyzed: &AnalyzedScript<'_>) -> Vec<usize> {
    let mut order = Vec::new();
    let mut seen = vec![false; analyzed.functions.len()];

    for global in &analyzed.globals {
        if let Some(init) = &global.init {
            gather_expr(analyzed, init, &mut order, &mut seen);
        }
    }

    let entry = analyzed.entry.name();
    if let Some((index, _)) = analyzed.function(entry) {
        if !seen[index] {
            seen[index] = true;
            order.push(index);
            if let Some(body) = analyzed.body(index) {
                gather_block(analyzed, body, &mut order, &mut seen);
            }
        }
    }

    order
}

fn gather_block(
    analyzed: &AnalyzedScript<'_>,
    block: &Block,
    order: &mut Vec<usize>,
    seen: &mut [bool],
) {
    for statement in &block.statements {
        gather_stmt(analyzed, statement, order, seen);
    }
}

fn gather_stmt(
    analyzed: &AnalyzedScript<'_>,
    statement: &Stmt,
    order: &mut Vec<usize>,
    seen: &mut [bool],
) {
    let mut walk = |expr: &Expr| gather_expr(analyzed, expr, order, seen);
    match statement {
        Stmt::Empty(_) | Stmt::Break(_) | Stmt::Continue(_) => {}
        Stmt::Block(block) => gather_block(analyzed, block, order, seen),
        Stmt::VarDecl(VarDecl { init, .. }) => {
            if let Some(init) = init {
                walk(init);
            }
        }
        Stmt::Expr { expr, .. } => walk(expr),
        Stmt::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            walk(condition);
            gather_stmt(analyzed, then_branch, order, seen);
            if let Some(branch) = else_branch {
                gather_stmt(analyzed, branch, order, seen);
            }
        }
        Stmt::While {
            condition, body, ..
        } => {
            walk(condition);
            gather_stmt(analyzed, body, order, seen);
        }
        Stmt::DoWhile {
            body, condition, ..
        } => {
            gather_stmt(analyzed, body, order, seen);
            gather_expr(analyzed, condition, order, seen);
        }
        Stmt::For {
            init,
            condition,
            step,
            body,
            ..
        } => {
            for expr in [init, condition, step].into_iter().flatten() {
                gather_expr(analyzed, expr, order, seen);
            }
            gather_stmt(analyzed, body, order, seen);
        }
        Stmt::Switch { subject, body, .. } => {
            walk(subject);
            gather_stmt(analyzed, body, order, seen);
        }
        Stmt::Case { label, .. } => {
            if let Some(label) = label {
                walk(label);
            }
        }
        Stmt::Return { value, .. } => {
            if let Some(value) = value {
                walk(value);
            }
        }
    }
}

fn gather_expr(
    analyzed: &AnalyzedScript<'_>,
    expr: &Expr,
    order: &mut Vec<usize>,
    seen: &mut [bool],
) {
    match expr {
        Expr::IntLit { .. }
        | Expr::FloatLit { .. }
        | Expr::StrLit { .. }
        | Expr::ObjectLit { .. }
        | Expr::VectorLit { .. } => {}
        Expr::Name { name, .. } => {
            // A constant's initializer is emitted at every use, so anything it
            // calls is reachable from every use too.
            if let Some((_, value)) = analyzed.constant(name) {
                let value = value.clone();
                gather_expr(analyzed, &value, order, seen);
            }
        }
        Expr::Unary { operand, .. } | Expr::Postfix { operand, .. } => {
            gather_expr(analyzed, operand, order, seen)
        }
        Expr::Binary { left, right, .. } => {
            gather_expr(analyzed, left, order, seen);
            gather_expr(analyzed, right, order, seen);
        }
        Expr::Assign { target, value, .. } => {
            gather_expr(analyzed, target, order, seen);
            gather_expr(analyzed, value, order, seen);
        }
        Expr::Conditional {
            condition,
            then_value,
            else_value,
            ..
        } => {
            gather_expr(analyzed, condition, order, seen);
            gather_expr(analyzed, then_value, order, seen);
            gather_expr(analyzed, else_value, order, seen);
        }
        Expr::Member { value, .. } => gather_expr(analyzed, value, order, seen),
        Expr::Call {
            callee, arguments, ..
        } => {
            for argument in arguments {
                gather_expr(analyzed, argument, order, seen);
            }
            let Expr::Name { name, .. } = callee.as_ref() else {
                return;
            };
            let Some((index, _)) = analyzed.function(name) else {
                return;
            };
            if seen[index] {
                return;
            }
            seen[index] = true;
            order.push(index);
            if let Some(body) = analyzed.body(index) {
                gather_block(analyzed, body, order, seen);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nwscript::Game;

    fn analyze_source(source: &str) -> AnalyzedScript<'static> {
        // Leaked on purpose: a test fixture that lives as long as the process.
        let nwscript: &'static NwScript = Box::leak(Box::new(NwScript::empty(Game::K1)));
        let expansion = crate::preprocessor::expand(
            source,
            std::path::Path::new("test.nss"),
            &crate::preprocessor::IncludeResolver::new(),
        )
        .expect("the fixture preprocesses");
        let lexed = crate::lexer::scan(&expansion.source, &expansion.engine_structures);
        let script = crate::parser::parse(lexed.tokens).expect("the fixture parses");
        analyze(script, nwscript).expect("the fixture analyzes")
    }

    #[test]
    fn a_struct_lays_its_fields_out_in_order_with_no_padding() {
        let analyzed = analyze_source("struct Pair { int a; vector v; int b; }; void main() { }");
        let layout = analyzed.struct_layout("Pair").expect("Pair is laid out");
        assert_eq!(layout.size, 5);
        assert_eq!(layout.field("a").unwrap().slot, 0);
        assert_eq!(layout.field("v").unwrap().slot, 1);
        assert_eq!(layout.field("b").unwrap().slot, 4);
    }

    #[test]
    fn a_bare_struct_declaration_is_enough_to_need_a_globals_block() {
        let analyzed = analyze_source("struct Pair { int a; }; void main() { }");
        assert!(analyzed.globals.is_empty());
        assert!(analyzed.needs_globals);
    }

    #[test]
    fn a_script_with_neither_globals_nor_structs_needs_no_globals_block() {
        assert!(!analyze_source("void main() { }").needs_globals);
    }

    #[test]
    fn an_uncalled_function_is_left_out_of_the_emission_order() {
        let analyzed = analyze_source("void Used() { } void Unused() { } void main() { Used(); }");
        let names: Vec<_> = analyzed
            .emit_order
            .iter()
            .map(|index| analyzed.functions[*index].name.as_str())
            .collect();
        assert_eq!(names, vec!["main", "Used"]);
    }

    #[test]
    fn arguments_are_reached_before_the_call_that_takes_them() {
        let analyzed = analyze_source(
            "int Inner() { return 1; } void Outer(int n) { } void main() { Outer(Inner()); }",
        );
        let names: Vec<_> = analyzed
            .emit_order
            .iter()
            .map(|index| analyzed.functions[*index].name.as_str())
            .collect();
        assert_eq!(names, vec!["main", "Inner", "Outer"]);
    }

    #[test]
    fn globals_take_slots_in_declaration_order() {
        let analyzed = analyze_source("int gA; vector gV; int gB; void main() { }");
        assert_eq!(analyzed.global("gA").unwrap().slot, 0);
        assert_eq!(analyzed.global("gV").unwrap().slot, 1);
        assert_eq!(analyzed.global("gB").unwrap().slot, 4);
    }

    #[test]
    fn a_const_global_is_a_constant_and_takes_no_slot() {
        let analyzed = analyze_source("const int LIMIT = 5; void main() { }");
        assert!(analyzed.globals.is_empty());
        assert!(analyzed.constant("LIMIT").is_some());
    }
}
