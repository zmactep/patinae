//! Lexer/tokenizer for the selection language
//!
//! Converts selection strings into a stream of tokens using nom combinators.

use nom::{
    branch::alt,
    bytes::complete::{escaped_transform, is_not, tag, take_while, take_while1},
    character::complete::{char, digit1, multispace0},
    combinator::{eof, map, opt, recognize, value},
    sequence::{delimited, pair, preceded, tuple},
    IResult,
};

use crate::error::ParseError;

/// Token types for the selection language
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    /// Opening parenthesis
    LParen,
    /// Closing parenthesis
    RParen,
    /// Comma separator
    Comma,
    /// Slash for macro notation
    Slash,
    /// Backtick for resn`resi notation
    Backtick,
    /// Colon for ranges
    Colon,
    /// Plus for lists or OR
    Plus,
    /// Minus for ranges or AND NOT
    Minus,
    /// Asterisk for wildcard or all
    Asterisk,
    /// Question mark for single char wildcard
    Question,
    /// Percent for selection reference
    Percent,
    /// Ampersand for AND
    Ampersand,
    /// Pipe for OR
    Pipe,
    /// Exclamation for NOT
    Exclamation,
    /// Equals sign
    Equals,
    /// Not equals (!=)
    NotEquals,
    /// Less than
    LessThan,
    /// Less than or equal
    LessOrEqual,
    /// Greater than
    GreaterThan,
    /// Greater than or equal
    GreaterOrEqual,
    /// Integer literal
    Integer(i32),
    /// Float literal
    Float(f32),
    /// Identifier (keyword or atom name)
    Ident(String),
    /// Quoted string
    QuotedString(String),
    /// The "of" keyword (used in within/beyond X of selection)
    Of,
    /// End of input
    Eof,
}

impl Token {
    /// Check if this token is an operator
    #[allow(dead_code)]
    pub fn is_operator(&self) -> bool {
        matches!(
            self,
            Token::Plus | Token::Minus | Token::Ampersand | Token::Pipe | Token::Exclamation
        )
    }

    /// Check if this token is a comparison operator
    #[allow(dead_code)]
    pub fn is_comparison(&self) -> bool {
        matches!(
            self,
            Token::Equals
                | Token::NotEquals
                | Token::LessThan
                | Token::LessOrEqual
                | Token::GreaterThan
                | Token::GreaterOrEqual
        )
    }
}

/// Result of lexing
pub type LexResult<'a, T> = IResult<&'a str, T>;

/// Parse whitespace (including multispace)
fn ws(input: &str) -> LexResult<'_, ()> {
    value((), multispace0)(input)
}

/// Parse opening parenthesis
fn lparen(input: &str) -> LexResult<'_, Token> {
    value(Token::LParen, char('('))(input)
}

/// Parse closing parenthesis
fn rparen(input: &str) -> LexResult<'_, Token> {
    value(Token::RParen, char(')'))(input)
}

/// Parse comma
fn comma(input: &str) -> LexResult<'_, Token> {
    value(Token::Comma, char(','))(input)
}

/// Parse slash
fn slash(input: &str) -> LexResult<'_, Token> {
    value(Token::Slash, char('/'))(input)
}

/// Parse backtick
fn backtick(input: &str) -> LexResult<'_, Token> {
    value(Token::Backtick, char('`'))(input)
}

/// Parse colon
fn colon(input: &str) -> LexResult<'_, Token> {
    value(Token::Colon, char(':'))(input)
}

/// Parse plus
fn plus(input: &str) -> LexResult<'_, Token> {
    value(Token::Plus, char('+'))(input)
}

/// Parse minus (but not part of a number)
fn minus(input: &str) -> LexResult<'_, Token> {
    value(Token::Minus, char('-'))(input)
}

/// Parse asterisk
fn asterisk(input: &str) -> LexResult<'_, Token> {
    value(Token::Asterisk, char('*'))(input)
}

/// Parse question mark
fn question(input: &str) -> LexResult<'_, Token> {
    value(Token::Question, char('?'))(input)
}

/// Parse percent
fn percent(input: &str) -> LexResult<'_, Token> {
    value(Token::Percent, char('%'))(input)
}

/// Parse ampersand
fn ampersand(input: &str) -> LexResult<'_, Token> {
    value(Token::Ampersand, char('&'))(input)
}

/// Parse pipe
fn pipe(input: &str) -> LexResult<'_, Token> {
    value(Token::Pipe, char('|'))(input)
}

/// Parse exclamation
fn exclamation(input: &str) -> LexResult<'_, Token> {
    value(Token::Exclamation, char('!'))(input)
}

/// Parse comparison operators
fn comparison(input: &str) -> LexResult<'_, Token> {
    alt((
        value(Token::NotEquals, tag("!=")),
        value(Token::LessOrEqual, tag("<=")),
        value(Token::GreaterOrEqual, tag(">=")),
        value(Token::Equals, alt((tag("=="), tag("=")))),
        value(Token::LessThan, char('<')),
        value(Token::GreaterThan, char('>')),
    ))(input)
}

/// Parse a number (integer or float) - does NOT consume leading minus
fn number(input: &str) -> LexResult<'_, Token> {
    let original_input = input;
    let (input, int_part) = digit1(input)?;
    let (input, frac_part) = opt(preceded(char('.'), digit1))(input)?;

    match frac_part {
        Some(frac) => {
            let s = format!("{}.{}", int_part, frac);
            let f: f32 = s.parse().map_err(|_| {
                nom::Err::Error(nom::error::Error::new(
                    original_input,
                    nom::error::ErrorKind::Float,
                ))
            })?;
            Ok((input, Token::Float(f)))
        }
        None => {
            let i: i32 = int_part.parse().map_err(|_| {
                nom::Err::Error(nom::error::Error::new(
                    original_input,
                    nom::error::ErrorKind::Digit,
                ))
            })?;
            Ok((input, Token::Integer(i)))
        }
    }
}

/// Check if a character can start an identifier
fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

/// Check if a character can be part of an identifier
fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.' || c == '\''
}

/// Parse an identifier
fn ident(input: &str) -> LexResult<'_, Token> {
    let (input, s) =
        recognize(pair(take_while1(is_ident_start), take_while(is_ident_char)))(input)?;

    // Check for special "of" keyword
    if s.eq_ignore_ascii_case("of") {
        return Ok((input, Token::Of));
    }

    Ok((input, Token::Ident(s.to_string())))
}

/// Parse an identifier that starts with a digit (like atom names "1H", "2HG")
fn digit_ident(input: &str) -> LexResult<'_, Token> {
    // Match digit(s) followed by at least one letter, then any alphanumeric/quote chars.
    // This handles PDB codes like "1ubq", "1t46", "1hpx" as well as atom names like "1H".
    let (input, s) = recognize(tuple((
        digit1,
        take_while1(|c: char| c.is_alphabetic() || c == '\''),
        take_while(|c: char| c.is_alphanumeric() || c == '_' || c == '\''),
    )))(input)?;
    Ok((input, Token::Ident(s.to_string())))
}

/// Parse a quoted string
fn quoted_string(input: &str) -> LexResult<'_, Token> {
    alt((
        map(
            delimited(char('"'), double_quoted_content, char('"')),
            Token::QuotedString,
        ),
        map(
            delimited(char('\''), single_quoted_content, char('\'')),
            Token::QuotedString,
        ),
    ))(input)
}

fn double_quoted_content(input: &str) -> LexResult<'_, String> {
    map(
        opt(escaped_transform(
            is_not("\\\""),
            '\\',
            alt((
                value("\\", tag("\\")),
                value("\"", tag("\"")),
                value("'", tag("'")),
                value("\n", tag("n")),
                value("\r", tag("r")),
                value("\t", tag("t")),
            )),
        )),
        Option::unwrap_or_default,
    )(input)
}

fn single_quoted_content(input: &str) -> LexResult<'_, String> {
    map(
        opt(escaped_transform(
            is_not("\\'"),
            '\\',
            alt((
                value("\\", tag("\\")),
                value("\"", tag("\"")),
                value("'", tag("'")),
                value("\n", tag("n")),
                value("\r", tag("r")),
                value("\t", tag("t")),
            )),
        )),
        Option::unwrap_or_default,
    )(input)
}

/// Parse end of input
fn eof_token(input: &str) -> LexResult<'_, Token> {
    value(Token::Eof, eof)(input)
}

/// Parse a single token
fn token(input: &str) -> LexResult<'_, Token> {
    preceded(
        ws,
        alt((
            // Multi-char operators first
            comparison,
            // Single-char tokens
            lparen,
            rparen,
            comma,
            slash,
            backtick,
            colon,
            plus,
            minus,
            asterisk,
            question,
            percent,
            ampersand,
            pipe,
            exclamation,
            // Quoted strings
            quoted_string,
            // Identifiers starting with digit (like 1H) - must come before number
            digit_ident,
            // Numbers (does not include leading minus)
            number,
            // Regular identifiers
            ident,
            // EOF
            eof_token,
        )),
    )(input)
}

/// Tokenize an entire selection string
pub fn tokenize(input: &str) -> Result<Vec<Token>, ParseError> {
    let mut tokens = Vec::new();
    let mut remaining = input;

    loop {
        // Skip whitespace
        let (rest, _) = ws(remaining).map_err(|_| ParseError::UnexpectedEof)?;
        remaining = rest;

        if remaining.is_empty() {
            tokens.push(Token::Eof);
            break;
        }

        match token(remaining) {
            Ok((rest, tok)) => {
                if matches!(tok, Token::Eof) {
                    tokens.push(tok);
                    break;
                }
                tokens.push(tok);
                remaining = rest;
            }
            Err(_) => {
                return Err(ParseError::UnexpectedToken(
                    remaining.chars().take(10).collect(),
                ));
            }
        }
    }

    Ok(tokens)
}

/// A token stream for parsing
#[derive(Debug, Clone)]
pub struct TokenStream {
    tokens: Vec<Token>,
    pos: usize,
}

impl TokenStream {
    /// Create a new token stream from a list of tokens
    pub fn new(tokens: Vec<Token>) -> Self {
        TokenStream { tokens, pos: 0 }
    }

    /// Create a token stream from a selection string
    pub fn from_str(input: &str) -> Result<Self, ParseError> {
        Ok(TokenStream::new(tokenize(input)?))
    }

    /// Peek at the current token without consuming it
    pub fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    /// Peek at the nth token ahead
    #[allow(dead_code)]
    pub fn peek_n(&self, n: usize) -> Option<&Token> {
        self.tokens.get(self.pos + n)
    }

    /// Consume and return the current token
    pub fn next(&mut self) -> Option<Token> {
        if self.pos < self.tokens.len() {
            let tok = self.tokens[self.pos].clone();
            self.pos += 1;
            Some(tok)
        } else {
            None
        }
    }

    /// Check if we're at the end
    pub fn is_eof(&self) -> bool {
        matches!(self.peek(), Some(Token::Eof) | None)
    }

    /// Expect a specific token
    #[allow(dead_code)]
    pub fn expect(&mut self, expected: &Token) -> Result<Token, ParseError> {
        match self.next() {
            Some(tok) if &tok == expected => Ok(tok),
            Some(tok) => Err(ParseError::Expected {
                expected: format!("{:?}", expected),
                found: format!("{:?}", tok),
            }),
            None => Err(ParseError::UnexpectedEof),
        }
    }

    /// Get the current position
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Set the position (for backtracking)
    pub fn set_position(&mut self, pos: usize) {
        self.pos = pos;
    }

    /// Save position for potential backtracking
    #[allow(dead_code)]
    pub fn save(&self) -> usize {
        self.pos
    }

    /// Restore position (backtrack)
    #[allow(dead_code)]
    pub fn restore(&mut self, pos: usize) {
        self.pos = pos;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize_simple() {
        let tokens = tokenize("all").unwrap();
        assert_eq!(tokens, vec![Token::Ident("all".to_string()), Token::Eof]);
    }

    #[test]
    fn test_tokenize_name() {
        let tokens = tokenize("name CA").unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Ident("name".to_string()),
                Token::Ident("CA".to_string()),
                Token::Eof
            ]
        );
    }

    #[test]
    fn test_tokenize_and() {
        let tokens = tokenize("name CA and chain A").unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Ident("name".to_string()),
                Token::Ident("CA".to_string()),
                Token::Ident("and".to_string()),
                Token::Ident("chain".to_string()),
                Token::Ident("A".to_string()),
                Token::Eof
            ]
        );
    }

    #[test]
    fn test_tokenize_parens() {
        let tokens = tokenize("(name CA)").unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::LParen,
                Token::Ident("name".to_string()),
                Token::Ident("CA".to_string()),
                Token::RParen,
                Token::Eof
            ]
        );
    }

    #[test]
    fn test_tokenize_comparison() {
        let tokens = tokenize("b > 50").unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Ident("b".to_string()),
                Token::GreaterThan,
                Token::Integer(50),
                Token::Eof
            ]
        );
    }

    #[test]
    fn test_tokenize_float() {
        let tokens = tokenize("b > 50.5").unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Ident("b".to_string()),
                Token::GreaterThan,
                Token::Float(50.5),
                Token::Eof
            ]
        );
    }

    #[test]
    fn test_tokenize_range() {
        let tokens = tokenize("resi 100-200").unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Ident("resi".to_string()),
                Token::Integer(100),
                Token::Minus,
                Token::Integer(200),
                Token::Eof
            ]
        );
    }

    #[test]
    fn test_tokenize_list() {
        let tokens = tokenize("name CA+CB+CD").unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Ident("name".to_string()),
                Token::Ident("CA".to_string()),
                Token::Plus,
                Token::Ident("CB".to_string()),
                Token::Plus,
                Token::Ident("CD".to_string()),
                Token::Eof
            ]
        );
    }

    #[test]
    fn test_tokenize_within() {
        let tokens = tokenize("within 5 of name CA").unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Ident("within".to_string()),
                Token::Integer(5),
                Token::Of,
                Token::Ident("name".to_string()),
                Token::Ident("CA".to_string()),
                Token::Eof
            ]
        );
    }

    #[test]
    fn test_tokenize_macro() {
        let tokens = tokenize("/protein//A/100/CA").unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Slash,
                Token::Ident("protein".to_string()),
                Token::Slash,
                Token::Slash,
                Token::Ident("A".to_string()),
                Token::Slash,
                Token::Integer(100),
                Token::Slash,
                Token::Ident("CA".to_string()),
                Token::Eof
            ]
        );
    }

    #[test]
    fn test_tokenize_quoted() {
        let tokens = tokenize("name \"C A\"").unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Ident("name".to_string()),
                Token::QuotedString("C A".to_string()),
                Token::Eof
            ]
        );
    }

    #[test]
    fn test_tokenize_empty_quoted() {
        let tokens = tokenize("chain \"\"").unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Ident("chain".to_string()),
                Token::QuotedString(String::new()),
                Token::Eof
            ]
        );
    }

    #[test]
    fn test_tokenize_quoted_with_escapes() {
        let tokens = tokenize(r#"chain "A\"B\\C""#).unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Ident("chain".to_string()),
                Token::QuotedString("A\"B\\C".to_string()),
                Token::Eof
            ]
        );
    }

    #[test]
    fn test_tokenize_digit_ident() {
        let tokens = tokenize("name 1H").unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Ident("name".to_string()),
                Token::Ident("1H".to_string()),
                Token::Eof
            ]
        );
    }

    #[test]
    fn test_tokenize_rejects_integer_overflow() {
        let overflow = (i64::from(i32::MAX) + 1).to_string();

        assert!(tokenize(&overflow).is_err());
    }

    #[test]
    fn test_token_stream() {
        let mut stream = TokenStream::from_str("name CA").unwrap();
        assert_eq!(stream.next(), Some(Token::Ident("name".to_string())));
        assert_eq!(stream.next(), Some(Token::Ident("CA".to_string())));
        assert_eq!(stream.next(), Some(Token::Eof));
    }

    #[test]
    fn test_token_stream_peek() {
        let stream = TokenStream::from_str("name CA").unwrap();
        assert_eq!(stream.peek(), Some(&Token::Ident("name".to_string())));
        assert_eq!(stream.peek_n(1), Some(&Token::Ident("CA".to_string())));
    }
}
