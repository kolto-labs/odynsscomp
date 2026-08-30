//! Lowers a checked script to compiled script bytes.
//!
//! # The rule this module exists to obey
//!
//! **Emit what the tree says, and nothing cleverer.** The reference compiler
//! gates every optimization it has on one constructor flag, and the KOTOR fork
//! turned its `-o` switch into a no-op, so that flag is permanently off. `2 + 3`
//! stays two `CONSTI` and an `ADD`. `if (0) { ... }` keeps its dead body. Code
//! after a `return` is still emitted. Nothing is folded, shortened, or dropped.
//!
//! Three consequences look like bugs and are not:
//!
//! - **`JMP +6`.** An `if` always takes the shape that has an `else`, so an
//!   `if` without one still emits a jump to the next instruction. Nested `if`s
//!   that end together produce a chain of them. Retail scripts are full of it.
//! - **`||` does not short-circuit.** The version-100 code generator emits a
//!   `JZ` where it meant a `JMP`, so the right operand always runs.
//! - **`break` and `continue` leak the stack.** They emit a bare jump with no
//!   cleanup, so locals declared inside the block they leave are never popped.
//!
//! The one thing that *is* left out is a function nothing calls, and that is
//! reachability rather than optimization. [`crate::sema`] works it out.
//!
//! # Order of operations
//!
//! `#loader` first, then `#globals` if the script needs one, then the entry
//! point, then every other reachable function.

use crate::ast::{
    simplify_constant, AssignOp, BinaryOp, Block, EngineType, EntryPoint, Expr, Parameter,
    PostfixOp, Stmt, Type, UnaryOp, VarDecl,
};
use crate::error::{CompileError, Diagnostic, Span, Stage};
use crate::ncs::{op, qualifier, NcsWriter};
use crate::nwscript::Routine;
use crate::sema::AnalyzedScript;

/// How hard to work at shrinking the output.
///
/// Both settings emit the same bytes. The reference compiler's `-o` is a no-op
/// in the KOTOR fork, and byte equality with it is the point, so there is
/// nothing for a higher setting to do. The variant is kept because callers
/// pass it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OptimizationLevel {
    /// Emit what the tree says, instruction for instruction.
    #[default]
    None,
    /// Accepted, and identical to [`OptimizationLevel::None`].
    Basic,
}

/// Turns a checked script into compiled script bytes, header included.
///
/// # Errors
///
/// Returns the type and resolution errors found while lowering.
pub fn generate(
    script: &AnalyzedScript<'_>,
    optimization: OptimizationLevel,
) -> Result<Vec<u8>, CompileError> {
    // Deliberately unused: see OptimizationLevel.
    let _ = optimization;
    Emitter::new(script).run()
}

// ---------------------------------------------------------------------------
// labels
// ---------------------------------------------------------------------------

type LabelId = usize;

/// A jump target, and every instruction still waiting to learn where it is.
#[derive(Debug, Default)]
struct Label {
    target: Option<usize>,
    /// Offsets of the jump instructions that point here, each at its opcode
    /// byte. A displacement is measured from there, not from the byte after.
    references: Vec<usize>,
}

// ---------------------------------------------------------------------------
// scopes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Binding {
    /// A variable at a slot index counted from the bottom of the frame.
    Slot(i32),
    /// A `const`, which has no storage. Its initializer is emitted at each use.
    Constant(Expr),
}

#[derive(Debug, Clone)]
struct Local {
    name: String,
    ty: Type,
    binding: Binding,
}

/// Which depth counter an instruction moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Depth {
    /// Nothing. The reference passes a null counter in a few places, and the
    /// mismatch it leaves behind is load bearing.
    Untracked,
    /// Declared frame content: arguments and live locals.
    Frame,
    /// Temporaries pushed by the expression being evaluated.
    Expression,
}

/// Where a variable lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Storage {
    Local,
    Global,
}

/// A resolved reference to storage, possibly to one member of it.
#[derive(Debug, Clone)]
struct Place {
    storage: Storage,
    slot: i32,
    /// The type of the whole variable, which is what a member read copies.
    source_ty: Type,
    /// The type this reference yields.
    ty: Type,
    /// The member's slot offset within the variable, or `None` for the whole.
    element: Option<i32>,
}

/// What a variable reference should do to the variable on the way past.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Increment,
    Decrement,
}

// ---------------------------------------------------------------------------
// the emitter
// ---------------------------------------------------------------------------

struct Emitter<'a> {
    program: &'a AnalyzedScript<'a>,
    out: NcsWriter,
    labels: Vec<Label>,
    /// One label per entry of `program.functions`, plus one for `#globals`.
    function_labels: Vec<LabelId>,
    globals_label: LabelId,

    frame_depth: i32,
    expression_depth: i32,
    global_depth: i32,
    return_size: i32,
    argument_size: i32,
    in_globals: bool,

    return_label: LabelId,
    break_label: Option<LabelId>,
    continue_label: Option<LabelId>,

    scopes: Vec<Vec<Local>>,
    /// Case labels planted by the scan pass, keyed by the statement's address.
    case_labels: Vec<(usize, LabelId)>,
    default_label: Option<LabelId>,
}

impl<'a> Emitter<'a> {
    fn new(program: &'a AnalyzedScript<'a>) -> Self {
        let mut emitter = Emitter {
            program,
            out: NcsWriter::new(),
            labels: Vec::new(),
            function_labels: Vec::new(),
            globals_label: 0,
            frame_depth: 0,
            expression_depth: 0,
            global_depth: 0,
            return_size: 0,
            argument_size: 0,
            in_globals: false,
            return_label: 0,
            break_label: None,
            continue_label: None,
            scopes: Vec::new(),
            case_labels: Vec::new(),
            default_label: None,
        };
        emitter.globals_label = emitter.new_label();
        emitter.function_labels = (0..program.functions.len())
            .map(|_| emitter.new_label())
            .collect();
        emitter
    }

    fn run(mut self) -> Result<Vec<u8>, CompileError> {
        let program = self.program;
        let entry_name = program.entry.name();
        let Some((entry_index, _)) = program.function(entry_name) else {
            return Err(error(
                Span::default(),
                format!("Unable to find the entry point \"{entry_name}\""),
            ));
        };
        let is_main = program.entry == EntryPoint::Main;

        // #loader. A conditional script's result slot is reserved here, before
        // the call, so the whole program has somewhere to leave its answer.
        if !is_main {
            self.declare(&Type::Int, Depth::Untracked, None, Span::default())?;
        }
        let first = if program.needs_globals {
            self.globals_label
        } else {
            self.function_labels[entry_index]
        };
        self.jump(op::JSR, first);
        self.simple(op::RETN, qualifier::NONE);

        if program.needs_globals {
            self.emit_globals(entry_index, is_main)?;
        }

        for index in &program.emit_order {
            self.emit_function(*index)?;
        }

        for (id, label) in self.labels.iter().enumerate() {
            if label.target.is_none() && !label.references.is_empty() {
                return Err(error(
                    Span::default(),
                    format!("internal error: jump target {id} was never resolved"),
                ));
            }
        }

        Ok(self.out.finish())
    }

    /// Emits the `#globals` routine: the globals, then the call into the entry.
    fn emit_globals(&mut self, entry_index: usize, is_main: bool) -> Result<(), CompileError> {
        let program = self.program;
        self.in_globals = true;
        self.resolve(self.globals_label);

        for global in &program.globals {
            // The running total already counts this global while its own
            // initializer is being emitted, so an initializer that reads an
            // earlier global gets the right offset.
            self.global_depth = global.slot + program.type_size(&global.ty);
            self.declare(&global.ty, Depth::Frame, global.init.as_ref(), global.span)?;
        }
        self.global_depth = program
            .globals
            .iter()
            .map(|global| program.type_size(&global.ty))
            .sum();

        self.simple(op::SAVEBP, qualifier::NONE);
        if !is_main {
            self.declare(&Type::Int, Depth::Expression, None, Span::default())?;
        }
        self.jump(op::JSR, self.function_labels[entry_index]);
        if !is_main {
            // Past the result slot just reserved, the saved base pointer, the
            // globals, and `#loader`'s own result slot.
            let depth = self.global_depth + 3;
            self.copy(op::CPDOWNSP, depth, 1);
            self.movsp(1, Depth::Expression);
        }
        self.simple(op::RESTOREBP, qualifier::NONE);
        self.movsp(self.global_depth, Depth::Frame);
        self.simple(op::RETN, qualifier::NONE);

        self.in_globals = false;
        Ok(())
    }

    fn emit_function(&mut self, index: usize) -> Result<(), CompileError> {
        let program = self.program;
        let function = &program.functions[index];
        let Some(body) = program.body(index) else {
            return Ok(());
        };

        self.resolve(self.function_labels[index]);
        self.return_label = self.new_label();
        self.return_size = program.type_size(&function.return_type);
        self.argument_size = arguments_size(program, &function.parameters);
        self.frame_depth = self.argument_size;
        self.expression_depth = 0;

        // Parameters sit below the locals, and the caller pushed them in
        // reverse, so the last parameter is the one nearest the stack pointer.
        let mut slot = self.argument_size;
        let mut parameters = Vec::new();
        for parameter in &function.parameters {
            slot -= program.type_size(&parameter.ty);
            parameters.push(Local {
                name: parameter.name.clone(),
                ty: parameter.ty.clone(),
                binding: Binding::Slot(slot),
            });
        }
        self.scopes.push(parameters);

        self.block(body)?;

        self.resolve(self.return_label);
        // The callee pops its own arguments, once, on the way out.
        self.movsp(self.argument_size, Depth::Frame);
        self.simple(op::RETN, qualifier::NONE);

        self.scopes.pop();
        Ok(())
    }

    // -----------------------------------------------------------------------
    // statements
    // -----------------------------------------------------------------------

    fn block(&mut self, block: &Block) -> Result<(), CompileError> {
        self.scopes.push(Vec::new());
        let before = self.frame_depth;
        for statement in &block.statements {
            self.statement(statement)?;
        }
        let locals = self.frame_depth - before;
        self.movsp(locals, Depth::Frame);
        self.scopes.pop();
        Ok(())
    }

    /// Declares one local, or binds one `const` with no code at all.
    fn var_decl(&mut self, declaration: &VarDecl) -> Result<(), CompileError> {
        if declaration.is_const {
            let Some(value) = declaration.init.as_ref() else {
                return Err(error(
                    declaration.span,
                    format!(
                        "Required value for constant \"{}\" missing.",
                        declaration.name
                    ),
                ));
            };
            self.declare_local(
                &declaration.name,
                &declaration.ty,
                Binding::Constant(simplify_constant(value)),
            );
            return Ok(());
        }
        let slot = self.frame_depth;
        self.declare_local(&declaration.name, &declaration.ty, Binding::Slot(slot));
        self.declare(
            &declaration.ty,
            Depth::Frame,
            declaration.init.as_ref(),
            declaration.span,
        )
    }

    fn statement(&mut self, statement: &Stmt) -> Result<(), CompileError> {
        match statement {
            Stmt::Empty(_) => Ok(()),
            Stmt::Block(block) => self.block(block),
            Stmt::VarDecl(declaration) => self.var_decl(declaration),
            Stmt::VarDecls { declarations, .. } => {
                for declaration in declarations {
                    self.var_decl(declaration)?;
                }
                Ok(())
            }
            Stmt::Expr { expr, .. } => {
                let ty = self.expression(expr)?;
                let size = self.program.type_size(&ty);
                self.movsp(size, Depth::Expression);
                Ok(())
            }
            Stmt::If {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                let end = self.new_label();
                let otherwise = self.new_label();
                self.condition(condition)?;
                self.jump(op::JZ, otherwise);
                self.statement(then_branch)?;
                // Emitted even with no `else`, which is where `JMP +6` comes
                // from. Both labels then resolve to the very next byte.
                self.jump(op::JMP, end);
                self.resolve(otherwise);
                if let Some(branch) = else_branch {
                    self.statement(branch)?;
                }
                self.resolve(end);
                Ok(())
            }
            Stmt::While {
                condition, body, ..
            } => {
                let test = self.new_label();
                let end = self.new_label();
                let again = self.new_label();
                let saved = (self.break_label, self.continue_label);
                self.break_label = Some(end);
                self.continue_label = Some(again);

                self.resolve(test);
                self.condition(condition)?;
                self.jump(op::JZ, end);
                self.statement(body)?;
                // `continue` lands on the back jump, not on the test. Folding
                // the two together is an optimization, and there are none.
                self.resolve(again);
                self.jump(op::JMP, test);
                self.resolve(end);

                (self.break_label, self.continue_label) = saved;
                Ok(())
            }
            Stmt::DoWhile {
                body, condition, ..
            } => {
                let start = self.new_label();
                let end = self.new_label();
                let test = self.new_label();
                let saved = (self.break_label, self.continue_label);
                self.break_label = Some(end);
                self.continue_label = Some(test);

                self.resolve(start);
                self.statement(body)?;
                self.resolve(test);
                self.condition(condition)?;
                self.jump(op::JZ, end);
                self.jump(op::JMP, start);
                self.resolve(end);

                (self.break_label, self.continue_label) = saved;
                Ok(())
            }
            Stmt::For {
                init,
                condition,
                step,
                body,
                ..
            } => {
                let test = self.new_label();
                let end = self.new_label();
                let increment = self.new_label();
                let saved = (self.break_label, self.continue_label);
                self.break_label = Some(end);
                self.continue_label = Some(increment);

                if let Some(init) = init {
                    self.expression(init)?;
                    self.discard_temporaries();
                }
                self.resolve(test);
                match condition {
                    Some(condition) => self.condition(condition)?,
                    // An absent condition is not "always true", it is a pushed
                    // 1 and a test that never fires.
                    None => self.constant_int(1),
                }
                self.jump(op::JZ, end);
                self.statement(body)?;
                self.resolve(increment);
                if let Some(step) = step {
                    self.expression(step)?;
                    self.discard_temporaries();
                }
                self.jump(op::JMP, test);
                self.resolve(end);

                (self.break_label, self.continue_label) = saved;
                Ok(())
            }
            Stmt::Switch { subject, body, .. } => self.switch(subject, body),
            Stmt::Case { .. } => {
                let key = statement as *const Stmt as usize;
                if let Some((_, label)) = self.case_labels.iter().find(|(at, _)| *at == key) {
                    self.resolve(*label);
                }
                Ok(())
            }
            Stmt::Break(span) => match self.break_label {
                // No stack cleanup. The version-100 generator forgets it, and
                // scripts in the wild depend on the exact bytes.
                Some(label) => {
                    self.jump(op::JMP, label);
                    Ok(())
                }
                None => Err(error(*span, "\"break\" outside of a loop or switch")),
            },
            Stmt::Continue(span) => match self.continue_label {
                Some(label) => {
                    self.jump(op::JMP, label);
                    Ok(())
                }
                None => Err(error(*span, "\"continue\" outside of a loop")),
            },
            Stmt::Return { value, span } => self.return_statement(value.as_ref(), *span),
        }
    }

    fn return_statement(&mut self, value: Option<&Expr>, span: Span) -> Result<(), CompileError> {
        if let Some(value) = value {
            let ty = self.expression(value)?;
            if self.program.type_size(&ty) != self.return_size {
                return Err(error(span, "Return type does not match function type"));
            }
            let depth = self.return_size + self.frame_depth + self.expression_depth;
            self.copy(op::CPDOWNSP, depth, self.return_size);
        }
        // Untracked on purpose: the counters keep the value they had, which is
        // what leaves the dead `MOVSP` below the jump.
        let pop = self.frame_depth + self.expression_depth - self.argument_size;
        self.movsp(pop, Depth::Untracked);
        self.jump(op::JMP, self.return_label);
        if value.is_some() {
            self.movsp(self.return_size, Depth::Expression);
        }
        Ok(())
    }

    fn switch(&mut self, subject: &Expr, body: &Stmt) -> Result<(), CompileError> {
        let end = self.new_label();
        let saved_break = self.break_label;
        let saved_cases = std::mem::take(&mut self.case_labels);
        let saved_default = self.default_label.take();
        self.break_label = Some(end);

        let ty = self.expression(subject)?;
        if ty != Type::Int {
            return Err(error(
                subject.span(),
                "Switch expressions must be of type int",
            ));
        }
        self.scan_cases(body)?;
        let fallback = self.default_label.unwrap_or(end);
        self.jump(op::JMP, fallback);

        // The subject stops being a temporary and starts being a local, so
        // that anything declared in the body addresses itself correctly.
        self.frame_depth += 1;
        self.expression_depth -= 1;

        self.statement(body)?;

        self.resolve(end);
        self.movsp(1, Depth::Frame);

        self.break_label = saved_break;
        self.case_labels = saved_cases;
        self.default_label = saved_default;
        Ok(())
    }

    /// Emits the comparison chain that dispatches a `switch`.
    ///
    /// There is no jump table: each `case` gets a duplicate of the subject, a
    /// comparison, and a conditional jump, in source order. `default` gets no
    /// comparison, only the label the fall-through jump uses.
    fn scan_cases(&mut self, statement: &Stmt) -> Result<(), CompileError> {
        match statement {
            Stmt::Block(block) => {
                for inner in &block.statements {
                    self.scan_cases(inner)?;
                }
                Ok(())
            }
            Stmt::Case {
                label: Some(value), ..
            } => {
                let target = self.new_label();
                self.case_labels
                    .push((statement as *const Stmt as usize, target));
                self.copy(op::CPTOPSP, 1, 1);
                let ty = self.expression(&simplify_constant(value))?;
                if ty != Type::Int {
                    return Err(error(value.span(), "Case values must be of type int"));
                }
                self.binary(
                    op::EQUAL,
                    false,
                    &Type::Int,
                    &Type::Int,
                    &Type::Int,
                    value.span(),
                )?;
                self.jump(op::JNZ, target);
                Ok(())
            }
            Stmt::Case { label: None, .. } => {
                let target = self.new_label();
                self.case_labels
                    .push((statement as *const Stmt as usize, target));
                self.default_label = Some(target);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Emits a condition and checks it is something a `JZ` can test.
    fn condition(&mut self, expr: &Expr) -> Result<(), CompileError> {
        let ty = self.expression(expr)?;
        if ty != Type::Int {
            return Err(error(expr.span(), "Conditions must be of type int"));
        }
        Ok(())
    }

    /// Drops whatever an evaluated-for-effect expression left behind.
    fn discard_temporaries(&mut self) {
        let depth = self.expression_depth;
        self.movsp(depth, Depth::Expression);
    }

    // -----------------------------------------------------------------------
    // expressions
    // -----------------------------------------------------------------------

    fn expression(&mut self, expr: &Expr) -> Result<Type, CompileError> {
        match expr {
            Expr::IntLit { value, .. } => {
                self.constant_int(*value);
                Ok(Type::Int)
            }
            Expr::FloatLit { value, .. } => {
                self.simple(op::CONST, qualifier::FLOAT);
                self.out.write_f32(*value);
                self.expression_depth += 1;
                Ok(Type::Float)
            }
            Expr::StrLit { value, span } => {
                let length = u16::try_from(value.len())
                    .map_err(|_| error(*span, "String constant is longer than 65535 bytes"))?;
                self.simple(op::CONST, qualifier::STR);
                self.out.write_u16(length);
                self.out.write_bytes(value);
                self.expression_depth += 1;
                Ok(Type::Str)
            }
            Expr::ObjectLit { value, .. } => {
                self.simple(op::CONST, qualifier::OBJECT);
                self.out.write_u32(*value);
                self.expression_depth += 1;
                Ok(Type::Object)
            }
            Expr::VectorLit { x, y, z, .. } => {
                for component in [x, y, z] {
                    self.simple(op::CONST, qualifier::FLOAT);
                    self.out.write_f32(*component);
                    self.expression_depth += 1;
                }
                Ok(Type::Vector)
            }
            Expr::Name { name, span } => {
                // A constant has no storage, so a use of one emits the
                // initializer again rather than reading anything.
                if let Some(local) = self.lookup(name) {
                    if let Binding::Constant(value) = local.binding {
                        self.expression(&value)?;
                        return Ok(local.ty);
                    }
                }
                if let Some(place) = self.place(expr)? {
                    let ty = place.ty.clone();
                    self.read(&place, None);
                    return Ok(ty);
                }
                if let Some((ty, value)) = self.program.constant(name) {
                    let (ty, value) = (ty.clone(), value.clone());
                    self.expression(&value)?;
                    return Ok(ty);
                }
                Err(error(*span, format!("Undeclared identifier \"{name}\"")))
            }
            Expr::Unary { op, operand, span } => self.unary(*op, operand, *span),
            Expr::Postfix { op, operand, span } => {
                let step = match op {
                    PostfixOp::Increment => Step::Increment,
                    PostfixOp::Decrement => Step::Decrement,
                };
                self.step(operand, step, false, *span)
            }
            Expr::Binary {
                op,
                left,
                right,
                span,
            } => self.binary_expression(*op, left, right, *span),
            Expr::Assign {
                op,
                target,
                value,
                span,
            } => self.assign(*op, target, value, *span),
            Expr::Conditional {
                condition,
                then_value,
                else_value,
                span,
            } => {
                let end = self.new_label();
                let otherwise = self.new_label();
                self.condition(condition)?;
                self.jump(op::JZ, otherwise);
                let ty = self.expression(then_value)?;
                // The arms are mutually exclusive, so only one of them really
                // pushes. Take the double count back off.
                self.expression_depth -= self.program.type_size(&ty);
                self.jump(op::JMP, end);
                self.resolve(otherwise);
                let other = self.expression(else_value)?;
                self.resolve(end);
                if ty != other {
                    return Err(error(
                        *span,
                        "Resulting values for in a conditional must match",
                    ));
                }
                Ok(ty)
            }
            Expr::Call {
                callee,
                arguments,
                span,
            } => self.call(callee, arguments, *span),
            Expr::Member { span, .. } => {
                if let Some(place) = self.place(expr)? {
                    let ty = place.ty.clone();
                    self.read(&place, None);
                    return Ok(ty);
                }
                // The value is not in a variable, so it has to be built on the
                // stack and then cut down to the member.
                let Expr::Member { value, field, .. } = expr else {
                    unreachable!("the outer match already matched a member access")
                };
                let source = self.expression(value)?;
                let (offset, ty) = self.member(&source, field, *span)?;
                let total = self.program.type_size(&source);
                let size = self.program.type_size(&ty);
                self.destruct(total, offset, size);
                Ok(ty)
            }
        }
    }

    fn unary(&mut self, op: UnaryOp, operand: &Expr, span: Span) -> Result<Type, CompileError> {
        match op {
            // Accepted and does nothing, which is what the reference does.
            UnaryOp::Plus => {
                let ty = self.expression(operand)?;
                if ty != Type::Int && ty != Type::Float {
                    return Err(operator_error(span, "+", &ty));
                }
                Ok(ty)
            }
            UnaryOp::Negate => {
                let ty = self.expression(operand)?;
                if ty != Type::Int && ty != Type::Float {
                    return Err(operator_error(span, "-", &ty));
                }
                let code = if ty == Type::Int {
                    qualifier::INT
                } else {
                    qualifier::FLOAT
                };
                self.simple(op::NEG, code);
                Ok(ty)
            }
            UnaryOp::Not => {
                let ty = self.expression(operand)?;
                if ty != Type::Int {
                    return Err(operator_error(span, "!", &ty));
                }
                self.simple(op::NOT, qualifier::INT);
                Ok(Type::Int)
            }
            UnaryOp::BitNot => {
                let ty = self.expression(operand)?;
                if ty != Type::Int {
                    return Err(operator_error(span, "~", &ty));
                }
                self.simple(op::COMP, qualifier::INT);
                Ok(Type::Int)
            }
            UnaryOp::PreIncrement => self.step(operand, Step::Increment, true, span),
            UnaryOp::PreDecrement => self.step(operand, Step::Decrement, true, span),
        }
    }

    /// Emits `++x`, `x++`, `--x` or `x--`.
    ///
    /// The update is fused into the variable reference rather than being a
    /// separate load and store, and the copy is always emitted even when the
    /// value is thrown away a moment later.
    fn step(
        &mut self,
        operand: &Expr,
        step: Step,
        prefix: bool,
        span: Span,
    ) -> Result<Type, CompileError> {
        let Some(place) = self.place(operand)? else {
            return Err(error(span, "Left hand side of assignment not a variable"));
        };
        if place.ty != Type::Int {
            return Err(error(
                span,
                "Increment and decrement only work on an int variable",
            ));
        }
        self.read(&place, Some((step, prefix)));
        Ok(Type::Int)
    }

    fn binary_expression(
        &mut self,
        operator: BinaryOp,
        left: &Expr,
        right: &Expr,
        span: Span,
    ) -> Result<Type, CompileError> {
        if operator == BinaryOp::LogicalAnd {
            let end = self.new_label();
            let lhs = self.expression(left)?;
            self.copy(op::CPTOPSP, 1, 1);
            self.jump(op::JZ, end);
            let rhs = self.expression(right)?;
            if lhs != Type::Int || rhs != Type::Int {
                return Err(operator_error(span, "&&", &lhs));
            }
            self.binary(op::LOGAND, false, &Type::Int, &Type::Int, &Type::Int, span)?;
            self.resolve(end);
            return Ok(Type::Int);
        }
        if operator == BinaryOp::LogicalOr {
            // The second test is meant to be a jump. It is not, so the right
            // operand always runs. Reproducing that is the whole point.
            let end = self.new_label();
            let rhs_label = self.new_label();
            let lhs = self.expression(left)?;
            self.copy(op::CPTOPSP, 1, 1);
            self.jump(op::JZ, rhs_label);
            self.copy(op::CPTOPSP, 1, 1);
            self.jump(op::JZ, end);
            self.resolve(rhs_label);
            let rhs = self.expression(right)?;
            self.resolve(end);
            if lhs != Type::Int || rhs != Type::Int {
                return Err(operator_error(span, "||", &lhs));
            }
            self.binary(op::LOGOR, false, &Type::Int, &Type::Int, &Type::Int, span)?;
            return Ok(Type::Int);
        }

        let lhs = self.expression(left)?;
        let rhs = self.expression(right)?;
        let Some(result) = result_type(operator, &lhs, &rhs) else {
            return Err(operator_error(span, operator.as_str(), &lhs));
        };
        let compares = matches!(operator, BinaryOp::Eq | BinaryOp::NotEq);
        self.binary(opcode_for(operator), compares, &result, &lhs, &rhs, span)?;
        Ok(result)
    }

    fn assign(
        &mut self,
        operator: AssignOp,
        target: &Expr,
        value: &Expr,
        span: Span,
    ) -> Result<Type, CompileError> {
        let Some(place) = self.place(target)? else {
            return Err(error(span, "Left hand side of assignment not a variable"));
        };
        let ty = place.ty.clone();

        match operator.binary_op() {
            Some(operator) => {
                self.read(&place, None);
                let rhs = self.expression(value)?;
                let Some(result) = result_type(operator, &ty, &rhs) else {
                    return Err(operator_error(span, operator.as_str(), &ty));
                };
                if result != ty {
                    return Err(operator_error(span, operator.as_str(), &ty));
                }
                let compares = matches!(operator, BinaryOp::Eq | BinaryOp::NotEq);
                self.binary(opcode_for(operator), compares, &result, &ty, &rhs, span)?;
            }
            None => {
                let rhs = self.expression(value)?;
                if rhs != ty {
                    return Err(error(span, "Assignment type mismatch"));
                }
            }
        }

        // The stored value stays on the stack. Whatever wraps the assignment
        // is what drops it.
        self.write(&place);
        Ok(ty)
    }

    fn call(
        &mut self,
        callee: &Expr,
        arguments: &[Expr],
        span: Span,
    ) -> Result<Type, CompileError> {
        let Expr::Name { name, .. } = callee else {
            return Err(error(span, "Only a named function can be called"));
        };
        let program = self.program;

        if let Some((index, function)) = program.function(name) {
            let return_type = function.return_type.clone();
            let parameters = function.parameters.clone();
            if return_type != Type::Void {
                self.declare(&return_type, Depth::Expression, None, span)?;
            }
            let size = arguments_size(program, &parameters);
            self.push_arguments(name, &parameters, arguments, span)?;
            self.jump(op::JSR, self.function_labels[index]);
            // The callee pops them; this only keeps the counter honest.
            self.expression_depth -= size;
            return Ok(return_type);
        }

        if let Some(routine) = program.nwscript.routine(name) {
            let parameters = routine_parameters(routine);
            let size = arguments_size(program, &parameters);
            self.push_arguments(name, &parameters, arguments, span)?;
            let count = u8::try_from(parameters.len())
                .map_err(|_| error(span, format!("Routine \"{name}\" takes too many arguments")))?;
            self.simple(op::ACTION, qualifier::NONE);
            self.out.write_u16(routine.id);
            self.out.write_u8(count);
            self.expression_depth -= size;
            self.expression_depth += program.type_size(&routine.return_type);
            return Ok(routine.return_type.clone());
        }

        Err(error(span, format!("Undeclared identifier \"{name}\"")))
    }

    /// Pushes a call's arguments, last one first.
    ///
    /// An argument the call left out is filled in from the declaration and
    /// emitted here, at the call site, exactly as if it had been written.
    fn push_arguments(
        &mut self,
        name: &str,
        parameters: &[Parameter],
        arguments: &[Expr],
        span: Span,
    ) -> Result<(), CompileError> {
        if arguments.len() > parameters.len() {
            return Err(error(
                span,
                format!("Too many arguments specified in call to \"{name}\""),
            ));
        }
        for index in (0..parameters.len()).rev() {
            let parameter = &parameters[index];
            let supplied = arguments.get(index).or(parameter.default.as_ref());
            let Some(argument) = supplied else {
                return Err(error(
                    span,
                    format!(
                        "Required argument \"{}\" missing in call to \"{name}\"",
                        parameter.name
                    ),
                ));
            };
            let argument = argument.clone();

            if parameter.ty == Type::Action {
                self.store_state(&argument)?;
                continue;
            }
            let ty = self.expression(&argument)?;
            if ty != parameter.ty {
                return Err(error(
                    argument.span(),
                    format!(
                        "Type mismatch in argument \"{}\" of call to \"{name}\"",
                        parameter.name
                    ),
                ));
            }
        }
        Ok(())
    }

    /// Emits a deferred call: a snapshot, a jump over the body, then the body.
    ///
    /// The virtual machine resumes a snapshot at a fixed offset of sixteen
    /// bytes, which is exactly the snapshot plus the jump, so nothing may come
    /// between them.
    fn store_state(&mut self, argument: &Expr) -> Result<(), CompileError> {
        let end = self.new_label();
        let pending = self.expression_depth;
        // Temporaries already pushed for this call belong to the frame the
        // snapshot captures.
        self.frame_depth += pending;
        self.expression_depth = 0;

        self.simple(op::STORE_STATE, qualifier::STORED_STATE);
        let globals = self.global_depth * 4;
        let frame = (self.return_size + self.frame_depth) * 4;
        self.out.write_u32(globals as u32);
        self.out.write_u32(frame as u32);
        self.jump(op::JMP, end);
        self.expression(argument)?;
        self.simple(op::RETN, qualifier::NONE);
        self.resolve(end);

        self.frame_depth -= pending;
        self.expression_depth = pending;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // variables
    // -----------------------------------------------------------------------

    /// Returns the innermost binding of `name`, if any is in scope.
    fn lookup(&self, name: &str) -> Option<Local> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.iter().rev().find(|local| local.name == name))
            .cloned()
    }

    /// Resolves an expression to storage, or `None` if it is not storage.
    fn place(&mut self, expr: &Expr) -> Result<Option<Place>, CompileError> {
        match expr {
            Expr::Name { name, .. } => {
                if let Some(local) = self.lookup(name) {
                    return Ok(match local.binding {
                        Binding::Slot(slot) => Some(Place {
                            storage: Storage::Local,
                            slot,
                            source_ty: local.ty.clone(),
                            ty: local.ty,
                            element: None,
                        }),
                        Binding::Constant(_) => None,
                    });
                }
                Ok(self.program.global(name).map(|global| Place {
                    storage: Storage::Global,
                    slot: global.slot,
                    source_ty: global.ty.clone(),
                    ty: global.ty.clone(),
                    element: None,
                }))
            }
            Expr::Member { value, field, span } => {
                let Some(place) = self.place(value)? else {
                    return Ok(None);
                };
                let (offset, ty) = self.member(&place.ty, field, *span)?;
                Ok(Some(Place {
                    element: Some(place.element.unwrap_or(0) + offset),
                    ty,
                    ..place
                }))
            }
            _ => Ok(None),
        }
    }

    /// Returns the slot offset and type of `field` within `ty`.
    fn member(&self, ty: &Type, field: &str, span: Span) -> Result<(i32, Type), CompileError> {
        match ty {
            Type::Vector => match field {
                "x" => Ok((0, Type::Float)),
                "y" => Ok((1, Type::Float)),
                "z" => Ok((2, Type::Float)),
                _ => Err(error(
                    span,
                    format!("Unknown structure element \"{field}\""),
                )),
            },
            Type::Struct(name) => {
                let Some(layout) = self.program.struct_layout(name) else {
                    return Err(error(span, format!("Undeclared identifier \"{name}\"")));
                };
                match layout.field(field) {
                    Some(field) => Ok((field.slot, field.ty.clone())),
                    None => Err(error(
                        span,
                        format!("Unknown structure element \"{field}\""),
                    )),
                }
            }
            _ => Err(error(
                span,
                format!("\"{}\" has no members", ty.display_name()),
            )),
        }
    }

    /// Emits the address arithmetic and the copy for a variable reference.
    ///
    /// Reading one member of a variable copies the *whole* variable to the top
    /// of the stack and then cuts it down. That is what the unoptimized
    /// generator does, and shortening it to a single narrow copy changes the
    /// bytes.
    fn read(&mut self, place: &Place, step: Option<(Step, bool)>) {
        self.access(place, true, step)
    }

    fn write(&mut self, place: &Place) {
        self.access(place, false, None)
    }

    fn access(&mut self, place: &Place, to_top: bool, step: Option<(Step, bool)>) {
        let program = self.program;
        let element = place.element.unwrap_or(0);
        let size = program.type_size(&place.ty);

        let (code, address, sp_relative) = match place.storage {
            // Globals are reached through the base pointer, and the base
            // pointer does not move as an expression pushes, so temporaries
            // are deliberately left out of the sum.
            Storage::Global if !self.in_globals => (
                if to_top { op::CPTOPBP } else { op::CPDOWNBP },
                self.global_depth - place.slot,
                false,
            ),
            Storage::Global => (
                if to_top { op::CPTOPSP } else { op::CPDOWNSP },
                self.global_depth + self.expression_depth - place.slot,
                true,
            ),
            Storage::Local => (
                if to_top { op::CPTOPSP } else { op::CPDOWNSP },
                self.frame_depth + self.expression_depth - place.slot,
                true,
            ),
        };

        if let Some((step, true)) = step {
            self.increment(step, sp_relative, address - element);
        }

        if !to_top || place.element.is_none() {
            self.copy(code, address - element, size);
        } else {
            let total = program.type_size(&place.source_ty);
            self.copy(code, address, total);
            self.destruct(total, element, size);
        }

        if let Some((step, false)) = step {
            // The copy just pushed a slot, so the variable is one deeper now.
            self.increment(step, sp_relative, address - element + 1);
        }
    }

    fn increment(&mut self, step: Step, sp_relative: bool, address: i32) {
        let code = match (step, sp_relative) {
            (Step::Increment, true) => op::INCISP,
            (Step::Increment, false) => op::INCIBP,
            (Step::Decrement, true) => op::DECISP,
            (Step::Decrement, false) => op::DECIBP,
        };
        self.simple(code, qualifier::INT);
        self.out.write_i32(-address * 4);
    }

    fn declare_local(&mut self, name: &str, ty: &Type, binding: Binding) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.push(Local {
                name: name.to_string(),
                ty: ty.clone(),
                binding,
            });
        }
    }

    /// Reserves space for a value, then runs its initializer into that space.
    fn declare(
        &mut self,
        ty: &Type,
        track: Depth,
        init: Option<&Expr>,
        span: Span,
    ) -> Result<(), CompileError> {
        self.reserve(ty, span)?;
        let size = self.program.type_size(ty);
        match track {
            Depth::Frame => self.frame_depth += size,
            Depth::Expression => self.expression_depth += size,
            Depth::Untracked => {}
        }
        if let Some(init) = init {
            let actual = self.expression(init)?;
            if actual != *ty {
                return Err(error(
                    init.span(),
                    "Declaration and initialial value type do not match",
                ));
            }
            self.copy(op::CPDOWNSP, size * 2, size);
            self.movsp(size, Depth::Expression);
        }
        Ok(())
    }

    /// Emits the `RSADD` sequence that zeroes a value of `ty`.
    fn reserve(&mut self, ty: &Type, span: Span) -> Result<(), CompileError> {
        match ty {
            Type::Int => self.simple(op::RSADD, qualifier::INT),
            Type::Float => self.simple(op::RSADD, qualifier::FLOAT),
            Type::Str => self.simple(op::RSADD, qualifier::STR),
            Type::Object => self.simple(op::RSADD, qualifier::OBJECT),
            Type::Engine(engine) => self.simple(op::RSADD, engine_qualifier(engine)),
            Type::Vector => {
                for _ in 0..3 {
                    self.simple(op::RSADD, qualifier::FLOAT);
                }
            }
            Type::Struct(name) => {
                let Some(layout) = self.program.struct_layout(name) else {
                    return Err(error(span, format!("Undeclared identifier \"{name}\"")));
                };
                let fields: Vec<Type> =
                    layout.fields.iter().map(|field| field.ty.clone()).collect();
                for field in fields {
                    self.reserve(&field, span)?;
                }
            }
            Type::Void | Type::Action => {
                return Err(error(
                    span,
                    format!("A variable cannot be of type {}", ty.display_name()),
                ))
            }
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // instruction emission
    // -----------------------------------------------------------------------

    fn simple(&mut self, code: u8, qualifier: u8) {
        self.out.write_u8(code);
        self.out.write_u8(qualifier);
    }

    fn constant_int(&mut self, value: i32) {
        self.simple(op::CONST, qualifier::INT);
        self.out.write_i32(value);
        self.expression_depth += 1;
    }

    fn copy(&mut self, code: u8, address: i32, count: i32) {
        self.simple(code, qualifier::STRUCTURAL);
        self.out.write_i32(-address * 4);
        self.out.write_u16((count * 4) as u16);
        if code == op::CPTOPSP || code == op::CPTOPBP {
            self.expression_depth += count;
        }
    }

    fn destruct(&mut self, total: i32, element: i32, size: i32) {
        self.simple(op::DESTRUCT, qualifier::STRUCTURAL);
        self.out.write_u16((total * 4) as u16);
        self.out.write_i16((element * 4) as i16);
        self.out.write_u16((size * 4) as u16);
        self.expression_depth += size - total;
    }

    fn movsp(&mut self, count: i32, track: Depth) {
        // A zero-sized move is not emitted at all, which is the one thing the
        // reference does drop.
        if count == 0 {
            return;
        }
        self.simple(op::MOVSP, qualifier::NONE);
        self.out.write_i32(-count * 4);
        match track {
            Depth::Frame => self.frame_depth -= count,
            Depth::Expression => self.expression_depth -= count,
            Depth::Untracked => {}
        }
    }

    fn binary(
        &mut self,
        code: u8,
        compares: bool,
        result: &Type,
        lhs: &Type,
        rhs: &Type,
        span: Span,
    ) -> Result<(), CompileError> {
        let Some((qualifier, extra)) = self.pair_qualifier(lhs, rhs, compares) else {
            return Err(error(
                span,
                format!(
                    "Operator not valid for {} and {}",
                    lhs.display_name(),
                    rhs.display_name()
                ),
            ));
        };
        let program = self.program;
        self.expression_depth +=
            program.type_size(result) - (program.type_size(lhs) + program.type_size(rhs));
        self.simple(code, qualifier);
        if let Some(extra) = extra {
            self.out.write_u16(extra);
        }
        Ok(())
    }

    fn pair_qualifier(&self, lhs: &Type, rhs: &Type, compares: bool) -> Option<(u8, Option<u16>)> {
        use Type::*;
        Some(match (lhs, rhs) {
            (Int, Int) => (qualifier::INT_INT, None),
            (Int, Float) => (qualifier::INT_FLOAT, None),
            (Float, Int) => (qualifier::FLOAT_INT, None),
            (Float, Float) => (qualifier::FLOAT_FLOAT, None),
            (Float, Vector) => (qualifier::FLOAT_VECTOR, None),
            (Vector, Float) => (qualifier::VECTOR_FLOAT, None),
            (Object, Object) => (qualifier::OBJECT_OBJECT, None),
            (Str, Str) => (qualifier::STR_STR, None),
            (Vector, Vector) if compares => (qualifier::AGGREGATE, Some(12)),
            (Vector, Vector) => (qualifier::VECTOR_VECTOR, None),
            (Engine(left), Engine(right)) if left.index == right.index => {
                (qualifier::ENGINE_PAIR.wrapping_add(left.index), None)
            }
            (Struct(left), Struct(right)) if left == right && compares => {
                let size = self.program.type_size(lhs);
                (qualifier::AGGREGATE, Some((size * 4) as u16))
            }
            _ => return None,
        })
    }

    // -----------------------------------------------------------------------
    // labels
    // -----------------------------------------------------------------------

    fn new_label(&mut self) -> LabelId {
        self.labels.push(Label::default());
        self.labels.len() - 1
    }

    /// Emits a branch, and remembers to come back and fill the target in.
    fn jump(&mut self, code: u8, label: LabelId) {
        let at = self.out.len();
        self.simple(code, qualifier::NONE);
        match self.labels[label].target {
            Some(target) => self.out.write_i32(target as i32 - at as i32),
            None => {
                self.labels[label].references.push(at);
                self.out.write_i32(0);
            }
        }
        if code == op::JZ || code == op::JNZ {
            self.expression_depth -= 1;
        }
    }

    fn resolve(&mut self, label: LabelId) {
        let target = self.out.len();
        self.labels[label].target = Some(target);
        let references = std::mem::take(&mut self.labels[label].references);
        for at in references {
            // A displacement counts from the first byte of the branch, not
            // from the byte after it.
            let displacement = target as i32 - at as i32;
            let _ = self.out.patch_u32(at + 2, displacement as u32);
        }
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn error(span: Span, message: impl Into<String>) -> CompileError {
    CompileError::from_diagnostic(Diagnostic::error(Stage::Codegen, span, message))
}

fn operator_error(span: Span, operator: &str, ty: &Type) -> CompileError {
    error(
        span,
        format!(
            "Operator ({operator}) not valid for specified types ({})",
            ty.display_name()
        ),
    )
}

fn engine_qualifier(engine: &EngineType) -> u8 {
    qualifier::ENGINE.wrapping_add(engine.index)
}

fn arguments_size(program: &AnalyzedScript<'_>, parameters: &[Parameter]) -> i32 {
    parameters
        .iter()
        .map(|parameter| program.type_size(&parameter.ty))
        .sum()
}

fn routine_parameters(routine: &Routine) -> Vec<Parameter> {
    routine
        .parameters
        .iter()
        .map(|parameter| Parameter {
            ty: parameter.ty.clone(),
            name: parameter.name.clone(),
            default: parameter.default.clone(),
            is_const: false,
            span: routine.span,
        })
        .collect()
}

fn opcode_for(operator: BinaryOp) -> u8 {
    match operator {
        BinaryOp::Add => op::ADD,
        BinaryOp::Sub => op::SUB,
        BinaryOp::Mul => op::MUL,
        BinaryOp::Div => op::DIV,
        BinaryOp::Mod => op::MOD,
        BinaryOp::Eq => op::EQUAL,
        BinaryOp::NotEq => op::NEQUAL,
        BinaryOp::Less => op::LT,
        BinaryOp::LessEq => op::LEQ,
        BinaryOp::Greater => op::GT,
        BinaryOp::GreaterEq => op::GEQ,
        BinaryOp::LogicalAnd => op::LOGAND,
        BinaryOp::LogicalOr => op::LOGOR,
        BinaryOp::BitAnd => op::BOOLAND,
        BinaryOp::BitOr => op::INCOR,
        BinaryOp::BitXor => op::EXCOR,
        BinaryOp::Shl => op::SHLEFT,
        BinaryOp::Shr => op::SHRIGHT,
        BinaryOp::UShr => op::USHRIGHT,
    }
}

/// Returns what an operator produces from a pair of operand types.
///
/// There is no implicit conversion anywhere. `int + float` is a real mixed
/// instruction, `ADD` with the `int, float` qualifier, not an int widened
/// first.
fn result_type(operator: BinaryOp, lhs: &Type, rhs: &Type) -> Option<Type> {
    use BinaryOp::*;
    use Type::*;
    match operator {
        Add => Some(match (lhs, rhs) {
            (Int, Int) => Int,
            (Float, Float) | (Int, Float) | (Float, Int) => Float,
            (Str, Str) => Str,
            (Vector, Vector) => Vector,
            _ => return None,
        }),
        Sub => Some(match (lhs, rhs) {
            (Int, Int) => Int,
            (Float, Float) | (Int, Float) | (Float, Int) => Float,
            (Vector, Vector) => Vector,
            _ => return None,
        }),
        Mul | Div => Some(match (lhs, rhs) {
            (Int, Int) => Int,
            (Float, Float) | (Int, Float) | (Float, Int) => Float,
            (Vector, Float) | (Float, Vector) => Vector,
            _ => return None,
        }),
        Mod => match (lhs, rhs) {
            (Int, Int) => Some(Int),
            _ => None,
        },
        Less | LessEq | Greater | GreaterEq => match (lhs, rhs) {
            (Int, Int) | (Float, Float) | (Int, Float) | (Float, Int) => Some(Int),
            _ => None,
        },
        Eq | NotEq => (lhs == rhs).then_some(Int),
        LogicalAnd | LogicalOr | BitAnd | BitOr | BitXor | Shl | Shr | UShr => match (lhs, rhs) {
            (Int, Int) => Some(Int),
            _ => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nwscript::{Game, NwScript};
    use crate::{compile, CompileOptions};

    fn build(source: &str) -> Vec<u8> {
        let nwscript = NwScript::empty(Game::K1);
        let options = CompileOptions::new(&nwscript);
        compile(source, &options).expect("the fixture compiles")
    }

    #[test]
    fn optimization_defaults_to_off() {
        assert_eq!(OptimizationLevel::default(), OptimizationLevel::None);
    }

    #[test]
    fn an_empty_main_is_twenty_three_bytes() {
        let bytes = build("void main() { }");
        assert_eq!(
            bytes,
            vec![
                b'N', b'C', b'S', b' ', b'V', b'1', b'.', b'0', 0x42, 0, 0, 0, 23, //
                0x1e, 0x00, 0x00, 0x00, 0x00, 0x08, // JSR +8
                0x20, 0x00, // RETN
                0x20, 0x00, // RETN, main
            ]
        );
    }

    #[test]
    fn an_if_without_an_else_still_jumps_to_the_next_instruction() {
        let bytes = build("void main() { int n; if (n) { } }");
        let jump = [0x1d, 0x00, 0x00, 0x00, 0x00, 0x06];
        assert!(
            bytes.windows(6).any(|window| window == jump),
            "expected a JMP +6, got {bytes:02x?}"
        );
    }

    #[test]
    fn nothing_is_folded() {
        let bytes = build("void main() { int n = 2 + 3; }");
        // Two pushes and an add, not a single push of 5.
        assert!(bytes.windows(6).any(|w| w == [0x04, 0x03, 0, 0, 0, 2]));
        assert!(bytes.windows(6).any(|w| w == [0x04, 0x03, 0, 0, 0, 3]));
        assert!(bytes.windows(2).any(|w| w == [0x14, 0x20]));
        assert!(!bytes.windows(6).any(|w| w == [0x04, 0x03, 0, 0, 0, 5]));
    }

    #[test]
    fn a_bare_struct_declaration_forces_a_globals_block() {
        let bytes = build("struct Pair { int a; }; void main() { }");
        // SAVEBP only appears in a #globals routine.
        assert!(bytes.windows(2).any(|w| w == [0x2a, 0x00]));
    }
}
