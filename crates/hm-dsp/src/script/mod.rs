pub mod lexer;
pub use lexer::lex;

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
