//! The syntax tree the parser builds and the rest of the pipeline reads.
//!
//! # Contract
//!
//! This module is the interface between the front end (preprocessor, lexer,
//! parser) and the back end (sema, codegen). The parser produces exactly these
//! shapes and nothing else; sema and codegen consume them.
//!
//! Three rules the shapes encode, all of which come from the reference
//! compiler's observed behaviour:
//!
//! - **Engine structure types are data, not a fixed list.** `effect`, `event`,
//!   `location` and `talent` are declared by `#define ENGINE_STRUCTURE_<n>`
//!   lines in `nwscript.nss`, so [`EngineType`] carries the index the file gave
//!   them rather than naming them in an enum. A modified `nwscript.nss` can
//!   declare different names, and the compiler must follow it.
//! - **A prototype and a definition are the same node.** [`FunctionDecl`] with
//!   `body: None` is a prototype. While `nwscript.nss` itself is being
//!   compiled, a body-less function is an *engine routine*, and its position
//!   among the other body-less functions is its ACTION id. That rule lives in
//!   the caller, not here, because it depends on which file is being compiled.
//! - **A declarator list becomes several declarations.** `int a, b = 1;`
//!   parses into two [`VarDecl`]s in source order, so nothing downstream has to
//!   loop over declarators.

use crate::error::Span;

/// The name of the ordinary entry point.
pub const MAIN: &str = "main";

/// The name of the conditional-script entry point.
pub const STARTING_CONDITIONAL: &str = "StartingConditional";

/// A value type in NWScript.
///
/// `struct` types are named and resolved during semantic analysis, so they
/// carry their name rather than a resolved id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    /// No value.
    Void,
    /// 32-bit signed integer. Written `int`, or `INT`.
    Int,
    /// 32-bit float.
    Float,
    /// Immutable string.
    Str,
    /// A game object handle. Written `object`, or `OBJECT_ID`.
    Object,
    /// Three floats, laid out as `x`, `y`, `z`.
    Vector,
    /// An engine type declared by `#define ENGINE_STRUCTURE_<n>`.
    Engine(EngineType),
    /// A user-declared `struct`, by name.
    Struct(String),
    /// An `action`, the delayed-call type.
    ///
    /// Only `nwscript.nss` may name this type. Everywhere else it is the error
    /// `"action" identifier only valid in the context of "nwscript.nss"`.
    Action,
}

impl Type {
    /// Returns the type as it is written in source, for diagnostics.
    ///
    /// `int` and `object` come back in their lowercase spelling even when the
    /// source wrote `INT` or `OBJECT_ID`.
    pub fn display_name(&self) -> String {
        match self {
            Type::Void => "void".to_string(),
            Type::Int => "int".to_string(),
            Type::Float => "float".to_string(),
            Type::Str => "string".to_string(),
            Type::Object => "object".to_string(),
            Type::Vector => "vector".to_string(),
            Type::Engine(engine) => engine.name.clone(),
            Type::Struct(name) => format!("struct {name}"),
            Type::Action => "action".to_string(),
        }
    }

    /// Returns true for an engine structure type.
    pub fn is_engine(&self) -> bool {
        matches!(self, Type::Engine(_))
    }
}

/// An engine-defined opaque type, as declared in `nwscript.nss`.
///
/// `#define ENGINE_STRUCTURE_0 effect` produces `EngineType { index: 0, name:
/// "effect" }`. Stock KOTOR declares four: `effect`, `event`, `location`,
/// `talent`. NWN declares a fifth, `itemproperty`, which KOTOR does not have —
/// which is exactly why the list is not hard-coded here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineType {
    /// The `<n>` from `ENGINE_STRUCTURE_<n>`.
    pub index: u8,
    /// The type name the define gave it.
    pub name: String,
}

impl EngineType {
    /// Builds an engine type.
    pub fn new(index: u8, name: impl Into<String>) -> Self {
        EngineType {
            index,
            name: name.into(),
        }
    }

    /// Returns the NCS qualifier byte for an instruction taking one of these.
    ///
    /// `0x10 + index`, so `effect` is `0x10` and `talent` is `0x13`.
    ///
    /// Note the behavioural spec's §5.4 reports the reference compiler's
    /// single-type mapping as sending everything that is not int, float,
    /// string or object to `0x00`, while the bytecode spec's §1.5 reports
    /// `0x10 + n`. Retail bytecode contains `RSADDEFF` (`0x02 0x10`), so
    /// `0x10 + n` is what this returns. See `SPEC-GAP: engine-type-qualifier`.
    pub fn unary_type_code(&self) -> u8 {
        0x10u8.wrapping_add(self.index)
    }

    /// Returns the NCS qualifier byte for an instruction taking a pair.
    ///
    /// `0x30 + index`, so a pair of `effect`s is `0x30`.
    pub fn binary_type_code(&self) -> u8 {
        0x30u8.wrapping_add(self.index)
    }
}

/// A whole `.nss` translation unit after preprocessing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Script {
    /// Everything declared at file scope, in source order.
    pub declarations: Vec<Declaration>,
}

impl Script {
    /// Returns the function declaration for `name` that has a body.
    ///
    /// Prototypes are skipped, so this finds the definition even when a
    /// prototype precedes it.
    pub fn function(&self, name: &str) -> Option<&FunctionDecl> {
        self.declarations
            .iter()
            .filter_map(|declaration| match declaration {
                Declaration::Function(function) => Some(function),
                _ => None,
            })
            .find(|function| function.name == name && function.body.is_some())
    }

    /// Returns the struct declaration for `name`.
    pub fn struct_decl(&self, name: &str) -> Option<&StructDecl> {
        self.declarations
            .iter()
            .filter_map(|declaration| match declaration {
                Declaration::Struct(declaration) => Some(declaration),
                _ => None,
            })
            .find(|declaration| declaration.name == name)
    }

    /// Returns the script's entry point, preferring `main`.
    ///
    /// The reference compiler accepts either `void main()` or
    /// `int StartingConditional()`, and picks `main` when a script defines
    /// both. A script with neither is an include file, not a script.
    pub fn entry_point(&self) -> Option<(EntryPoint, &FunctionDecl)> {
        if let Some(function) = self.function(MAIN) {
            return Some((EntryPoint::Main, function));
        }
        self.function(STARTING_CONDITIONAL)
            .map(|function| (EntryPoint::StartingConditional, function))
    }

    /// Returns every body-less function declaration, in source order.
    ///
    /// When the file being compiled is `nwscript.nss`, this is the engine
    /// routine table and each entry's position is its ACTION id.
    pub fn prototypes(&self) -> impl Iterator<Item = &FunctionDecl> {
        self.declarations
            .iter()
            .filter_map(|declaration| match declaration {
                Declaration::Function(function) if function.body.is_none() => Some(function),
                _ => None,
            })
    }
}

/// Which entry point a script uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryPoint {
    /// `void main()`.
    Main,
    /// `int StartingConditional()`.
    StartingConditional,
}

impl EntryPoint {
    /// Returns the function name this entry point is spelled with.
    pub fn name(self) -> &'static str {
        match self {
            EntryPoint::Main => MAIN,
            EntryPoint::StartingConditional => STARTING_CONDITIONAL,
        }
    }
}

/// Anything that can appear at file scope.
#[derive(Debug, Clone, PartialEq)]
pub enum Declaration {
    /// A `struct` type declaration.
    Struct(StructDecl),
    /// A global variable, with or without an initializer.
    Global(VarDecl),
    /// A function prototype or definition.
    Function(FunctionDecl),
}

impl Declaration {
    /// Returns where the declaration sits in the source.
    pub fn span(&self) -> Span {
        match self {
            Declaration::Struct(declaration) => declaration.span,
            Declaration::Global(declaration) => declaration.span,
            Declaration::Function(declaration) => declaration.span,
        }
    }

    /// Returns the declared name.
    pub fn name(&self) -> &str {
        match self {
            Declaration::Struct(declaration) => &declaration.name,
            Declaration::Global(declaration) => &declaration.name,
            Declaration::Function(declaration) => &declaration.name,
        }
    }
}

/// A `struct Name { ... };` declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct StructDecl {
    /// The type name.
    pub name: String,
    /// The fields, in declaration order. Layout follows this order.
    pub fields: Vec<Field>,
    /// Where it was declared.
    pub span: Span,
}

/// One field of a struct.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    /// The field type.
    pub ty: Type,
    /// The field name.
    pub name: String,
    /// Whether the member was written `const`.
    ///
    /// The grammar accepts it; the reference compiler then rejects it with
    /// `"const" qualifier not allowed in structure definition`. The flag is
    /// here so sema can raise that rather than the parser losing the fact.
    pub is_const: bool,
    /// Where it was declared.
    pub span: Span,
}

/// A variable declaration, global or local.
///
/// One per declarator: `int a, b;` produces two of these.
#[derive(Debug, Clone, PartialEq)]
pub struct VarDecl {
    /// The declared type.
    pub ty: Type,
    /// The variable name.
    pub name: String,
    /// The initializer, if any.
    pub init: Option<Expr>,
    /// Whether the declaration was marked `const`.
    pub is_const: bool,
    /// Where it was declared.
    pub span: Span,
}

/// A function prototype or definition.
///
/// `body: None` is a prototype. In `nwscript.nss` a prototype is an engine
/// routine and its position among the prototypes is its ACTION id; elsewhere it
/// is a forward declaration that a later definition must match.
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionDecl {
    /// The return type.
    pub return_type: Type,
    /// The function name.
    pub name: String,
    /// The parameters, in order.
    pub parameters: Vec<Parameter>,
    /// The body, or `None` for a prototype.
    pub body: Option<Block>,
    /// Whether the return type was written `const`.
    ///
    /// Rejected by sema with `"const" qualifier not allowed on function return
    /// type`, not by the parser, because the grammar accepts it.
    pub is_const: bool,
    /// Where it was declared.
    pub span: Span,
}

impl FunctionDecl {
    /// Returns true when this declaration has no body.
    pub fn is_prototype(&self) -> bool {
        self.body.is_none()
    }

    /// Returns how many parameters have no default and must be passed.
    pub fn required_argument_count(&self) -> usize {
        self.parameters
            .iter()
            .filter(|parameter| parameter.default.is_none())
            .count()
    }
}

/// One parameter of a function.
#[derive(Debug, Clone, PartialEq)]
pub struct Parameter {
    /// The parameter type.
    pub ty: Type,
    /// The parameter name.
    pub name: String,
    /// The default value, if the parameter has one.
    ///
    /// NWScript allows default arguments, and `nwscript.nss` leans on them
    /// heavily. Defaults must be constants, and a parameter without a default
    /// may not follow one that has one. Both rules are checked in sema, not
    /// here, so the tree still describes what was written.
    pub default: Option<Expr>,
    /// Whether the parameter type was written `const`.
    ///
    /// Rejected by sema with `"const" qualifier not allowed in function
    /// prototype`.
    pub is_const: bool,
    /// Where it was declared.
    pub span: Span,
}

/// A braced list of statements.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Block {
    /// The statements, in order.
    pub statements: Vec<Stmt>,
    /// Where the block sits in the source.
    pub span: Span,
}

/// A statement.
#[derive(Debug, Clone, PartialEq)]
#[allow(missing_docs)]
pub enum Stmt {
    /// `;`
    Empty(Span),
    /// A nested block.
    Block(Block),
    /// A local variable declaration. One per declarator.
    VarDecl(VarDecl),
    /// An expression evaluated for its side effects.
    Expr {
        expr: Expr,
        span: Span,
    },
    If {
        condition: Expr,
        then_branch: Box<Stmt>,
        else_branch: Option<Box<Stmt>>,
        span: Span,
    },
    While {
        condition: Expr,
        body: Box<Stmt>,
        span: Span,
    },
    DoWhile {
        body: Box<Stmt>,
        condition: Expr,
        span: Span,
    },
    /// `for (init; condition; step) body`.
    ///
    /// All three slots are expressions, and all three are optional. The
    /// reference grammar has no production for a declaration in the initializer,
    /// so `for (int i = 0; ...)` is a syntax error, not a feature this parser
    /// declined to implement.
    For {
        init: Option<Expr>,
        condition: Option<Expr>,
        step: Option<Expr>,
        body: Box<Stmt>,
        span: Span,
    },
    /// `switch (subject) body`.
    ///
    /// The body is one statement, in practice always a block. Its `case` and
    /// `default` labels are [`Stmt::Case`] statements inside that block, not a
    /// separate list, because that is how the grammar spells them: a label is a
    /// statement that can appear anywhere a statement can.
    Switch {
        subject: Expr,
        body: Box<Stmt>,
        span: Span,
    },
    /// `case value:` or, with no value, `default:`.
    Case {
        label: Option<Expr>,
        span: Span,
    },
    Break(Span),
    Continue(Span),
    Return {
        value: Option<Expr>,
        span: Span,
    },
}

impl Stmt {
    /// Returns where the statement sits in the source.
    pub fn span(&self) -> Span {
        match self {
            Stmt::Empty(span)
            | Stmt::Break(span)
            | Stmt::Continue(span)
            | Stmt::Expr { span, .. }
            | Stmt::If { span, .. }
            | Stmt::While { span, .. }
            | Stmt::DoWhile { span, .. }
            | Stmt::For { span, .. }
            | Stmt::Switch { span, .. }
            | Stmt::Case { span, .. }
            | Stmt::Return { span, .. } => *span,
            Stmt::Block(block) => block.span,
            Stmt::VarDecl(declaration) => declaration.span,
        }
    }
}

/// An expression.
#[derive(Debug, Clone, PartialEq)]
#[allow(missing_docs)]
pub enum Expr {
    IntLit {
        value: i32,
        span: Span,
    },
    FloatLit {
        value: f32,
        span: Span,
    },
    /// A string literal, in the source file's own bytes.
    ///
    /// Not text. `CONSTS` writes a byte count and then those bytes, and the
    /// game reads them as Windows-1252, so `"café"` is four bytes ending in
    /// `0xe9`. Decoding to UTF-8 anywhere in between would make it five.
    StrLit {
        value: Vec<u8>,
        span: Span,
    },
    /// `OBJECT_SELF` (0) or `OBJECT_INVALID` (1).
    ///
    /// Both are reserved words in the lexer, not constants declared in
    /// `nwscript.nss`, so they resolve even against an empty routine table.
    /// KOTOR's `OBJECT_INVALID` is 1; NWN's is `0x7f000000`. Do not port that.
    ObjectLit {
        value: u32,
        span: Span,
    },
    /// `[x, y, z]`, a vector constant.
    ///
    /// The components must be float literals, not expressions, and any of the
    /// three may be left out: `[]` is the zero vector and `[1.0, 2.0]` has a
    /// `z` of zero. That is the whole syntax — there is no `[a, b, c]` over
    /// variables.
    VectorLit {
        x: f32,
        y: f32,
        z: f32,
        span: Span,
    },
    /// A name: a variable, a constant, or a function being called.
    Name {
        name: String,
        span: Span,
    },
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
        span: Span,
    },
    /// `x++` and `x--`, which evaluate before they update.
    Postfix {
        op: PostfixOp,
        operand: Box<Expr>,
        span: Span,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
        span: Span,
    },
    Assign {
        op: AssignOp,
        target: Box<Expr>,
        value: Box<Expr>,
        span: Span,
    },
    Conditional {
        condition: Box<Expr>,
        then_value: Box<Expr>,
        else_value: Box<Expr>,
        span: Span,
    },
    Call {
        callee: Box<Expr>,
        arguments: Vec<Expr>,
        span: Span,
    },
    /// `value.field`, for structs and for the `x`, `y`, `z` of a vector.
    Member {
        value: Box<Expr>,
        field: String,
        span: Span,
    },
}

impl Expr {
    /// Returns where the expression sits in the source.
    pub fn span(&self) -> Span {
        match self {
            Expr::IntLit { span, .. }
            | Expr::FloatLit { span, .. }
            | Expr::StrLit { span, .. }
            | Expr::ObjectLit { span, .. }
            | Expr::VectorLit { span, .. }
            | Expr::Name { span, .. }
            | Expr::Unary { span, .. }
            | Expr::Postfix { span, .. }
            | Expr::Binary { span, .. }
            | Expr::Assign { span, .. }
            | Expr::Conditional { span, .. }
            | Expr::Call { span, .. }
            | Expr::Member { span, .. } => *span,
        }
    }

    /// Returns true when the expression can appear on the left of `=`.
    ///
    /// The reference compiler rejects anything else with
    /// `Left hand side of assignment not a variable`.
    pub fn is_lvalue(&self) -> bool {
        match self {
            Expr::Name { .. } => true,
            Expr::Member { value, .. } => value.is_lvalue(),
            _ => false,
        }
    }
}

/// Folds a leading minus into the literal it negates.
///
/// This is the one and only fold the reference compiler performs, and unlike
/// every other one it is not gated on optimization being enabled. It applies in
/// exactly three places — a `const` declaration's value, a default argument,
/// and a `case` label — and only to a minus applied directly to a numeric
/// literal. `-1` becomes the literal `-1`; `-(1)` and `-x` are left alone, and
/// so is a `-1` anywhere else, which really does emit a push and a negate.
///
/// Skipping it costs two bytes per negative default argument, and
/// `ActionStartConversation` alone has two of them.
pub fn simplify_constant(expr: &Expr) -> Expr {
    match expr {
        Expr::Unary {
            op: UnaryOp::Negate,
            operand,
            span,
        } => match operand.as_ref() {
            Expr::IntLit { value, .. } => Expr::IntLit {
                value: value.wrapping_neg(),
                span: *span,
            },
            Expr::FloatLit { value, .. } => Expr::FloatLit {
                value: -*value,
                span: *span,
            },
            _ => expr.clone(),
        },
        _ => expr.clone(),
    }
}

/// A prefix operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum UnaryOp {
    /// `+`, which the grammar accepts and which does nothing.
    ///
    /// BioWare's own compiler rejects it. Torlack's accepts it for `int` and
    /// `float` and evaluates to the operand unchanged.
    Plus,
    Negate,
    Not,
    BitNot,
    PreIncrement,
    PreDecrement,
}

impl UnaryOp {
    /// Returns the operator as it is written, for diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            UnaryOp::Plus => "+",
            UnaryOp::Negate => "-",
            UnaryOp::Not => "!",
            UnaryOp::BitNot => "~",
            UnaryOp::PreIncrement => "++",
            UnaryOp::PreDecrement => "--",
        }
    }
}

/// A postfix operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum PostfixOp {
    Increment,
    Decrement,
}

impl PostfixOp {
    /// Returns the operator as it is written, for diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            PostfixOp::Increment => "++",
            PostfixOp::Decrement => "--",
        }
    }
}

/// An infix operator.
///
/// `BitAnd`, `BitOr` and `BitXor` are integer bitwise operators despite the
/// opcodes they lower to being named `BOOLAND`, `INCOR` and `EXCOR`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    NotEq,
    Less,
    LessEq,
    Greater,
    GreaterEq,
    LogicalAnd,
    LogicalOr,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    /// `>>>`, unsigned right shift. NWScript has no C equivalent for it.
    UShr,
}

impl BinaryOp {
    /// Returns the operator as it is written, for diagnostics.
    ///
    /// The reference compiler's `Operator (%s) not valid for specified types`
    /// substitutes these spellings.
    pub fn as_str(self) -> &'static str {
        match self {
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Mod => "%",
            BinaryOp::Eq => "==",
            BinaryOp::NotEq => "!=",
            BinaryOp::Less => "<",
            BinaryOp::LessEq => "<=",
            BinaryOp::Greater => ">",
            BinaryOp::GreaterEq => ">=",
            BinaryOp::LogicalAnd => "&&",
            BinaryOp::LogicalOr => "||",
            BinaryOp::BitAnd => "&",
            BinaryOp::BitOr => "|",
            BinaryOp::BitXor => "^",
            BinaryOp::Shl => "<<",
            BinaryOp::Shr => ">>",
            BinaryOp::UShr => ">>>",
        }
    }
}

/// An assignment operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum AssignOp {
    /// `=`
    Assign,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    UShr,
}

impl AssignOp {
    /// Returns the operator as it is written, for diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            AssignOp::Assign => "=",
            AssignOp::Add => "+=",
            AssignOp::Sub => "-=",
            AssignOp::Mul => "*=",
            AssignOp::Div => "/=",
            AssignOp::Mod => "%=",
            AssignOp::BitAnd => "&=",
            AssignOp::BitOr => "|=",
            AssignOp::BitXor => "^=",
            AssignOp::Shl => "<<=",
            AssignOp::Shr => ">>=",
            AssignOp::UShr => ">>>=",
        }
    }

    /// Returns the infix operator this compound assignment applies.
    ///
    /// `=` has none.
    pub fn binary_op(self) -> Option<BinaryOp> {
        match self {
            AssignOp::Assign => None,
            AssignOp::Add => Some(BinaryOp::Add),
            AssignOp::Sub => Some(BinaryOp::Sub),
            AssignOp::Mul => Some(BinaryOp::Mul),
            AssignOp::Div => Some(BinaryOp::Div),
            AssignOp::Mod => Some(BinaryOp::Mod),
            AssignOp::BitAnd => Some(BinaryOp::BitAnd),
            AssignOp::BitOr => Some(BinaryOp::BitOr),
            AssignOp::BitXor => Some(BinaryOp::BitXor),
            AssignOp::Shl => Some(BinaryOp::Shl),
            AssignOp::Shr => Some(BinaryOp::Shr),
            AssignOp::UShr => Some(BinaryOp::UShr),
        }
    }
}

/// What the parser's first pass found at file scope.
///
/// The reference compiler parses every file twice: once to collect
/// declarations, once to generate code. The first pass is what lets a script
/// call a function that is defined further down without writing a prototype,
/// and it is what decides, before any code is generated, whether the file is a
/// script or an include file.
///
/// The parser hands this to the caller so the back end does not have to walk
/// the tree again to answer those questions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeclarationIndex {
    /// Every `struct` name declared at file scope, in source order.
    pub struct_names: Vec<String>,
    /// Every function name declared at file scope, in source order.
    ///
    /// A name appears once even when a prototype and a definition both give it.
    pub function_names: Vec<String>,
    /// Every global variable name, in source order.
    pub global_names: Vec<String>,
    /// Whether a `main` with a body was found.
    pub has_main: bool,
    /// Whether a `StartingConditional` with a body was found.
    pub has_starting_conditional: bool,
}

impl DeclarationIndex {
    /// Returns true when the file defines neither entry point.
    ///
    /// The reference compiler stops there and prints
    /// `File is an include file, ignored`.
    pub fn is_include_only(&self) -> bool {
        !self.has_main && !self.has_starting_conditional
    }

    /// Returns which entry point the file uses, preferring `main`.
    pub fn entry_point(&self) -> Option<EntryPoint> {
        if self.has_main {
            Some(EntryPoint::Main)
        } else if self.has_starting_conditional {
            Some(EntryPoint::StartingConditional)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn function(name: &str, return_type: Type, has_body: bool) -> Declaration {
        Declaration::Function(FunctionDecl {
            return_type,
            name: name.to_string(),
            parameters: Vec::new(),
            body: has_body.then(Block::default),
            is_const: false,
            span: Span::default(),
        })
    }

    #[test]
    fn expressions_report_their_span() {
        let span = Span::default();
        let expr = Expr::IntLit { value: 7, span };
        assert_eq!(expr.span(), span);
    }

    #[test]
    fn a_script_starts_empty() {
        assert!(Script::default().declarations.is_empty());
    }

    #[test]
    fn engine_types_carry_the_index_the_define_gave_them() {
        let talent = EngineType::new(3, "talent");
        assert_eq!(talent.unary_type_code(), 0x13);
        assert_eq!(talent.binary_type_code(), 0x33);
        assert_eq!(Type::Engine(talent).display_name(), "talent");
    }

    #[test]
    fn main_wins_over_starting_conditional() {
        let script = Script {
            declarations: vec![
                function(STARTING_CONDITIONAL, Type::Int, true),
                function(MAIN, Type::Void, true),
            ],
        };
        let (entry, _) = script.entry_point().expect("the script has an entry point");
        assert_eq!(entry, EntryPoint::Main);
    }

    #[test]
    fn a_prototype_is_not_an_entry_point() {
        let script = Script {
            declarations: vec![function(MAIN, Type::Void, false)],
        };
        assert!(script.entry_point().is_none());
        assert_eq!(script.prototypes().count(), 1);
    }

    #[test]
    fn an_index_without_an_entry_point_is_an_include() {
        let index = DeclarationIndex::default();
        assert!(index.is_include_only());
        assert_eq!(index.entry_point(), None);
    }

    #[test]
    fn only_names_and_members_are_assignable() {
        let name = Expr::Name {
            name: "v".to_string(),
            span: Span::default(),
        };
        assert!(name.is_lvalue());

        let member = Expr::Member {
            value: Box::new(name),
            field: "x".to_string(),
            span: Span::default(),
        };
        assert!(member.is_lvalue());

        let literal = Expr::IntLit {
            value: 1,
            span: Span::default(),
        };
        assert!(!literal.is_lvalue());
    }

    #[test]
    fn compound_assignment_names_the_operator_it_applies() {
        assert_eq!(AssignOp::UShr.binary_op(), Some(BinaryOp::UShr));
        assert_eq!(AssignOp::Assign.binary_op(), None);
        assert_eq!(BinaryOp::UShr.as_str(), ">>>");
    }
}
