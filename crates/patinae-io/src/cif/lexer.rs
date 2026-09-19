//! CIF/STAR format lexer
//!
//! Hand-written byte scanner for tokenizing CIF format files.
//! Operates on `&[u8]` with first-byte dispatch for O(1) token type
//! resolution. All token variants borrow from the input — zero heap
//! allocations.

/// CIF token types — all variants borrow from the input string
#[derive(Debug, Clone, PartialEq)]
pub enum Token<'a> {
    /// data_ block header (the name after `data_`)
    DataBlock(&'a str),
    /// loop_ keyword
    Loop,
    /// _category.item data name (includes leading `_`)
    DataName(&'a str),
    /// Unquoted value
    Value(&'a str),
    /// Single-quoted string (inner content, no quotes)
    SingleQuoted(&'a str),
    /// Double-quoted string (inner content, no quotes)
    DoubleQuoted(&'a str),
    /// Semicolon-delimited text block (content between delimiters)
    TextField(&'a str),
    /// Missing value (.)
    Missing,
    /// Unknown value (?)
    Unknown,
    /// End of input
    Eof,
}

impl<'a> Token<'a> {
    /// Extract string value — zero-copy borrow for all string variants
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Token::Value(s)
            | Token::SingleQuoted(s)
            | Token::DoubleQuoted(s)
            | Token::TextField(s)
            | Token::DataBlock(s)
            | Token::DataName(s) => Some(s),
            _ => None,
        }
    }

    /// Check if this token represents a missing or unknown value
    #[allow(dead_code)]
    pub fn is_missing(&self) -> bool {
        matches!(self, Token::Missing | Token::Unknown)
    }
}

#[inline(always)]
fn is_whitespace(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}

/// Tokenize a CIF file using a hand-written byte scanner.
///
/// All returned tokens borrow from `input` — no heap allocations.
pub fn tokenize(input: &str) -> Vec<Token<'_>> {
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut pos = 0;
    let mut tokens = Vec::with_capacity(len / 5);
    let mut at_line_start = true;

    loop {
        // Skip whitespace and comments
        while pos < len {
            match bytes[pos] {
                b' ' | b'\t' | b'\r' => {
                    pos += 1;
                    at_line_start = false;
                }
                b'\n' => {
                    pos += 1;
                    at_line_start = true;
                }
                b'#' => {
                    pos += 1;
                    while pos < len && bytes[pos] != b'\n' {
                        pos += 1;
                    }
                }
                _ => break,
            }
        }

        if pos >= len {
            tokens.push(Token::Eof);
            break;
        }

        let start = pos;

        match bytes[pos] {
            // Single-quoted string
            b'\'' => {
                pos += 1;
                let content_start = pos;
                while pos < len && bytes[pos] != b'\'' {
                    pos += 1;
                }
                let content = &input[content_start..pos];
                if pos < len {
                    pos += 1; // skip closing quote
                }
                tokens.push(Token::SingleQuoted(content));
            }

            // Double-quoted string
            b'"' => {
                pos += 1;
                let content_start = pos;
                while pos < len && bytes[pos] != b'"' {
                    pos += 1;
                }
                let content = &input[content_start..pos];
                if pos < len {
                    pos += 1; // skip closing quote
                }
                tokens.push(Token::DoubleQuoted(content));
            }

            // Semicolon text field (only at start of line)
            b';' if at_line_start => {
                pos += 1;
                let content_start = pos;
                let mut found = false;
                while pos < len {
                    if bytes[pos] == b'\n' && pos + 1 < len && bytes[pos + 1] == b';' {
                        tokens.push(Token::TextField(&input[content_start..pos]));
                        pos += 2; // skip \n;
                        found = true;
                        break;
                    }
                    pos += 1;
                }
                if !found {
                    tokens.push(Token::TextField(&input[content_start..len]));
                    pos = len;
                }
            }

            // Data name (_category.item)
            b'_' => {
                while pos < len && !is_whitespace(bytes[pos]) {
                    pos += 1;
                }
                tokens.push(Token::DataName(&input[start..pos]));
            }

            // data_ keyword
            b'd' if pos + 5 <= len && &bytes[pos..pos + 5] == b"data_" => {
                pos += 5;
                let name_start = pos;
                while pos < len && !is_whitespace(bytes[pos]) {
                    pos += 1;
                }
                tokens.push(Token::DataBlock(&input[name_start..pos]));
            }

            // loop_ keyword
            b'l' if pos + 5 <= len
                && &bytes[pos..pos + 5] == b"loop_"
                && (pos + 5 >= len || is_whitespace(bytes[pos + 5])) =>
            {
                pos += 5;
                tokens.push(Token::Loop);
            }

            // Unquoted value (including . and ? special cases)
            _ => {
                while pos < len && !is_whitespace(bytes[pos]) && bytes[pos] != b'#' {
                    pos += 1;
                }
                let value = &input[start..pos];
                if value.len() == 1 {
                    match bytes[start] {
                        b'.' => {
                            tokens.push(Token::Missing);
                            at_line_start = false;
                            continue;
                        }
                        b'?' => {
                            tokens.push(Token::Unknown);
                            at_line_start = false;
                            continue;
                        }
                        _ => {}
                    }
                }
                tokens.push(Token::Value(value));
            }
        }

        at_line_start = false;
    }

    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quoted() {
        let tokens = tokenize("'hello world' \"foo bar\"");
        assert_eq!(tokens[0], Token::SingleQuoted("hello world"));
        assert_eq!(tokens[1], Token::DoubleQuoted("foo bar"));
    }

    #[test]
    fn test_special_values() {
        let tokens = tokenize(". ? 42");
        assert_eq!(tokens[0], Token::Missing);
        assert_eq!(tokens[1], Token::Unknown);
        assert_eq!(tokens[2], Token::Value("42"));
    }

    #[test]
    fn test_text_field() {
        let tokens = tokenize(";\nsome text\n;");
        assert_eq!(tokens[0], Token::TextField("\nsome text"));
        assert_eq!(tokens[1], Token::Eof);
    }

    #[test]
    fn test_comments() {
        let tokens = tokenize("value1 # comment\nvalue2");
        assert_eq!(tokens[0], Token::Value("value1"));
        assert_eq!(tokens[1], Token::Value("value2"));
    }

    #[test]
    fn test_tokenize_full() {
        let cif = "data_TEST\n_cell.length_a 10.0\nloop_\n_atom_site.id\n1\n2\n";
        let tokens = tokenize(cif);

        assert_eq!(tokens[0], Token::DataBlock("TEST"));
        assert_eq!(tokens[1], Token::DataName("_cell.length_a"));
        assert_eq!(tokens[2], Token::Value("10.0"));
        assert_eq!(tokens[3], Token::Loop);
        assert_eq!(tokens[4], Token::DataName("_atom_site.id"));
        assert_eq!(tokens[5], Token::Value("1"));
        assert_eq!(tokens[6], Token::Value("2"));
        assert_eq!(tokens[7], Token::Eof);
    }

    #[test]
    fn test_loop_not_prefix() {
        // "loop_extra" should be an unquoted value, not Loop + "extra"
        let tokens = tokenize("loop_extra");
        assert_eq!(tokens[0], Token::Value("loop_extra"));
        assert_eq!(tokens[1], Token::Eof);
    }

    #[test]
    fn test_semicolon_not_at_line_start() {
        // ; in the middle of a line is an unquoted value, not a text field
        let tokens = tokenize("value ;stuff");
        assert_eq!(tokens[0], Token::Value("value"));
        assert_eq!(tokens[1], Token::Value(";stuff"));
    }
}
