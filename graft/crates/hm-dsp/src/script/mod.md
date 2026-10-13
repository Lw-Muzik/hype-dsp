# crates/hm-dsp/src/script/mod.rs

- lexer · module · L1-L1 — pub mod lexer;
- parser · module · L4-L4 — pub mod parser;
- compiler · module · L9-L9 — pub mod compiler;
- vm · module · L12-L12 — pub mod vm;
- ScriptError · struct · L16-L20 — pub struct ScriptError
- fmt · function · L23-L25 — fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result
- compile · function · L31-L35 — pub fn compile(src: &str) -> Result<Program, ScriptError>
