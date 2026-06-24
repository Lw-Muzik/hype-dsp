pub mod lexer;
pub use lexer::lex;

pub mod parser;
pub use parser::{
    parse, AssignOp, Ast, BinOp, Const as EelConst, Expr, Stmt, UnOp,
};

pub mod compiler;
pub use compiler::{compile_ast, Builtin, Op, Program};

pub mod vm;
pub use vm::{run_init, run_sample};

#[derive(Debug, Clone, PartialEq)]
pub struct ScriptError {
    pub line: u32,
    pub col: u32,
    pub message: String,
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}:{}] {}", self.line, self.col, self.message)
    }
}

impl std::error::Error for ScriptError {}

/// Convenience: lex → parse → compile a source string into a [`Program`].
pub fn compile(src: &str) -> Result<Program, ScriptError> {
    let tokens = lex(src)?;
    let ast = parse(&tokens)?;
    compile_ast(&ast)
}
