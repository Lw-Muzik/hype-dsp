//! EEL2-subset compiler: walks the [`Ast`] and emits a flat postfix [`Program`]
//! consisting of [`Op`] opcodes.
//!
//! # Design decisions
//!
//! * **Register file**: three reserved slots — `spl0`→0, `spl1`→1, `srate`→2.
//!   Every other variable is allocated on first use starting at index 3.
//!
//! * **Jump targets**: absolute indices into the owning `Vec<Op>`.  Forward
//!   jumps are emitted with a placeholder (`u32::MAX`) then backpatched once the
//!   target is known.
//!
//! * **`loop(n)`**: compiled with a hidden counter register so the VM never
//!   needs to handle floating-point loop counts specially.
//!
//! * **Validation**: unknown function names, wrong argument counts, and
//!   assignments to the read-only `srate` register produce a [`ScriptError`]
//!   with `line=0, col=0` (position information is not preserved in the AST).

use std::collections::HashMap;

use super::parser::{AssignOp, Ast, BinOp, Const, Expr, Stmt, UnOp};
use super::ScriptError;

// ─────────────────────────────────────────────────────────────────────────────
// Public types
// ─────────────────────────────────────────────────────────────────────────────

/// A single opcode in the postfix instruction stream.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    // Stack manipulation
    PushConst(f32),
    LoadReg(u16),
    StoreReg(u16),
    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Neg,
    // Comparison / logic
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Not,
    /// Function call — pops `arity` args (right-most on top), pushes result.
    Call(Builtin),
    /// Control flow — operand is an absolute index into the containing Vec<Op>.
    JumpIfFalse(u32),
    Jump(u32),
    /// Discard the top-of-stack (used for Stmt::Expr and loop bodies).
    Pop,
}

/// Mathematical built-in functions.
///
/// This enum is the **single source of truth** shared between the compiler
/// (Task 4, validation & arity) and the VM (Task 5, dispatch).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    // 1-arg
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Sqrt,
    Exp,
    Log,
    Log10,
    Abs,
    Floor,
    Ceil,
    Round,
    Sign,
    Tanh,
    // 2-arg
    Atan2,
    Pow,
    Min,
    Max,
    Fmod,
}

impl Builtin {
    /// Look up a builtin by name, returning `(variant, arity)` or `None`.
    pub fn from_name(name: &str) -> Option<(Builtin, usize)> {
        match name {
            "sin" => Some((Builtin::Sin, 1)),
            "cos" => Some((Builtin::Cos, 1)),
            "tan" => Some((Builtin::Tan, 1)),
            "asin" => Some((Builtin::Asin, 1)),
            "acos" => Some((Builtin::Acos, 1)),
            "atan" => Some((Builtin::Atan, 1)),
            "sqrt" => Some((Builtin::Sqrt, 1)),
            "exp" => Some((Builtin::Exp, 1)),
            "log" => Some((Builtin::Log, 1)),
            "log10" => Some((Builtin::Log10, 1)),
            "abs" => Some((Builtin::Abs, 1)),
            "floor" => Some((Builtin::Floor, 1)),
            "ceil" => Some((Builtin::Ceil, 1)),
            "round" => Some((Builtin::Round, 1)),
            "sign" => Some((Builtin::Sign, 1)),
            "tanh" => Some((Builtin::Tanh, 1)),
            "atan2" => Some((Builtin::Atan2, 2)),
            "pow" => Some((Builtin::Pow, 2)),
            "min" => Some((Builtin::Min, 2)),
            "max" => Some((Builtin::Max, 2)),
            "fmod" => Some((Builtin::Fmod, 2)),
            _ => None,
        }
    }
}

/// A compiled program ready for the VM.
#[derive(Debug, Clone)]
pub struct Program {
    /// Opcodes for the `@init` section (run once on load).
    pub init: Vec<Op>,
    /// Opcodes for the `@sample` section (run per audio frame).
    pub sample: Vec<Op>,
    /// Total number of registers needed (`spl0/spl1/srate` + user vars).
    pub num_regs: usize,
    /// Register index for `spl0` (always 0).
    pub spl0_reg: u16,
    /// Register index for `spl1` (always 1).
    pub spl1_reg: u16,
    /// Register index for `srate` (always 2, read-only at script level).
    pub srate_reg: u16,
}

// ─────────────────────────────────────────────────────────────────────────────
// Compiler internals
// ─────────────────────────────────────────────────────────────────────────────

/// Build a `ScriptError` with no source position (compiler-stage errors).
fn compiler_err(msg: impl Into<String>) -> ScriptError {
    ScriptError {
        line: 0,
        col: 0,
        message: msg.into(),
    }
}

struct Compiler {
    /// `var_name → register_index` (including spl0/spl1/srate).
    syms: HashMap<String, u16>,
    /// Next free register index.
    next_reg: u16,
}

impl Compiler {
    fn new() -> Self {
        let mut syms = HashMap::new();
        // Reserve fixed registers for the special variables.
        syms.insert("spl0".to_string(), 0);
        syms.insert("spl1".to_string(), 1);
        syms.insert("srate".to_string(), 2);
        Compiler { syms, next_reg: 3 }
    }

    /// Look up or allocate a register for `name`.
    fn reg_of(&mut self, name: &str) -> u16 {
        if let Some(&idx) = self.syms.get(name) {
            return idx;
        }
        let idx = self.next_reg;
        self.next_reg += 1;
        self.syms.insert(name.to_string(), idx);
        idx
    }

    // ── Statement compilation ─────────────────────────────────────────────

    fn compile_stmts(&mut self, stmts: &[Stmt], out: &mut Vec<Op>) -> Result<(), ScriptError> {
        for stmt in stmts {
            self.compile_stmt(stmt, out)?;
        }
        Ok(())
    }

    fn compile_stmt(&mut self, stmt: &Stmt, out: &mut Vec<Op>) -> Result<(), ScriptError> {
        match stmt {
            Stmt::Assign { name, op, value } => {
                if name == "srate" {
                    return Err(compiler_err("srate is read-only"));
                }
                let reg = self.reg_of(name);
                match op {
                    AssignOp::Eq => {
                        self.compile_expr(value, out)?;
                    }
                    AssignOp::PlusEq => {
                        out.push(Op::LoadReg(reg));
                        self.compile_expr(value, out)?;
                        out.push(Op::Add);
                    }
                    AssignOp::MinusEq => {
                        out.push(Op::LoadReg(reg));
                        self.compile_expr(value, out)?;
                        out.push(Op::Sub);
                    }
                    AssignOp::StarEq => {
                        out.push(Op::LoadReg(reg));
                        self.compile_expr(value, out)?;
                        out.push(Op::Mul);
                    }
                    AssignOp::SlashEq => {
                        out.push(Op::LoadReg(reg));
                        self.compile_expr(value, out)?;
                        out.push(Op::Div);
                    }
                }
                out.push(Op::StoreReg(reg));
            }
            Stmt::Expr(e) => {
                self.compile_expr(e, out)?;
                out.push(Op::Pop);
            }
        }
        Ok(())
    }

    // ── Expression compilation ────────────────────────────────────────────

    fn compile_expr(&mut self, expr: &Expr, out: &mut Vec<Op>) -> Result<(), ScriptError> {
        match expr {
            Expr::Num(n) => {
                out.push(Op::PushConst(*n as f32));
            }
            Expr::Const(c) => {
                let v = match c {
                    Const::Pi => std::f32::consts::PI,
                    Const::E => std::f32::consts::E,
                };
                out.push(Op::PushConst(v));
            }
            Expr::Var(name) => {
                let reg = self.reg_of(name);
                out.push(Op::LoadReg(reg));
            }
            Expr::Unary(op, operand) => {
                self.compile_expr(operand, out)?;
                match op {
                    UnOp::Neg => out.push(Op::Neg),
                    UnOp::Not => out.push(Op::Not),
                }
            }
            Expr::Binary(op, left, right) => {
                self.compile_expr(left, out)?;
                self.compile_expr(right, out)?;
                out.push(binop_to_op(op));
            }
            Expr::Call(name, args) => {
                let (builtin, arity) = Builtin::from_name(name)
                    .ok_or_else(|| compiler_err(format!("unknown function: {}", name)))?;
                if args.len() != arity {
                    return Err(compiler_err(format!(
                        "function {} expects {} arg{}, got {}",
                        name,
                        arity,
                        if arity == 1 { "" } else { "s" },
                        args.len()
                    )));
                }
                for arg in args {
                    self.compile_expr(arg, out)?;
                }
                out.push(Op::Call(builtin));
            }
            Expr::If(cond, then, else_) => {
                // Compile condition.
                self.compile_expr(cond, out)?;
                // Emit JumpIfFalse with placeholder; record index for backpatch.
                let jif_idx = out.len();
                out.push(Op::JumpIfFalse(u32::MAX));
                // Compile then-branch.
                self.compile_expr(then, out)?;
                // Emit Jump over else-branch; record index for backpatch.
                let jmp_idx = out.len();
                out.push(Op::Jump(u32::MAX));
                // Backpatch JumpIfFalse to here (start of else).
                let else_target = out.len() as u32;
                out[jif_idx] = Op::JumpIfFalse(else_target);
                // Compile else-branch.
                self.compile_expr(else_, out)?;
                // Backpatch Jump to here (end of if).
                let end_target = out.len() as u32;
                out[jmp_idx] = Op::Jump(end_target);
            }
            Expr::While(cond, body) => {
                // [top] — record loop back-jump target.
                let top = out.len() as u32;
                self.compile_expr(cond, out)?;
                // JumpIfFalse → end (placeholder).
                let jif_idx = out.len();
                out.push(Op::JumpIfFalse(u32::MAX));
                // Compile body.
                self.compile_loop_body(body, out)?;
                // Jump back to top.
                out.push(Op::Jump(top));
                // Backpatch JumpIfFalse to here.
                let end = out.len() as u32;
                out[jif_idx] = Op::JumpIfFalse(end);
            }
            Expr::Loop(count_expr, body) => {
                // Allocate a hidden counter register.
                let counter_reg = self.next_reg;
                self.next_reg += 1;

                // Evaluate count and store in the counter register.
                self.compile_expr(count_expr, out)?;
                out.push(Op::StoreReg(counter_reg));

                // [top] test counter > 0.
                let top = out.len() as u32;
                out.push(Op::LoadReg(counter_reg));
                out.push(Op::PushConst(0.0));
                out.push(Op::Gt);
                // JumpIfFalse → end (placeholder).
                let jif_idx = out.len();
                out.push(Op::JumpIfFalse(u32::MAX));
                // Compile body.
                self.compile_loop_body(body, out)?;
                // Decrement counter.
                out.push(Op::LoadReg(counter_reg));
                out.push(Op::PushConst(1.0));
                out.push(Op::Sub);
                out.push(Op::StoreReg(counter_reg));
                // Jump back to top.
                out.push(Op::Jump(top));
                // Backpatch JumpIfFalse to here.
                let end = out.len() as u32;
                out[jif_idx] = Op::JumpIfFalse(end);
            }
        }
        Ok(())
    }

    /// Compile body statements inside `while`/`loop`.
    ///
    /// `Stmt::Expr` gets an explicit `Pop` because the body value is
    /// discarded. `Stmt::Assign` already terminates with `StoreReg` and
    /// leaves nothing on the stack.
    fn compile_loop_body(&mut self, body: &[Stmt], out: &mut Vec<Op>) -> Result<(), ScriptError> {
        for stmt in body {
            match stmt {
                Stmt::Assign { .. } => {
                    self.compile_stmt(stmt, out)?;
                }
                Stmt::Expr(e) => {
                    self.compile_expr(e, out)?;
                    out.push(Op::Pop);
                }
            }
        }
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn binop_to_op(op: &BinOp) -> Op {
    match op {
        BinOp::Add => Op::Add,
        BinOp::Sub => Op::Sub,
        BinOp::Mul => Op::Mul,
        BinOp::Div => Op::Div,
        BinOp::Mod => Op::Mod,
        BinOp::Pow => Op::Pow,
        BinOp::Eq => Op::Eq,
        BinOp::Ne => Op::Ne,
        BinOp::Lt => Op::Lt,
        BinOp::Le => Op::Le,
        BinOp::Gt => Op::Gt,
        BinOp::Ge => Op::Ge,
        BinOp::And => Op::And,
        BinOp::Or => Op::Or,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

/// Compile a parsed [`Ast`] into a [`Program`].
///
/// # Errors
/// Returns [`ScriptError`] if any function name is unknown, an argument count
/// is wrong, or an assignment to the read-only `srate` variable is attempted.
pub fn compile_ast(ast: &Ast) -> Result<Program, ScriptError> {
    let mut compiler = Compiler::new();

    let mut init = Vec::new();
    compiler.compile_stmts(&ast.init, &mut init)?;

    let mut sample = Vec::new();
    compiler.compile_stmts(&ast.sample, &mut sample)?;

    let num_regs = compiler.next_reg as usize;
    Ok(Program {
        init,
        sample,
        num_regs,
        spl0_reg: 0,
        spl1_reg: 1,
        srate_reg: 2,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::compile;

    // ── 1. spl0 = spl0 * 0.5 ────────────────────────────────────────────────

    #[test]
    fn test_basic_gain_opcodes() {
        let prog = compile("spl0=spl0*0.5;").expect("should compile");
        let ops = &prog.sample;
        // Expect: LoadReg(0), PushConst(0.5), Mul, StoreReg(0)
        assert!(
            ops.contains(&Op::LoadReg(0)),
            "expected LoadReg(spl0=0), got {:?}",
            ops
        );
        assert!(
            ops.contains(&Op::PushConst(0.5)),
            "expected PushConst(0.5), got {:?}",
            ops
        );
        assert!(ops.contains(&Op::Mul), "expected Mul, got {:?}", ops);
        assert!(
            ops.contains(&Op::StoreReg(0)),
            "expected StoreReg(spl0=0), got {:?}",
            ops
        );
        assert!(!prog.sample.is_empty());
        assert_eq!(prog.spl0_reg, 0);
    }

    // ── 2. Unknown function ──────────────────────────────────────────────────

    #[test]
    fn test_unknown_function_error() {
        let err = compile("foo(1);").expect_err("should fail on unknown function");
        assert!(
            err.message.contains("unknown function"),
            "message was: {}",
            err.message
        );
    }

    // ── 3. srate is read-only ────────────────────────────────────────────────

    #[test]
    fn test_srate_read_only() {
        let err = compile("srate = 5;").expect_err("should fail on srate assignment");
        assert!(
            err.message.contains("srate"),
            "message was: {}",
            err.message
        );
        assert!(
            err.message.contains("read-only"),
            "message was: {}",
            err.message
        );
    }

    // ── 4. Arity validation ──────────────────────────────────────────────────

    #[test]
    fn test_sin_correct_arity() {
        compile("sin(1);").expect("sin(1) should compile");
    }

    #[test]
    fn test_sin_wrong_arity() {
        let err = compile("sin(1,2);").expect_err("sin(1,2) should fail");
        assert!(
            err.message.contains("sin"),
            "message was: {}",
            err.message
        );
    }

    #[test]
    fn test_atan2_needs_two_args() {
        let err = compile("atan2(1);").expect_err("atan2(1) should fail");
        assert!(
            err.message.contains("atan2"),
            "message was: {}",
            err.message
        );
    }

    // ── 5. if emits JumpIfFalse and Jump ────────────────────────────────────

    #[test]
    fn test_if_emits_jumps() {
        let prog = compile("if(spl0>0, spl0, 0);").expect("should compile");
        let has_jif = prog
            .sample
            .iter()
            .any(|op| matches!(op, Op::JumpIfFalse(_)));
        let has_jmp = prog.sample.iter().any(|op| matches!(op, Op::Jump(_)));
        assert!(has_jif, "expected JumpIfFalse in sample ops");
        assert!(has_jmp, "expected Jump in sample ops");
    }

    // ── 6. num_regs counts all distinct vars ────────────────────────────────

    #[test]
    fn test_num_regs_counts_vars() {
        // spl0=0, spl1=1, srate=2, a=3, b=4 → num_regs >= 5
        let prog = compile("@init a=0; b=0; @sample spl0=spl0*0.5;")
            .expect("should compile");
        assert!(
            prog.num_regs >= 5,
            "expected num_regs >= 5 (spl0,spl1,srate,a,b), got {}",
            prog.num_regs
        );
    }

    // ── 7. Lex/parse errors propagate ───────────────────────────────────────

    #[test]
    fn test_parse_error_propagates() {
        compile("garbage(").expect_err("should fail on bad input");
    }

    // ── 8. While loop back-jump ──────────────────────────────────────────────

    #[test]
    fn test_while_back_jump() {
        let prog = compile("while(spl0>0) ( spl0=spl0-1; );").expect("should compile");
        // There must be a Jump whose target index is less than its own index
        // (a back-jump to the loop top).
        let has_back_jump = prog.sample.iter().enumerate().any(|(i, op)| {
            if let Op::Jump(target) = op {
                (*target as usize) < i
            } else {
                false
            }
        });
        assert!(
            has_back_jump,
            "expected a back-jump (while loop top), ops: {:?}",
            prog.sample
        );
    }

    // ── 9. Loop also produces a back-jump ───────────────────────────────────

    #[test]
    fn test_loop_back_jump() {
        let prog = compile("loop(4) ( spl0 = spl0 + 1; );").expect("should compile");
        let has_back_jump = prog.sample.iter().enumerate().any(|(i, op)| {
            if let Op::Jump(target) = op {
                (*target as usize) < i
            } else {
                false
            }
        });
        assert!(
            has_back_jump,
            "expected a back-jump (loop), ops: {:?}",
            prog.sample
        );
    }

    // ── extra: compound assign ───────────────────────────────────────────────

    #[test]
    fn test_compound_plus_eq() {
        let prog = compile("spl0 += 0.1;").expect("should compile");
        assert!(prog.sample.contains(&Op::LoadReg(0)));
        assert!(prog.sample.contains(&Op::Add));
        assert!(prog.sample.contains(&Op::StoreReg(0)));
    }

    // ── extra: $pi constant ──────────────────────────────────────────────────

    #[test]
    fn test_pi_constant() {
        let prog = compile("x = $pi;").expect("should compile");
        let has_pi = prog.sample.iter().any(|op| {
            if let Op::PushConst(v) = op {
                (*v - std::f32::consts::PI).abs() < 1e-6
            } else {
                false
            }
        });
        assert!(has_pi, "expected PushConst(PI) in sample ops");
    }

    // ── extra: srate can be READ ─────────────────────────────────────────────

    #[test]
    fn test_srate_can_be_read() {
        compile("spl0 = srate;").expect("reading srate should be fine");
    }
}
