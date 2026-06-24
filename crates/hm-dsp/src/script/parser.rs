//! EEL2-subset parser for LiveProg scripts.
//!
//! Turns the flat token stream produced by [`super::lexer`] into an [`Ast`]
//! split into `@init` / `@sample` sections. Expressions are parsed with a
//! precedence-climbing recursive-descent strategy that mirrors EEL2 semantics:
//! `^` (power) is right-associative, everything else is left-associative, and
//! the control forms (`if`, `while`, `loop`) plus function calls are dispatched
//! at the primary level by identifier name.
//!
//! Function-name validity, variable existence and arity are intentionally NOT
//! checked here — that is the compiler's job (Task 4). The parser only enforces
//! grammar.

use super::lexer::{ConstKind, OpTok, Spanned, Token};
use super::ScriptError;

/// A fully parsed script, split into its executable sections.
///
/// `@block` statements are tokenised by the lexer but deliberately dropped here
/// in v1 (block-rate processing is not yet implemented), so they never reach
/// the [`Ast`].
#[derive(Debug, Clone, PartialEq)]
pub struct Ast {
    pub init: Vec<Stmt>,
    pub sample: Vec<Stmt>,
}

/// The four compound-assignment operators plus plain `=`.
#[derive(Debug, Clone, PartialEq)]
pub enum AssignOp {
    Eq,      // =
    PlusEq,  // +=
    MinusEq, // -=
    StarEq,  // *=
    SlashEq, // /=
}

/// A single statement: either a (possibly compound) assignment to a named
/// variable, or a bare expression evaluated for its side effects / value.
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Assign {
        name: String,
        op: AssignOp,
        value: Expr,
    },
    Expr(Expr),
}

/// Built-in mathematical constants accessed via `$pi` / `$e`.
#[derive(Debug, Clone, PartialEq)]
pub enum Const {
    Pi,
    E,
}

/// Prefix unary operators.
#[derive(Debug, Clone, PartialEq)]
pub enum UnOp {
    Neg, // -x
    Not, // !x
}

/// Binary operators, ordered loosely by the precedence family they belong to.
#[derive(Debug, Clone, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

/// An expression tree. Control forms (`if`/`while`/`loop`) are modelled as
/// expressions because in EEL2 everything yields a value.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Num(f64),
    Const(Const),
    Var(String),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// Function call: name + args. Name/arity validated by the compiler.
    Call(String, Vec<Expr>),
    /// `if(cond, then, else)`.
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    /// `while(cond) ( body... )`.
    While(Box<Expr>, Vec<Stmt>),
    /// `loop(n) ( body... )` or `loop(n, body_expr)`.
    Loop(Box<Expr>, Vec<Stmt>),
}

/// Parse a token stream into an [`Ast`].
///
/// Statements before any section marker default to `@sample`. `@init` and
/// `@sample` markers route subsequent statements into their respective
/// sections; `@block` statements are parsed-then-discarded (v1 limitation).
pub fn parse(tokens: &[Spanned<Token>]) -> Result<Ast, ScriptError> {
    let mut parser = Parser::new(tokens);
    parser.parse_program()
}

/// Which section the cursor is currently filling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Init,
    Sample,
    Block,
}

struct Parser<'a> {
    tokens: &'a [Spanned<Token>],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(tokens: &'a [Spanned<Token>]) -> Self {
        Parser { tokens, pos: 0 }
    }

    // ---- cursor helpers -------------------------------------------------

    fn peek(&self) -> Option<&Spanned<Token>> {
        self.tokens.get(self.pos)
    }

    /// Peek the token `n` positions ahead of the cursor (0 == current).
    fn peek_at(&self, n: usize) -> Option<&Spanned<Token>> {
        self.tokens.get(self.pos + n)
    }

    /// Consume the current token, asserting it equals `tok`.
    fn expect(&mut self, tok: &Token) -> Result<(), ScriptError> {
        match self.peek() {
            Some(s) if &s.node == tok => {
                self.pos += 1;
                Ok(())
            }
            Some(s) => Err(ScriptError {
                line: s.line,
                col: s.col,
                message: format!("expected {:?}, found {:?}", tok, s.node),
            }),
            None => Err(self.eof_err(&format!("expected {:?}, found end of input", tok))),
        }
    }

    /// Build an error anchored at the current token (or EOF position).
    fn err(&self, msg: &str) -> ScriptError {
        match self.peek() {
            Some(s) => ScriptError {
                line: s.line,
                col: s.col,
                message: msg.to_string(),
            },
            None => self.eof_err(msg),
        }
    }

    /// Build an error anchored just past the last token (for EOF cases).
    fn eof_err(&self, msg: &str) -> ScriptError {
        match self.tokens.last() {
            Some(s) => ScriptError {
                line: s.line,
                col: s.col,
                message: msg.to_string(),
            },
            None => ScriptError {
                line: 1,
                col: 1,
                message: msg.to_string(),
            },
        }
    }

    // ---- top level ------------------------------------------------------

    fn parse_program(&mut self) -> Result<Ast, ScriptError> {
        let mut init = Vec::new();
        let mut sample = Vec::new();
        // Code before any explicit section marker belongs to @sample.
        let mut section = Section::Sample;

        loop {
            match self.peek().map(|s| &s.node) {
                None => break,
                Some(Token::SectionInit) => {
                    self.pos += 1;
                    section = Section::Init;
                }
                Some(Token::SectionSample) => {
                    self.pos += 1;
                    section = Section::Sample;
                }
                Some(Token::SectionBlock) => {
                    self.pos += 1;
                    section = Section::Block;
                }
                Some(Token::Semicolon) => {
                    // Stray empty statement between sections / statements.
                    self.pos += 1;
                }
                Some(_) => {
                    let stmt = self.parse_stmt()?;
                    match section {
                        Section::Init => init.push(stmt),
                        Section::Sample => sample.push(stmt),
                        // @block parsed for grammar validity but discarded in v1.
                        Section::Block => {}
                    }
                    // A statement is terminated by `;`, a section marker, or EOF.
                    self.consume_optional_semicolon();
                }
            }
        }

        Ok(Ast { init, sample })
    }

    /// Eat a single trailing `;` if present (statement terminator).
    fn consume_optional_semicolon(&mut self) {
        if matches!(self.peek().map(|s| &s.node), Some(Token::Semicolon)) {
            self.pos += 1;
        }
    }

    // ---- statements -----------------------------------------------------

    fn parse_stmt(&mut self) -> Result<Stmt, ScriptError> {
        // Assignment: Ident followed by one of = += -= *= /=.
        if let Some(Token::Ident(name)) = self.peek().map(|s| &s.node) {
            if let Some(op) = self.peek_at(1).and_then(|s| assign_op(&s.node)) {
                let name = name.clone();
                self.pos += 2; // consume ident + assign op
                let value = self.parse_expr()?;
                return Ok(Stmt::Assign { name, op, value });
            }
        }
        Ok(Stmt::Expr(self.parse_expr()?))
    }

    /// Parse a parenthesised statement block: `( stmt ; stmt ; ... )`.
    /// Assumes the opening `(` is the current token.
    fn parse_block(&mut self) -> Result<Vec<Stmt>, ScriptError> {
        self.expect(&Token::LParen)?;
        let mut stmts = Vec::new();
        loop {
            match self.peek().map(|s| &s.node) {
                Some(Token::RParen) => {
                    self.pos += 1;
                    break;
                }
                Some(Token::Semicolon) => {
                    // Empty / trailing statement separator.
                    self.pos += 1;
                }
                None => return Err(self.eof_err("unclosed block: expected ')'")),
                Some(_) => {
                    stmts.push(self.parse_stmt()?);
                    // Each statement is separated by `;` or closed by `)`.
                    match self.peek().map(|s| &s.node) {
                        Some(Token::Semicolon) => self.pos += 1,
                        Some(Token::RParen) => {} // loop will close on next iter
                        Some(_) => return Err(self.err("expected ';' or ')' in block")),
                        None => return Err(self.eof_err("unclosed block: expected ')'")),
                    }
                }
            }
        }
        Ok(stmts)
    }

    // ---- expressions (precedence climbing) ------------------------------

    fn parse_expr(&mut self) -> Result<Expr, ScriptError> {
        self.parse_or()
    }

    // 1. ||  (left-assoc, lowest precedence)
    fn parse_or(&mut self) -> Result<Expr, ScriptError> {
        let mut left = self.parse_and()?;
        while matches!(self.peek().map(|s| &s.node), Some(Token::Op(OpTok::OrOr))) {
            self.pos += 1;
            let right = self.parse_and()?;
            left = Expr::Binary(BinOp::Or, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    // 2. && (left-assoc)
    fn parse_and(&mut self) -> Result<Expr, ScriptError> {
        let mut left = self.parse_cmp()?;
        while matches!(self.peek().map(|s| &s.node), Some(Token::Op(OpTok::AndAnd))) {
            self.pos += 1;
            let right = self.parse_cmp()?;
            left = Expr::Binary(BinOp::And, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    // 3. == != < <= > >= (left-assoc)
    fn parse_cmp(&mut self) -> Result<Expr, ScriptError> {
        let mut left = self.parse_add()?;
        while let Some(op) = self.peek().and_then(|s| cmp_op(&s.node)) {
            self.pos += 1;
            let right = self.parse_add()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    // 4. + - (left-assoc)
    fn parse_add(&mut self) -> Result<Expr, ScriptError> {
        let mut left = self.parse_mul()?;
        loop {
            let op = match self.peek().map(|s| &s.node) {
                Some(Token::Op(OpTok::Plus)) => BinOp::Add,
                Some(Token::Op(OpTok::Minus)) => BinOp::Sub,
                _ => break,
            };
            self.pos += 1;
            let right = self.parse_mul()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    // 5. * / % (left-assoc)
    fn parse_mul(&mut self) -> Result<Expr, ScriptError> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.peek().map(|s| &s.node) {
                Some(Token::Op(OpTok::Star)) => BinOp::Mul,
                Some(Token::Op(OpTok::Slash)) => BinOp::Div,
                Some(Token::Op(OpTok::Percent)) => BinOp::Mod,
                _ => break,
            };
            self.pos += 1;
            let right = self.parse_unary()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    // 7. unary: - ! (prefix, right-assoc) — sits above `^` so that `-2^2`
    //    parses as `-(2^2)` (matching EEL2 / most calculators) because the
    //    unary parser recurses into `parse_pow` for its operand.
    fn parse_unary(&mut self) -> Result<Expr, ScriptError> {
        match self.peek().map(|s| &s.node) {
            Some(Token::Op(OpTok::Minus)) => {
                self.pos += 1;
                let operand = self.parse_unary()?;
                Ok(Expr::Unary(UnOp::Neg, Box::new(operand)))
            }
            Some(Token::Op(OpTok::Not)) => {
                self.pos += 1;
                let operand = self.parse_unary()?;
                Ok(Expr::Unary(UnOp::Not, Box::new(operand)))
            }
            _ => self.parse_pow(),
        }
    }

    // 6. ^ (RIGHT-assoc, EEL2 power). Right-recursion gives `2^3^2` ==
    //    `2^(3^2)`. The right operand goes back through `parse_unary` so that
    //    `2^-3` is accepted.
    fn parse_pow(&mut self) -> Result<Expr, ScriptError> {
        let base = self.parse_primary()?;
        if matches!(self.peek().map(|s| &s.node), Some(Token::Op(OpTok::Caret))) {
            self.pos += 1;
            let exp = self.parse_unary()?;
            Ok(Expr::Binary(BinOp::Pow, Box::new(base), Box::new(exp)))
        } else {
            Ok(base)
        }
    }

    // 8. primary
    fn parse_primary(&mut self) -> Result<Expr, ScriptError> {
        let s = self.peek().ok_or_else(|| self.eof_err("expected expression"))?;
        match &s.node {
            Token::Num(n) => {
                let n = *n;
                self.pos += 1;
                Ok(Expr::Num(n))
            }
            Token::Const(ConstKind::Pi) => {
                self.pos += 1;
                Ok(Expr::Const(Const::Pi))
            }
            Token::Const(ConstKind::E) => {
                self.pos += 1;
                Ok(Expr::Const(Const::E))
            }
            Token::LParen => {
                self.pos += 1;
                let inner = self.parse_expr()?;
                self.expect(&Token::RParen)?;
                Ok(inner)
            }
            Token::Ident(name) => {
                let name = name.clone();
                // A `(` immediately after the ident makes it a call or a
                // control form; otherwise it is a plain variable reference.
                if matches!(self.peek_at(1).map(|s| &s.node), Some(Token::LParen)) {
                    self.parse_call_or_form(name)
                } else {
                    self.pos += 1;
                    Ok(Expr::Var(name))
                }
            }
            other => Err(ScriptError {
                line: s.line,
                col: s.col,
                message: format!("unexpected token in expression: {:?}", other),
            }),
        }
    }

    /// Parse `name(...)` where `name` may be a control form (`if`/`while`/
    /// `loop`) or an ordinary function call. The cursor is on the ident.
    fn parse_call_or_form(&mut self, name: String) -> Result<Expr, ScriptError> {
        match name.as_str() {
            "if" => self.parse_if(),
            "while" => self.parse_while(),
            "loop" => self.parse_loop(),
            _ => {
                self.pos += 1; // consume ident
                let args = self.parse_arg_list()?;
                Ok(Expr::Call(name, args))
            }
        }
    }

    /// Parse a comma-separated, parenthesised argument list. Cursor on `(`.
    /// Permits an empty list `()`.
    fn parse_arg_list(&mut self) -> Result<Vec<Expr>, ScriptError> {
        self.expect(&Token::LParen)?;
        let mut args = Vec::new();
        if matches!(self.peek().map(|s| &s.node), Some(Token::RParen)) {
            self.pos += 1;
            return Ok(args);
        }
        loop {
            args.push(self.parse_expr()?);
            match self.peek().map(|s| &s.node) {
                Some(Token::Comma) => self.pos += 1,
                Some(Token::RParen) => {
                    self.pos += 1;
                    break;
                }
                Some(_) => return Err(self.err("expected ',' or ')' in argument list")),
                None => return Err(self.eof_err("unclosed argument list: expected ')'")),
            }
        }
        Ok(args)
    }

    /// `if(cond, then, else)` — exactly three arguments.
    fn parse_if(&mut self) -> Result<Expr, ScriptError> {
        self.pos += 1; // consume `if`
        let args = self.parse_arg_list()?;
        if args.len() != 3 {
            return Err(self.err(&format!(
                "if(...) expects exactly 3 arguments, got {}",
                args.len()
            )));
        }
        let mut it = args.into_iter();
        let cond = it.next().unwrap();
        let then = it.next().unwrap();
        let els = it.next().unwrap();
        Ok(Expr::If(Box::new(cond), Box::new(then), Box::new(els)))
    }

    /// `while(cond) ( body... )` — condition group then a statement block.
    fn parse_while(&mut self) -> Result<Expr, ScriptError> {
        self.pos += 1; // consume `while`
        let cond = self.parse_paren_expr()?;
        let body = self.parse_block()?;
        Ok(Expr::While(Box::new(cond), body))
    }

    /// `loop(n) ( body... )` or the inline `loop(n, body_expr)` form.
    fn parse_loop(&mut self) -> Result<Expr, ScriptError> {
        self.pos += 1; // consume `loop`
        // Distinguish `loop(n) ( ... )` from `loop(n, body_expr)` by parsing
        // the first `(` group as a comma list.
        let head = self.parse_arg_list()?;
        match head.len() {
            1 => {
                // Count followed by a separate body block.
                let count = head.into_iter().next().unwrap();
                let body = self.parse_block()?;
                Ok(Expr::Loop(Box::new(count), body))
            }
            2 => {
                // Inline form: the second arg is the (single-statement) body.
                let mut it = head.into_iter();
                let count = it.next().unwrap();
                let body_expr = it.next().unwrap();
                Ok(Expr::Loop(Box::new(count), vec![Stmt::Expr(body_expr)]))
            }
            n => Err(self.err(&format!(
                "loop(...) expects 1 (count, then body block) or 2 (count, body) arguments, got {}",
                n
            ))),
        }
    }

    /// Parse a single parenthesised expression: `( expr )`. Cursor on `(`.
    fn parse_paren_expr(&mut self) -> Result<Expr, ScriptError> {
        self.expect(&Token::LParen)?;
        let inner = self.parse_expr()?;
        self.expect(&Token::RParen)?;
        Ok(inner)
    }
}

/// Map an assignment-operator token to its [`AssignOp`], or `None`.
fn assign_op(tok: &Token) -> Option<AssignOp> {
    match tok {
        Token::Op(OpTok::Assign) => Some(AssignOp::Eq),
        Token::Op(OpTok::PlusEq) => Some(AssignOp::PlusEq),
        Token::Op(OpTok::MinusEq) => Some(AssignOp::MinusEq),
        Token::Op(OpTok::StarEq) => Some(AssignOp::StarEq),
        Token::Op(OpTok::SlashEq) => Some(AssignOp::SlashEq),
        _ => None,
    }
}

/// Map a comparison-operator token to its [`BinOp`], or `None`.
fn cmp_op(tok: &Token) -> Option<BinOp> {
    match tok {
        Token::Op(OpTok::EqEq) => Some(BinOp::Eq),
        Token::Op(OpTok::NotEq) => Some(BinOp::Ne),
        Token::Op(OpTok::Lt) => Some(BinOp::Lt),
        Token::Op(OpTok::Le) => Some(BinOp::Le),
        Token::Op(OpTok::Gt) => Some(BinOp::Gt),
        Token::Op(OpTok::Ge) => Some(BinOp::Ge),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::lex;

    /// Lex + parse, returning the AST or panicking with the lex/parse error.
    fn ast(src: &str) -> Ast {
        let toks = lex(src).expect("lexing should succeed");
        parse(&toks).expect("parsing should succeed")
    }

    /// Lex + parse, returning the parse error (asserting lex succeeds).
    fn parse_err(src: &str) -> ScriptError {
        let toks = lex(src).expect("lexing should succeed");
        parse(&toks).expect_err("parsing should fail")
    }

    fn num(n: f64) -> Expr {
        Expr::Num(n)
    }
    fn var(name: &str) -> Expr {
        Expr::Var(name.to_string())
    }
    fn bin(op: BinOp, l: Expr, r: Expr) -> Expr {
        Expr::Binary(op, Box::new(l), Box::new(r))
    }

    #[test]
    fn test_precedence_mul_over_add() {
        // 1 + 2 * 3  ==  1 + (2 * 3)
        let a = ast("1+2*3;");
        assert_eq!(a.sample.len(), 1);
        let expected = bin(BinOp::Add, num(1.0), bin(BinOp::Mul, num(2.0), num(3.0)));
        assert_eq!(a.sample[0], Stmt::Expr(expected));
    }

    #[test]
    fn test_right_assoc_pow() {
        // 2 ^ 3 ^ 2  ==  2 ^ (3 ^ 2)
        let a = ast("2^3^2;");
        let expected = bin(BinOp::Pow, num(2.0), bin(BinOp::Pow, num(3.0), num(2.0)));
        assert_eq!(a.sample[0], Stmt::Expr(expected));
    }

    #[test]
    fn test_assign_stmt() {
        let a = ast("spl0 = spl0 * 0.5;");
        assert_eq!(a.init.len(), 0);
        assert_eq!(a.sample.len(), 1);
        assert_eq!(
            a.sample[0],
            Stmt::Assign {
                name: "spl0".to_string(),
                op: AssignOp::Eq,
                value: bin(BinOp::Mul, var("spl0"), num(0.5)),
            }
        );
    }

    #[test]
    fn test_if_expr() {
        // if(spl0 > 0, spl0, -spl0)
        let a = ast("if(spl0>0, spl0, -spl0);");
        let expected = Expr::If(
            Box::new(bin(BinOp::Gt, var("spl0"), num(0.0))),
            Box::new(var("spl0")),
            Box::new(Expr::Unary(UnOp::Neg, Box::new(var("spl0")))),
        );
        assert_eq!(a.sample[0], Stmt::Expr(expected));
    }

    #[test]
    fn test_bare_script_defaults_to_sample() {
        let a = ast("spl0 = spl0 * 0.5;");
        assert!(a.init.is_empty());
        assert_eq!(a.sample.len(), 1);
        assert_eq!(
            a.sample[0],
            Stmt::Assign {
                name: "spl0".to_string(),
                op: AssignOp::Eq,
                value: bin(BinOp::Mul, var("spl0"), num(0.5)),
            }
        );
    }

    #[test]
    fn test_sections() {
        let a = ast("@init n = 0; @sample n = n + 1;");
        assert_eq!(a.init.len(), 1);
        assert_eq!(a.sample.len(), 1);
        assert_eq!(
            a.init[0],
            Stmt::Assign {
                name: "n".to_string(),
                op: AssignOp::Eq,
                value: num(0.0),
            }
        );
        assert_eq!(
            a.sample[0],
            Stmt::Assign {
                name: "n".to_string(),
                op: AssignOp::Eq,
                value: bin(BinOp::Add, var("n"), num(1.0)),
            }
        );
    }

    #[test]
    fn test_missing_paren() {
        // if(spl0>0, spl0   — missing closing ')'
        let err = parse_err("if(spl0>0, spl0");
        // Error should carry position info (non-zero line/col).
        assert!(err.line >= 1);
        assert!(!err.message.is_empty());
    }

    #[test]
    fn test_logical_precedence() {
        // a && b || c  ==  (a && b) || c   (|| lower than &&)
        let a = ast("a && b || c;");
        let expected = bin(
            BinOp::Or,
            bin(BinOp::And, var("a"), var("b")),
            var("c"),
        );
        assert_eq!(a.sample[0], Stmt::Expr(expected));
    }

    #[test]
    fn test_while() {
        // while(spl0 > 0) ( spl0 = spl0 - 1; )
        let a = ast("while(spl0>0) ( spl0 = spl0 - 1; );");
        let expected = Expr::While(
            Box::new(bin(BinOp::Gt, var("spl0"), num(0.0))),
            vec![Stmt::Assign {
                name: "spl0".to_string(),
                op: AssignOp::Eq,
                value: bin(BinOp::Sub, var("spl0"), num(1.0)),
            }],
        );
        assert_eq!(a.sample[0], Stmt::Expr(expected));
    }

    #[test]
    fn test_function_call() {
        let a = ast("sqrt(spl0);");
        let expected = Expr::Call("sqrt".to_string(), vec![var("spl0")]);
        assert_eq!(a.sample[0], Stmt::Expr(expected));
    }

    // ---- additional coverage beyond the required matrix ----------------

    #[test]
    fn test_block_section_ignored() {
        // @block statements must not leak into init or sample.
        let a = ast("@init a = 1; @block b = 2; @sample c = 3;");
        assert_eq!(a.init.len(), 1);
        assert_eq!(a.sample.len(), 1);
    }

    #[test]
    fn test_loop_count_and_block() {
        // loop(4) ( a = a + 1; )
        let a = ast("loop(4) ( a = a + 1; );");
        let expected = Expr::Loop(
            Box::new(num(4.0)),
            vec![Stmt::Assign {
                name: "a".to_string(),
                op: AssignOp::Eq,
                value: bin(BinOp::Add, var("a"), num(1.0)),
            }],
        );
        assert_eq!(a.sample[0], Stmt::Expr(expected));
    }

    #[test]
    fn test_loop_inline_body() {
        // loop(4, a += 1) — EEL2 inline body form.
        let a = ast("loop(4, a + 1);");
        let expected = Expr::Loop(
            Box::new(num(4.0)),
            vec![Stmt::Expr(bin(BinOp::Add, var("a"), num(1.0)))],
        );
        assert_eq!(a.sample[0], Stmt::Expr(expected));
    }

    #[test]
    fn test_compound_assign() {
        let a = ast("gain += 0.1;");
        assert_eq!(
            a.sample[0],
            Stmt::Assign {
                name: "gain".to_string(),
                op: AssignOp::PlusEq,
                value: num(0.1),
            }
        );
    }

    #[test]
    fn test_unary_not_and_neg() {
        let a = ast("!a; -b;");
        assert_eq!(
            a.sample[0],
            Stmt::Expr(Expr::Unary(UnOp::Not, Box::new(var("a"))))
        );
        assert_eq!(
            a.sample[1],
            Stmt::Expr(Expr::Unary(UnOp::Neg, Box::new(var("b"))))
        );
    }

    #[test]
    fn test_parenthesised_overrides_precedence() {
        // (1 + 2) * 3
        let a = ast("(1+2)*3;");
        let expected = bin(BinOp::Mul, bin(BinOp::Add, num(1.0), num(2.0)), num(3.0));
        assert_eq!(a.sample[0], Stmt::Expr(expected));
    }

    #[test]
    fn test_const_pi() {
        let a = ast("x = $pi;");
        assert_eq!(
            a.sample[0],
            Stmt::Assign {
                name: "x".to_string(),
                op: AssignOp::Eq,
                value: Expr::Const(Const::Pi),
            }
        );
    }

    #[test]
    fn test_multi_arg_call() {
        let a = ast("pow(spl0, 2);");
        assert_eq!(
            a.sample[0],
            Stmt::Expr(Expr::Call("pow".to_string(), vec![var("spl0"), num(2.0)]))
        );
    }

    #[test]
    fn test_empty_statement_skipped() {
        let a = ast(";; a = 1 ;;");
        assert_eq!(a.sample.len(), 1);
    }

    #[test]
    fn test_if_wrong_arity() {
        // Two-arg if is rejected.
        let err = parse_err("if(a, b);");
        assert!(err.message.contains("3 arguments"));
    }
}
