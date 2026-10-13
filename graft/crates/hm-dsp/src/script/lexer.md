# crates/hm-dsp/src/script/lexer.rs

- Spanned · struct · L4-L8 — pub struct Spanned<T>
- ConstKind · enum · L11-L14 — pub enum ConstKind
- OpTok · enum · L17-L38 — pub enum OpTok
- Token · enum · L41-L53 — pub enum Token
- lex · function · L55-L301 — pub fn lex(src: &str) -> Result<Vec<Spanned<Token>>, ScriptError>
- tests · module · L304-L413 — mod tests
- nodes · function · L307-L309 — fn nodes(src: &str) -> Vec<Token>
- test_basic_expression · function · L312-L325 — fn test_basic_expression()
- test_section_markers · function · L328-L331 — fn test_section_markers()
- test_floats · function · L334-L340 — fn test_floats()
- test_multi_char_ops · function · L343-L357 — fn test_multi_char_ops()
- test_comment_and_newline_tracking · function · L360-L370 — fn test_comment_and_newline_tracking()
- test_illegal_char · function · L373-L377 — fn test_illegal_char()
- test_const_pi · function · L380-L383 — fn test_const_pi()
- test_const_e · function · L386-L389 — fn test_const_e()
- test_section_block · function · L392-L395 — fn test_section_block()
- test_assign_ops · function · L398-L406 — fn test_assign_ops()
- test_case_insensitive_ident · function · L409-L412 — fn test_case_insensitive_ident()
