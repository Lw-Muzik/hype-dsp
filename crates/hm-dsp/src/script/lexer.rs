use super::ScriptError;

#[derive(Debug, Clone, PartialEq)]
pub struct Spanned<T> {
    pub node: T,
    pub line: u32,
    pub col: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConstKind {
    Pi,
    E,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OpTok {
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    Assign,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    EqEq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    AndAnd,
    OrOr,
    Not,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Num(f64),
    Ident(String),
    Const(ConstKind),
    Op(OpTok),
    LParen,
    RParen,
    Comma,
    Semicolon,
    SectionInit,
    SectionSample,
    SectionBlock,
}

pub fn lex(src: &str) -> Result<Vec<Spanned<Token>>, ScriptError> {
    let chars: Vec<char> = src.chars().collect();
    let mut tokens = Vec::new();
    let mut pos = 0usize;
    let mut line = 1u32;
    let mut col = 1u32;

    while pos < chars.len() {
        let c = chars[pos];

        // Skip whitespace
        if c == ' ' || c == '\t' || c == '\r' {
            pos += 1;
            col += 1;
            continue;
        }

        if c == '\n' {
            pos += 1;
            line += 1;
            col = 1;
            continue;
        }

        // Line comments
        if c == '/' && pos + 1 < chars.len() && chars[pos + 1] == '/' {
            // skip to end of line
            while pos < chars.len() && chars[pos] != '\n' {
                pos += 1;
                col += 1;
            }
            continue;
        }

        let tok_line = line;
        let tok_col = col;

        // Section markers: @init, @sample, @block
        if c == '@' {
            let start = pos;
            pos += 1;
            col += 1;
            while pos < chars.len() && (chars[pos].is_alphanumeric() || chars[pos] == '_') {
                pos += 1;
                col += 1;
            }
            let word: String = chars[start..pos].iter().collect();
            let tok = match word.as_str() {
                "@init" => Token::SectionInit,
                "@sample" => Token::SectionSample,
                "@block" => Token::SectionBlock,
                _ => {
                    return Err(ScriptError {
                        line: tok_line,
                        col: tok_col,
                        message: format!("unknown section marker '{}'", word),
                    })
                }
            };
            tokens.push(Spanned { node: tok, line: tok_line, col: tok_col });
            continue;
        }

        // $pi, $e constants
        if c == '$' {
            let start = pos;
            pos += 1;
            col += 1;
            while pos < chars.len() && (chars[pos].is_alphanumeric() || chars[pos] == '_') {
                pos += 1;
                col += 1;
            }
            let word: String = chars[start..pos].iter().collect();
            let tok = match word.to_lowercase().as_str() {
                "$pi" => Token::Const(ConstKind::Pi),
                "$e" => Token::Const(ConstKind::E),
                _ => {
                    return Err(ScriptError {
                        line: tok_line,
                        col: tok_col,
                        message: format!("unknown constant '{}'", word),
                    })
                }
            };
            tokens.push(Spanned { node: tok, line: tok_line, col: tok_col });
            continue;
        }

        // Numbers: integer, float, scientific
        // Also handles leading dot like .5
        if c.is_ascii_digit()
            || (c == '.' && pos + 1 < chars.len() && chars[pos + 1].is_ascii_digit())
        {
            let start = pos;
            while pos < chars.len() && chars[pos].is_ascii_digit() {
                pos += 1;
                col += 1;
            }
            if pos < chars.len() && chars[pos] == '.' {
                pos += 1;
                col += 1;
                while pos < chars.len() && chars[pos].is_ascii_digit() {
                    pos += 1;
                    col += 1;
                }
            }
            // scientific notation
            if pos < chars.len() && (chars[pos] == 'e' || chars[pos] == 'E') {
                pos += 1;
                col += 1;
                if pos < chars.len() && (chars[pos] == '+' || chars[pos] == '-') {
                    pos += 1;
                    col += 1;
                }
                while pos < chars.len() && chars[pos].is_ascii_digit() {
                    pos += 1;
                    col += 1;
                }
            }
            let num_str: String = chars[start..pos].iter().collect();
            let val: f64 = num_str.parse().map_err(|_| ScriptError {
                line: tok_line,
                col: tok_col,
                message: format!("invalid number '{}'", num_str),
            })?;
            tokens.push(Spanned { node: Token::Num(val), line: tok_line, col: tok_col });
            continue;
        }

        // Identifiers
        if c.is_alphabetic() || c == '_' {
            let start = pos;
            while pos < chars.len() && (chars[pos].is_alphanumeric() || chars[pos] == '_') {
                pos += 1;
                col += 1;
            }
            let ident: String = chars[start..pos].iter().collect::<String>().to_lowercase();
            tokens.push(Spanned { node: Token::Ident(ident), line: tok_line, col: tok_col });
            continue;
        }

        // Multi-char operators (longest match first)
        let remaining: String = chars[pos..].iter().take(2).collect();
        if remaining == "<=" {
            tokens.push(Spanned { node: Token::Op(OpTok::Le), line: tok_line, col: tok_col });
            pos += 2;
            col += 2;
            continue;
        }
        if remaining == ">=" {
            tokens.push(Spanned { node: Token::Op(OpTok::Ge), line: tok_line, col: tok_col });
            pos += 2;
            col += 2;
            continue;
        }
        if remaining == "==" {
            tokens.push(Spanned { node: Token::Op(OpTok::EqEq), line: tok_line, col: tok_col });
            pos += 2;
            col += 2;
            continue;
        }
        if remaining == "!=" {
            tokens.push(Spanned { node: Token::Op(OpTok::NotEq), line: tok_line, col: tok_col });
            pos += 2;
            col += 2;
            continue;
        }
        if remaining == "&&" {
            tokens.push(Spanned { node: Token::Op(OpTok::AndAnd), line: tok_line, col: tok_col });
            pos += 2;
            col += 2;
            continue;
        }
        if remaining == "||" {
            tokens.push(Spanned { node: Token::Op(OpTok::OrOr), line: tok_line, col: tok_col });
            pos += 2;
            col += 2;
            continue;
        }
        if remaining == "+=" {
            tokens.push(Spanned { node: Token::Op(OpTok::PlusEq), line: tok_line, col: tok_col });
            pos += 2;
            col += 2;
            continue;
        }
        if remaining == "-=" {
            tokens.push(Spanned {
                node: Token::Op(OpTok::MinusEq),
                line: tok_line,
                col: tok_col,
            });
            pos += 2;
            col += 2;
            continue;
        }
        if remaining == "*=" {
            tokens.push(Spanned {
                node: Token::Op(OpTok::StarEq),
                line: tok_line,
                col: tok_col,
            });
            pos += 2;
            col += 2;
            continue;
        }
        if remaining == "/=" {
            tokens.push(Spanned {
                node: Token::Op(OpTok::SlashEq),
                line: tok_line,
                col: tok_col,
            });
            pos += 2;
            col += 2;
            continue;
        }

        // Single-char tokens
        let tok = match c {
            '+' => Token::Op(OpTok::Plus),
            '-' => Token::Op(OpTok::Minus),
            '*' => Token::Op(OpTok::Star),
            '/' => Token::Op(OpTok::Slash),
            '%' => Token::Op(OpTok::Percent),
            '^' => Token::Op(OpTok::Caret),
            '=' => Token::Op(OpTok::Assign),
            '<' => Token::Op(OpTok::Lt),
            '>' => Token::Op(OpTok::Gt),
            '!' => Token::Op(OpTok::Not),
            '(' => Token::LParen,
            ')' => Token::RParen,
            ',' => Token::Comma,
            ';' => Token::Semicolon,
            _ => {
                return Err(ScriptError {
                    line: tok_line,
                    col: tok_col,
                    message: format!("unexpected character '{}'", c),
                })
            }
        };
        tokens.push(Spanned { node: tok, line: tok_line, col: tok_col });
        pos += 1;
        col += 1;
    }

    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nodes(src: &str) -> Vec<Token> {
        lex(src).unwrap().into_iter().map(|s| s.node).collect()
    }

    #[test]
    fn test_basic_expression() {
        let toks = nodes("spl0 = spl0 * 0.5;");
        assert_eq!(
            toks,
            vec![
                Token::Ident("spl0".to_string()),
                Token::Op(OpTok::Assign),
                Token::Ident("spl0".to_string()),
                Token::Op(OpTok::Star),
                Token::Num(0.5),
                Token::Semicolon,
            ]
        );
    }

    #[test]
    fn test_section_markers() {
        let toks = nodes("@init\n@sample");
        assert_eq!(toks, vec![Token::SectionInit, Token::SectionSample]);
    }

    #[test]
    fn test_floats() {
        assert_eq!(nodes("0.5"), vec![Token::Num(0.5)]);
        assert_eq!(nodes("1.0"), vec![Token::Num(1.0)]);
        assert_eq!(nodes("42"), vec![Token::Num(42.0)]);
        assert_eq!(nodes(".5"), vec![Token::Num(0.5)]);
        assert_eq!(nodes("1e3"), vec![Token::Num(1000.0)]);
    }

    #[test]
    fn test_multi_char_ops() {
        let toks = nodes("a <= b && c == d");
        assert_eq!(
            toks,
            vec![
                Token::Ident("a".to_string()),
                Token::Op(OpTok::Le),
                Token::Ident("b".to_string()),
                Token::Op(OpTok::AndAnd),
                Token::Ident("c".to_string()),
                Token::Op(OpTok::EqEq),
                Token::Ident("d".to_string()),
            ]
        );
    }

    #[test]
    fn test_comment_and_newline_tracking() {
        let src = "a\n// this is a comment\nb";
        let spanned = lex(src).unwrap();
        assert_eq!(spanned.len(), 2);
        assert_eq!(spanned[0].node, Token::Ident("a".to_string()));
        assert_eq!(spanned[0].line, 1);
        assert_eq!(spanned[0].col, 1);
        assert_eq!(spanned[1].node, Token::Ident("b".to_string()));
        assert_eq!(spanned[1].line, 3);
        assert_eq!(spanned[1].col, 1);
    }

    #[test]
    fn test_illegal_char() {
        let err = lex("a # b").unwrap_err();
        assert_eq!(err.line, 1);
        assert_eq!(err.col, 3);
    }

    #[test]
    fn test_const_pi() {
        let toks = nodes("$pi");
        assert_eq!(toks, vec![Token::Const(ConstKind::Pi)]);
    }

    #[test]
    fn test_const_e() {
        let toks = nodes("$e");
        assert_eq!(toks, vec![Token::Const(ConstKind::E)]);
    }

    #[test]
    fn test_section_block() {
        let toks = nodes("@block");
        assert_eq!(toks, vec![Token::SectionBlock]);
    }

    #[test]
    fn test_assign_ops() {
        // tokens: x(0) +=(1) 1(2) ;(3) y(4) -=(5) 2(6) ;(7) z(8) *=(9) 3(10) ;(11) w(12) /=(13) 4(14) ;(15)
        let toks = nodes("x += 1; y -= 2; z *= 3; w /= 4;");
        use Token::Op;
        assert_eq!(toks[1], Op(OpTok::PlusEq));
        assert_eq!(toks[5], Op(OpTok::MinusEq));
        assert_eq!(toks[9], Op(OpTok::StarEq));
        assert_eq!(toks[13], Op(OpTok::SlashEq));
    }

    #[test]
    fn test_case_insensitive_ident() {
        let toks = nodes("SPL0");
        assert_eq!(toks, vec![Token::Ident("spl0".to_string())]);
    }
}
