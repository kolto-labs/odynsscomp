//! The syntax tree the parser builds and the rest of the pipeline reads.
//!
//! # Status
//!
//! Scaffold. The shapes here cover the whole NWScript grammar as the language
//! spec describes it, but nothing produces them yet. Extend freely; the parser
//! and code generator are the only consumers.

use crate::error::Span;

/// A value type in NWScript.
///
/// `struct` types are named and resolved during semantic analysis, so they
/// carry their name rather than a resolved id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    /// No value.
    Void,
    /// 32-bit signed integer.
    Int,
    /// 32-bit float.
    Float,
    /// Immutable string.
    Str,
    /// A game object handle.
    Object,
    /// Three floats.
    Vector,
    /// An engine type such as `effect`, `event`, `location`, `talent`, `itemproperty`.
    Engine(EngineType),
    /// A user-declared `struct`, by name.
    Struct(String),
    /// An `action`, the delayed-call type.
    Action,
}

/// The engine-defined opaque types. Both games share these five.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum EngineType {
    Effect,
    Event,
    Location,
    Talent,
    ItemProperty,
}

/// A whole `.nss` translation unit after preprocessing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Script {
    /// Everything declared at file scope, in source order.
    pub declarations: Vec<Declaration>,
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
    /// Where it was declared.
    pub span: Span,
}

/// A variable declaration, global or local.
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
    /// Where it was declared.
    pub span: Span,
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
    /// heavily. Defaults must be constants.
    pub default: Option<Expr>,
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
    /// A local variable declaration.
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
    For {
        init: Option<Expr>,
        condition: Option<Expr>,
        step: Option<Expr>,
        body: Box<Stmt>,
        span: Span,
    },
    Switch {
        subject: Expr,
        cases: Vec<SwitchCase>,
        span: Span,
    },
    Break(Span),
    Continue(Span),
    Return {
        value: Option<Expr>,
        span: Span,
    },
}

/// One `case` or `default` arm of a switch.
#[derive(Debug, Clone, PartialEq)]
pub struct SwitchCase {
    /// The label value, or `None` for `default`.
    pub label: Option<Expr>,
    /// The statements under this label. Fallthrough is the default, as in C.
    pub statements: Vec<Stmt>,
    /// Where the arm sits in the source.
    pub span: Span,
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
    StrLit {
        value: String,
        span: Span,
    },
    VectorLit {
        x: Box<Expr>,
        y: Box<Expr>,
        z: Box<Expr>,
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
}

/// A prefix operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum UnaryOp {
    Negate,
    Not,
    BitNot,
    PreIncrement,
    PreDecrement,
}

/// A postfix operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum PostfixOp {
    Increment,
    Decrement,
}

/// An infix operator.
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
    UShr,
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
