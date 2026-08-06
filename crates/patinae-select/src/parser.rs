//! Parser for the selection language
//!
//! Converts a token stream into an AST using recursive descent parsing
//! with operator precedence handling.

use crate::ast::{CompareOp, MacroSpec, PropertyValue, SelectionExpr};
use crate::error::ParseError;
use crate::keywords::{lookup, Keyword, KeywordType};
use crate::lexer::{Token, TokenStream};
use crate::pattern::{IntItem, IntSpec, Pattern, ResiItem, ResiSpec};

/// Parse a selection string into an AST
pub fn parse_selection(input: &str) -> Result<SelectionExpr, ParseError> {
    let mut stream = TokenStream::from_str(input)?;
    let expr = parse_expr(&mut stream, 0)?;

    // Ensure we consumed all input
    if !stream.is_eof() {
        return Err(ParseError::UnexpectedToken(format!("{:?}", stream.peek())));
    }

    Ok(expr)
}

/// Binary operator recognized during expression parsing.
#[derive(Debug, Clone, Copy)]
enum BinOp {
    And,
    Or,
    In,
    Like,
    AndNot,
}

impl BinOp {
    fn precedence(self) -> u8 {
        match self {
            BinOp::And | BinOp::AndNot => Keyword::And.precedence(),
            BinOp::Or => Keyword::Or.precedence(),
            BinOp::In => Keyword::In.precedence(),
            BinOp::Like => Keyword::Like.precedence(),
        }
    }

    fn build(self, left: SelectionExpr, right: SelectionExpr) -> SelectionExpr {
        match self {
            BinOp::And => SelectionExpr::And(Box::new(left), Box::new(right)),
            BinOp::Or => SelectionExpr::Or(Box::new(left), Box::new(right)),
            BinOp::In => SelectionExpr::In(Box::new(left), Box::new(right)),
            BinOp::Like => SelectionExpr::Like(Box::new(left), Box::new(right)),
            BinOp::AndNot => SelectionExpr::And(
                Box::new(left),
                Box::new(SelectionExpr::Not(Box::new(right))),
            ),
        }
    }
}

/// Parse an expression with minimum precedence
fn parse_expr(stream: &mut TokenStream, min_prec: u8) -> Result<SelectionExpr, ParseError> {
    let mut left = parse_unary(stream)?;

    while let Some(tok) = stream.peek().cloned() {
        // Check for distance-binary infix operators (within, beyond, near_to)
        if let Token::Ident(s) = &tok {
            if let Some(kw @ (Keyword::Within | Keyword::Beyond | Keyword::NearTo)) = lookup(s) {
                let prec = kw.precedence();
                if prec < min_prec {
                    break;
                }
                stream.next(); // consume keyword
                let dist = parse_number(stream)?;
                match stream.peek() {
                    Some(Token::Of) => {
                        stream.next();
                    }
                    _ => {
                        return Err(ParseError::Expected {
                            expected: "of".to_string(),
                            found: format!("{:?}", stream.peek()),
                        });
                    }
                }
                let right = parse_expr(stream, prec + 1)?;
                left = match kw {
                    Keyword::Within => SelectionExpr::Within(dist, Box::new(left), Box::new(right)),
                    Keyword::Beyond => SelectionExpr::Beyond(dist, Box::new(left), Box::new(right)),
                    Keyword::NearTo => SelectionExpr::NearTo(dist, Box::new(left), Box::new(right)),
                    _ => unreachable!(),
                };
                continue;
            }
        }

        let op = match &tok {
            Token::Ident(s) => match lookup(s) {
                Some(Keyword::And) => BinOp::And,
                Some(Keyword::Or) => BinOp::Or,
                Some(Keyword::In) => BinOp::In,
                Some(Keyword::Like) => BinOp::Like,
                _ => break,
            },
            Token::Ampersand => BinOp::And,
            Token::Pipe => BinOp::Or,
            Token::Plus => BinOp::Or,
            Token::Minus => BinOp::AndNot,
            _ => break,
        };

        if op.precedence() < min_prec {
            break;
        }

        stream.next();
        let right = parse_expr(stream, op.precedence() + 1)?;
        left = op.build(left, right);
    }

    Ok(left)
}

/// Parse a unary expression (NOT, prefix operators)
fn parse_unary(stream: &mut TokenStream) -> Result<SelectionExpr, ParseError> {
    let Some(tok) = stream.peek().cloned() else {
        return Err(ParseError::UnexpectedEof);
    };

    match &tok {
        Token::Ident(s) => {
            if let Some(kw) = lookup(s) {
                match kw.keyword_type() {
                    KeywordType::Opr1 => {
                        return parse_prefix_operator(stream, kw);
                    }
                    KeywordType::Op22 => {
                        return parse_distance_binary(stream, kw);
                    }
                    _ => {}
                }
            }
        }
        Token::Exclamation => {
            stream.next();
            let inner = parse_unary(stream)?;
            return Ok(SelectionExpr::Not(Box::new(inner)));
        }
        _ => {}
    }

    parse_primary(stream)
}

/// Parse prefix operators (not, byres, bychain, etc.)
fn parse_prefix_operator(
    stream: &mut TokenStream,
    kw: Keyword,
) -> Result<SelectionExpr, ParseError> {
    stream.next(); // Consume the keyword

    match kw {
        Keyword::Not => {
            let inner = parse_unary(stream)?;
            Ok(SelectionExpr::Not(Box::new(inner)))
        }
        Keyword::ByRes => {
            let inner = parse_unary(stream)?;
            Ok(SelectionExpr::ByRes(Box::new(inner)))
        }
        Keyword::ByChain => {
            let inner = parse_unary(stream)?;
            Ok(SelectionExpr::ByChain(Box::new(inner)))
        }
        Keyword::ByObject => {
            let inner = parse_unary(stream)?;
            Ok(SelectionExpr::ByObject(Box::new(inner)))
        }
        Keyword::ByMolecule => {
            let inner = parse_unary(stream)?;
            Ok(SelectionExpr::ByMolecule(Box::new(inner)))
        }
        Keyword::BySegment => {
            let inner = parse_unary(stream)?;
            Ok(SelectionExpr::BySegment(Box::new(inner)))
        }
        Keyword::ByFragment => {
            let inner = parse_unary(stream)?;
            Ok(SelectionExpr::ByFragment(Box::new(inner)))
        }
        Keyword::ByCAlpha => {
            let inner = parse_unary(stream)?;
            Ok(SelectionExpr::ByCAlpha(Box::new(inner)))
        }
        Keyword::ByRing => {
            let inner = parse_unary(stream)?;
            Ok(SelectionExpr::ByRing(Box::new(inner)))
        }
        Keyword::ByCell => {
            let inner = parse_unary(stream)?;
            Ok(SelectionExpr::ByCell(Box::new(inner)))
        }
        Keyword::Neighbor => {
            let inner = parse_unary(stream)?;
            Ok(SelectionExpr::Neighbor(Box::new(inner)))
        }
        Keyword::BoundTo => {
            let inner = parse_unary(stream)?;
            Ok(SelectionExpr::BoundTo(Box::new(inner)))
        }
        Keyword::First => {
            let inner = parse_unary(stream)?;
            Ok(SelectionExpr::First(Box::new(inner)))
        }
        Keyword::Last => {
            let inner = parse_unary(stream)?;
            Ok(SelectionExpr::Last(Box::new(inner)))
        }
        Keyword::Around => {
            let dist = parse_number(stream)?;
            let inner = parse_unary(stream)?;
            Ok(SelectionExpr::Around(dist, Box::new(inner)))
        }
        Keyword::Expand => {
            let dist = parse_number(stream)?;
            let inner = parse_unary(stream)?;
            Ok(SelectionExpr::Expand(dist, Box::new(inner)))
        }
        Keyword::Extend => {
            let n = parse_int(stream)? as u32;
            let inner = parse_unary(stream)?;
            Ok(SelectionExpr::Extend(n, Box::new(inner)))
        }
        Keyword::Gap => {
            let dist = parse_number(stream)?;
            let inner = parse_unary(stream)?;
            Ok(SelectionExpr::Gap(dist, Box::new(inner)))
        }
        _ => Err(ParseError::UnknownKeyword(format!("{:?}", kw))),
    }
}

/// Parse distance binary operators (within, beyond, near_to)
fn parse_distance_binary(
    stream: &mut TokenStream,
    kw: Keyword,
) -> Result<SelectionExpr, ParseError> {
    stream.next(); // Consume the keyword

    let dist = parse_number(stream)?;

    // Expect "of"
    match stream.peek() {
        Some(Token::Of) => {
            stream.next();
        }
        _ => {
            return Err(ParseError::Expected {
                expected: "of".to_string(),
                found: format!("{:?}", stream.peek()),
            });
        }
    }

    let inner = parse_unary(stream)?;

    match kw {
        Keyword::Within => Ok(SelectionExpr::Within(
            dist,
            Box::new(SelectionExpr::All),
            Box::new(inner),
        )),
        Keyword::Beyond => Ok(SelectionExpr::Beyond(
            dist,
            Box::new(SelectionExpr::All),
            Box::new(inner),
        )),
        Keyword::NearTo => Ok(SelectionExpr::NearTo(
            dist,
            Box::new(SelectionExpr::All),
            Box::new(inner),
        )),
        _ => Err(ParseError::UnknownKeyword(format!("{:?}", kw))),
    }
}

/// Parse a primary expression (atom, parenthesized, property selector)
fn parse_primary(stream: &mut TokenStream) -> Result<SelectionExpr, ParseError> {
    let Some(tok) = stream.peek().cloned() else {
        return Err(ParseError::UnexpectedEof);
    };

    match tok {
        Token::LParen => {
            stream.next();
            let inner = parse_expr(stream, 0)?;
            match stream.next() {
                Some(Token::RParen) => Ok(inner),
                _ => Err(ParseError::UnmatchedParen),
            }
        }
        Token::Slash => parse_macro(stream),
        Token::Percent => {
            stream.next();
            // Selection reference
            match stream.next() {
                Some(Token::Ident(name)) => Ok(SelectionExpr::Selection(name)),
                _ => Err(ParseError::MissingArgument("selection name".to_string())),
            }
        }
        Token::Ident(ref s) => {
            if let Some(kw) = lookup(s) {
                // Save position so we can backtrack if keyword parse fails
                let saved_pos = stream.position();
                let result = match kw.keyword_type() {
                    KeywordType::Sel0 => parse_sel0(stream, kw),
                    KeywordType::Sel1 => parse_sel1(stream, kw),
                    KeywordType::Sel2 => parse_sel2(stream, kw),
                    KeywordType::Sel3 => parse_sel3(stream, kw),
                    KeywordType::Prp1 => parse_prefix_operator(stream, kw),
                    KeywordType::Opr1 => parse_prefix_operator(stream, kw),
                    KeywordType::Op22 => parse_distance_binary(stream, kw),
                    KeywordType::Opr2 => {
                        // Binary operators shouldn't appear here
                        Err(ParseError::UnexpectedToken(s.clone()))
                    }
                };
                // If keyword parse failed, backtrack and treat as selection/object name.
                // This handles cases where an object name shadows a keyword (e.g., "ss").
                // Only check for immediate Slash (not full is_macro_ahead) to avoid
                // misinterpreting e.g. `ss 1+5/CA` as a macro starting from `ss`.
                match result {
                    Ok(expr) => Ok(expr),
                    Err(_) => {
                        stream.set_position(saved_pos);
                        stream.next();
                        if matches!(stream.peek(), Some(Token::Slash)) {
                            stream.set_position(stream.position() - 1);
                            parse_macro(stream)
                        } else {
                            Ok(SelectionExpr::Selection(s.clone()))
                        }
                    }
                }
            } else {
                stream.next();
                try_macro_or_fallback(stream, SelectionExpr::Selection(s.clone()))
            }
        }
        Token::Integer(n) => {
            stream.next();
            try_macro_or_fallback(stream, SelectionExpr::Resi(ResiSpec::single(n)))
        }
        Token::Asterisk => {
            stream.next();
            Ok(SelectionExpr::All)
        }
        _ => Err(ParseError::UnexpectedToken(format!("{:?}", tok))),
    }
}

/// After consuming one token, check if this is the start of macro notation.
///
/// Uses `is_macro_ahead` to scan for a `/` token. If found, backtracks one
/// position and parses the full macro. Otherwise returns the fallback expression.
fn try_macro_or_fallback(
    stream: &mut TokenStream,
    fallback: SelectionExpr,
) -> Result<SelectionExpr, ParseError> {
    if is_macro_ahead(stream) {
        stream.set_position(stream.position() - 1);
        parse_macro(stream)
    } else {
        Ok(fallback)
    }
}

/// Parse zero-argument selections
fn parse_sel0(stream: &mut TokenStream, kw: Keyword) -> Result<SelectionExpr, ParseError> {
    stream.next(); // Consume the keyword

    match kw {
        Keyword::All => Ok(SelectionExpr::All),
        Keyword::None => Ok(SelectionExpr::None),
        Keyword::Hetatm => Ok(SelectionExpr::Hetatm),
        Keyword::Hydrogens => Ok(SelectionExpr::Hydrogens),
        Keyword::Visible => Ok(SelectionExpr::Visible),
        Keyword::Enabled => Ok(SelectionExpr::Enabled),
        Keyword::Bonded => Ok(SelectionExpr::Bonded),
        Keyword::Polymer => Ok(SelectionExpr::Polymer),
        Keyword::PolymerProtein => Ok(SelectionExpr::PolymerProtein),
        Keyword::PolymerNucleic => Ok(SelectionExpr::PolymerNucleic),
        Keyword::Organic => Ok(SelectionExpr::Organic),
        Keyword::Inorganic => Ok(SelectionExpr::Inorganic),
        Keyword::Solvent => Ok(SelectionExpr::Solvent),
        Keyword::Metals => Ok(SelectionExpr::Metals),
        Keyword::Backbone => Ok(SelectionExpr::Backbone),
        Keyword::Sidechain => Ok(SelectionExpr::Sidechain),
        Keyword::Donors => Ok(SelectionExpr::Donors),
        Keyword::Acceptors => Ok(SelectionExpr::Acceptors),
        Keyword::HBondAcceptors => Ok(SelectionExpr::Acceptors),
        Keyword::HBondDonors => Ok(SelectionExpr::Donors),
        Keyword::Delocalized => Ok(SelectionExpr::Delocalized),
        Keyword::Fixed => Ok(SelectionExpr::Fixed),
        Keyword::Restrained => Ok(SelectionExpr::Restrained),
        Keyword::Masked => Ok(SelectionExpr::Masked),
        Keyword::Protected => Ok(SelectionExpr::Protected),
        Keyword::Present => Ok(SelectionExpr::Present),
        Keyword::Guide => Ok(SelectionExpr::Guide),
        Keyword::Origin => Ok(SelectionExpr::Origin),
        Keyword::Center => Ok(SelectionExpr::Center),
        _ => Err(ParseError::UnknownKeyword(format!("{:?}", kw))),
    }
}

/// Parse one-argument property selections
fn parse_sel1(stream: &mut TokenStream, kw: Keyword) -> Result<SelectionExpr, ParseError> {
    stream.next(); // Consume the keyword

    match kw {
        Keyword::Name => {
            let pattern = parse_pattern(stream)?;
            Ok(SelectionExpr::Name(pattern))
        }
        Keyword::Element => {
            let pattern = parse_pattern(stream)?;
            Ok(SelectionExpr::Elem(pattern))
        }
        Keyword::Resi => {
            let spec = parse_resi_spec(stream)?;
            Ok(SelectionExpr::Resi(spec))
        }
        Keyword::Resn => {
            let pattern = parse_pattern(stream)?;
            Ok(SelectionExpr::Resn(pattern))
        }
        Keyword::Chain => {
            let pattern = parse_pattern(stream)?;
            Ok(SelectionExpr::Chain(pattern))
        }
        Keyword::Segi => {
            let pattern = parse_pattern(stream)?;
            Ok(SelectionExpr::Segi(pattern))
        }
        Keyword::Alt => {
            let pattern = parse_pattern(stream)?;
            Ok(SelectionExpr::Alt(pattern))
        }
        Keyword::Flag => {
            let spec = parse_int_spec(stream)?;
            Ok(SelectionExpr::Flag(spec))
        }
        Keyword::TextType => {
            let pattern = parse_pattern(stream)?;
            Ok(SelectionExpr::TextType(pattern))
        }
        Keyword::NumericType => {
            let pattern = parse_pattern(stream)?;
            Ok(SelectionExpr::NumericType(pattern))
        }
        Keyword::Id => {
            let spec = parse_int_spec(stream)?;
            Ok(SelectionExpr::Id(spec))
        }
        Keyword::Index => {
            let spec = parse_int_spec(stream)?;
            Ok(SelectionExpr::Index(spec))
        }
        Keyword::Rank => {
            let spec = parse_int_spec(stream)?;
            Ok(SelectionExpr::Rank(spec))
        }
        Keyword::Model => {
            let pattern = parse_pattern(stream)?;
            Ok(SelectionExpr::Model(pattern))
        }
        Keyword::State => {
            let spec = parse_int_spec(stream)?;
            Ok(SelectionExpr::State(spec))
        }
        Keyword::SecondaryStructure => {
            let pattern = parse_pattern(stream)?;
            Ok(SelectionExpr::SecondaryStructure(pattern))
        }
        Keyword::Rep => {
            let pattern = parse_pattern(stream)?;
            Ok(SelectionExpr::Rep(pattern))
        }
        Keyword::Color => {
            let pattern = parse_pattern(stream)?;
            Ok(SelectionExpr::Color(pattern))
        }
        Keyword::CartoonColor => {
            let pattern = parse_pattern(stream)?;
            Ok(SelectionExpr::CartoonColor(pattern))
        }
        Keyword::RibbonColor => {
            let pattern = parse_pattern(stream)?;
            Ok(SelectionExpr::RibbonColor(pattern))
        }
        Keyword::PepSeq => {
            let pattern = parse_pattern(stream)?;
            Ok(SelectionExpr::PepSeq(pattern))
        }
        Keyword::Custom => {
            let pattern = parse_pattern(stream)?;
            Ok(SelectionExpr::Custom(pattern))
        }
        Keyword::Label => {
            let pattern = parse_pattern(stream)?;
            Ok(SelectionExpr::Label(pattern))
        }
        Keyword::Stereo => {
            let pattern = parse_pattern(stream)?;
            Ok(SelectionExpr::Stereo(pattern))
        }
        Keyword::Selection => match stream.next() {
            Some(Token::Ident(name)) => Ok(SelectionExpr::Selection(name)),
            _ => Err(ParseError::MissingArgument("selection name".to_string())),
        },
        _ => Err(ParseError::UnknownKeyword(format!("{:?}", kw))),
    }
}

/// Parse two-argument numeric comparison selections
fn parse_sel2(stream: &mut TokenStream, kw: Keyword) -> Result<SelectionExpr, ParseError> {
    stream.next(); // Consume the keyword

    let op = parse_compare_op(stream)?;
    let value = parse_number(stream)?;

    match kw {
        Keyword::BFactor => Ok(SelectionExpr::BFactor(op, value)),
        Keyword::Occupancy => Ok(SelectionExpr::Occupancy(op, value)),
        Keyword::PartialCharge => Ok(SelectionExpr::PartialCharge(op, value)),
        Keyword::FormalCharge => Ok(SelectionExpr::FormalCharge(op, value as i32)),
        Keyword::X => Ok(SelectionExpr::X(op, value)),
        Keyword::Y => Ok(SelectionExpr::Y(op, value)),
        Keyword::Z => Ok(SelectionExpr::Z(op, value)),
        _ => Err(ParseError::UnknownKeyword(format!("{:?}", kw))),
    }
}

/// Parse three-argument property access (p.property op value)
fn parse_sel3(stream: &mut TokenStream, _kw: Keyword) -> Result<SelectionExpr, ParseError> {
    stream.next(); // Consume "p."

    // Get the property name
    let prop_name = match stream.next() {
        Some(Token::Ident(name)) => name,
        _ => return Err(ParseError::MissingArgument("property name".to_string())),
    };

    let op = parse_compare_op(stream)?;

    // Get the value
    let value = match stream.next() {
        Some(Token::Integer(n)) => PropertyValue::Int(n),
        Some(Token::Float(f)) => PropertyValue::Float(f),
        Some(Token::Ident(s)) => PropertyValue::String(s),
        Some(Token::QuotedString(s)) => PropertyValue::String(s),
        _ => return Err(ParseError::MissingArgument("property value".to_string())),
    };

    Ok(SelectionExpr::Property(prop_name, op, value))
}

/// Parse a comparison operator
fn parse_compare_op(stream: &mut TokenStream) -> Result<CompareOp, ParseError> {
    match stream.next() {
        Some(Token::Equals) => Ok(CompareOp::Eq),
        Some(Token::NotEquals) => Ok(CompareOp::Ne),
        Some(Token::LessThan) => Ok(CompareOp::Lt),
        Some(Token::LessOrEqual) => Ok(CompareOp::Le),
        Some(Token::GreaterThan) => Ok(CompareOp::Gt),
        Some(Token::GreaterOrEqual) => Ok(CompareOp::Ge),
        other => Err(ParseError::Expected {
            expected: "comparison operator".to_string(),
            found: format!("{:?}", other),
        }),
    }
}

/// Parse a number (int or float as f32)
fn parse_number(stream: &mut TokenStream) -> Result<f32, ParseError> {
    match stream.next() {
        Some(Token::Integer(n)) => Ok(n as f32),
        Some(Token::Float(f)) => Ok(f),
        other => Err(ParseError::Expected {
            expected: "number".to_string(),
            found: format!("{:?}", other),
        }),
    }
}

/// Parse an integer
fn parse_int(stream: &mut TokenStream) -> Result<i32, ParseError> {
    match stream.next() {
        Some(Token::Integer(n)) => Ok(n),
        other => Err(ParseError::Expected {
            expected: "integer".to_string(),
            found: format!("{:?}", other),
        }),
    }
}

/// Parse a pattern (identifier, wildcard, list, or range)
fn parse_pattern(stream: &mut TokenStream) -> Result<Pattern, ParseError> {
    let first = parse_single_pattern(stream)?;

    // Check for list (+ separator) or range (: separator)
    match stream.peek() {
        Some(Token::Plus) => {
            let mut patterns = vec![first];
            while matches!(stream.peek(), Some(Token::Plus)) {
                stream.next(); // Consume +
                patterns.push(parse_single_pattern(stream)?);
            }
            Ok(Pattern::List(patterns))
        }
        Some(Token::Colon) => {
            stream.next(); // Consume :
            let second = parse_single_pattern(stream)?;
            match (first, second) {
                (Pattern::Exact(start), Pattern::Exact(end)) => Ok(Pattern::Range(start, end)),
                _ => Err(ParseError::InvalidPattern(
                    "range requires exact patterns".to_string(),
                )),
            }
        }
        _ => Ok(first),
    }
}

/// Parse a single pattern element
fn parse_single_pattern(stream: &mut TokenStream) -> Result<Pattern, ParseError> {
    match stream.next() {
        Some(Token::Ident(mut s)) => {
            // Check for trailing wildcards
            loop {
                match stream.peek() {
                    Some(Token::Asterisk) => {
                        stream.next();
                        s.push('*');
                    }
                    Some(Token::Question) => {
                        stream.next();
                        s.push('?');
                    }
                    _ => break,
                }
            }
            if s.contains('*') || s.contains('?') {
                Ok(Pattern::Wildcard(s))
            } else {
                Ok(Pattern::Exact(s))
            }
        }
        Some(Token::QuotedString(s)) => Ok(Pattern::Exact(s)),
        Some(Token::Integer(n)) => {
            // Check for wildcard after number
            let mut s = n.to_string();
            loop {
                match stream.peek() {
                    Some(Token::Asterisk) => {
                        stream.next();
                        s.push('*');
                    }
                    Some(Token::Question) => {
                        stream.next();
                        s.push('?');
                    }
                    _ => break,
                }
            }
            if s.contains('*') || s.contains('?') {
                Ok(Pattern::Wildcard(s))
            } else {
                Ok(Pattern::Exact(s))
            }
        }
        Some(Token::Asterisk) => {
            // Could be just "*" or "*" followed by more characters
            let mut s = "*".to_string();
            // Check for identifier following (like *H for ending in H)
            if let Some(Token::Ident(id)) = stream.peek().cloned() {
                stream.next();
                s.push_str(&id);
                // Check for more wildcards
                loop {
                    match stream.peek() {
                        Some(Token::Asterisk) => {
                            stream.next();
                            s.push('*');
                        }
                        Some(Token::Question) => {
                            stream.next();
                            s.push('?');
                        }
                        _ => break,
                    }
                }
            }
            Ok(Pattern::Wildcard(s))
        }
        other => Err(ParseError::Expected {
            expected: "pattern".to_string(),
            found: format!("{:?}", other),
        }),
    }
}

/// Parse an integer specification (for index, id, etc.)
fn parse_int_spec(stream: &mut TokenStream) -> Result<IntSpec, ParseError> {
    let mut items = vec![parse_int_item(stream)?];

    // Check for list (+ separator)
    while matches!(stream.peek(), Some(Token::Plus)) {
        stream.next(); // Consume +
        items.push(parse_int_item(stream)?);
    }

    Ok(IntSpec { items })
}

/// Parse a single integer item (value or range)
fn parse_int_item(stream: &mut TokenStream) -> Result<IntItem, ParseError> {
    let first = parse_int(stream)?;

    // Check for range (- or :)
    match stream.peek() {
        Some(Token::Minus) | Some(Token::Colon) => {
            stream.next(); // Consume - or :
            let second = parse_int(stream)?;
            Ok(IntItem::Range(first, second))
        }
        _ => Ok(IntItem::Single(first)),
    }
}

/// Parse a residue specification
fn parse_resi_spec(stream: &mut TokenStream) -> Result<ResiSpec, ParseError> {
    let mut items = vec![parse_resi_item(stream)?];

    // Check for list (+ separator)
    while matches!(stream.peek(), Some(Token::Plus)) {
        stream.next(); // Consume +
        items.push(parse_resi_item(stream)?);
    }

    Ok(ResiSpec { items })
}

/// Parse a single residue item
fn parse_resi_item(stream: &mut TokenStream) -> Result<ResiItem, ParseError> {
    // First token could be an integer or identifier (for insertion codes)
    let first_tok = stream.next();
    let (first_val, first_ins) = match first_tok {
        Some(Token::Integer(n)) => (n, None),
        Some(Token::Ident(s)) => {
            // Try to parse as number with insertion code (e.g., "100A")
            parse_resi_with_inscode(&s)?
        }
        other => {
            return Err(ParseError::Expected {
                expected: "residue number".to_string(),
                found: format!("{:?}", other),
            });
        }
    };

    // Check for insertion code after integer
    let first_ins = first_ins.or_else(|| try_consume_inscode(stream));

    // Check for range (- or :)
    match stream.peek() {
        Some(Token::Minus) | Some(Token::Colon) => {
            stream.next(); // Consume - or :

            // Parse second part
            let second_tok = stream.next();
            let (second_val, second_ins) = match second_tok {
                Some(Token::Integer(n)) => (n, None),
                Some(Token::Ident(s)) => parse_resi_with_inscode(&s)?,
                other => {
                    return Err(ParseError::Expected {
                        expected: "residue number".to_string(),
                        found: format!("{:?}", other),
                    });
                }
            };

            // Check for insertion code after second integer
            let second_ins = second_ins.or_else(|| try_consume_inscode(stream));

            match (first_ins, second_ins) {
                (Some(fi), Some(si)) => Ok(ResiItem::InsCodeRange(first_val, fi, second_val, si)),
                (None, None) => Ok(ResiItem::Range(first_val, second_val)),
                _ => {
                    // Mixed - just use range
                    Ok(ResiItem::Range(first_val, second_val))
                }
            }
        }
        _ => {
            // Single value
            match first_ins {
                Some(c) => Ok(ResiItem::InsCode(first_val, c)),
                None => Ok(ResiItem::Single(first_val)),
            }
        }
    }
}

/// Try to consume a single-letter insertion code from the stream.
///
/// Peeks at the next token; if it's a single alphabetic `Ident`, consumes and
/// returns the character. Otherwise returns `None` without advancing.
fn try_consume_inscode(stream: &mut TokenStream) -> Option<char> {
    if let Some(Token::Ident(s)) = stream.peek() {
        if s.len() == 1 {
            if let Some(c) = s.chars().next() {
                if c.is_alphabetic() {
                    stream.next();
                    return Some(c);
                }
            }
        }
    }
    None
}

/// Parse a residue number with optional insertion code (e.g., "100A")
fn parse_resi_with_inscode(s: &str) -> Result<(i32, Option<char>), ParseError> {
    let mut chars = s.chars().peekable();
    let mut num_part = String::new();
    if chars.peek() == Some(&'-') {
        num_part.push('-');
        chars.next();
    }
    while chars.peek().is_some_and(char::is_ascii_digit) {
        num_part.push(chars.next().expect("peeked digit must be present"));
    }

    let remainder: Vec<char> = chars.collect();
    let ins_code = match remainder.as_slice() {
        [] => None,
        [code] if code.is_alphabetic() || *code == ' ' => Some(*code),
        _ => return Err(ParseError::InvalidResidue(s.to_string())),
    };

    let num: i32 = num_part
        .parse()
        .map_err(|_| ParseError::InvalidResidue(s.to_string()))?;

    Ok((num, ins_code))
}

/// Look ahead (without consuming) to check if a `/` appears before any non-macro token.
/// This detects patterns like `1/CA`, `1+5/CA`, `100-200/CA` as macro notation
/// rather than `resi 1 or 5/CA`.
fn is_macro_ahead(stream: &mut TokenStream) -> bool {
    let saved = stream.position();
    let result = loop {
        match stream.peek() {
            Some(Token::Slash) => break true,
            Some(
                Token::Integer(_)
                | Token::Plus
                | Token::Minus
                | Token::Colon
                | Token::Ident(_)
                | Token::Asterisk
                | Token::Question
                | Token::Backtick,
            ) => {
                stream.next();
            }
            _ => break false,
        }
    };
    stream.set_position(saved);
    result
}

/// Parse slash macro notation
///
/// Supports two forms of slash macro notation:
/// - Leading `/`: fields assigned left-to-right: `/object/segi/chain/resn_or_resi/name`
/// - No leading `/`: fields assigned right-to-left: e.g. `A/100/CA` → chain/resi/name
///
/// Field 3 (resn_or_resi): digit-start → resi, otherwise → resn, backtick → resn`resi
/// Field 4 (name): backtick → name`alt
fn parse_macro(stream: &mut TokenStream) -> Result<SelectionExpr, ParseError> {
    let mut spec = MacroSpec::default();

    // Check and consume leading slash
    let leading_slash = matches!(stream.peek(), Some(Token::Slash));
    if leading_slash {
        stream.next();
    }

    // Collect slash-separated components
    let mut components = Vec::new();
    let mut current = MacroComponent::default();

    loop {
        let peeked = stream.peek().cloned();
        match peeked {
            Some(Token::Slash) => {
                stream.next();
                components.push(current);
                current = MacroComponent::default();
            }
            Some(Token::Ident(s)) => {
                stream.next();
                current.push_unquoted(&s);
            }
            Some(Token::Integer(n)) => {
                stream.next();
                current.push_unquoted(&n.to_string());
            }
            Some(Token::QuotedString(s)) => {
                stream.next();
                current.push_quoted(&s);
            }
            Some(Token::Backtick) => {
                stream.next();
                current.push_backtick();
            }
            Some(Token::Asterisk) => {
                stream.next();
                current.push_unquoted("*");
            }
            Some(Token::Question) => {
                stream.next();
                current.push_unquoted("?");
            }
            Some(Token::Minus) => {
                stream.next();
                current.push_unquoted("-");
            }
            Some(Token::Plus) => {
                stream.next();
                current.push_unquoted("+");
            }
            Some(Token::Colon) => {
                stream.next();
                current.push_unquoted(":");
            }
            Some(Token::Of) => {
                stream.next();
                current.push_unquoted("of");
            }
            _ => {
                // End of macro — push final component
                if current.is_present() {
                    components.push(current);
                }
                break;
            }
        }
    }

    // Assign components to spec fields: [object, segi, chain, resn_or_resi, name]
    // Leading `/`: left-to-right (field 0, 1, 2, ...)
    // No leading `/`: right-to-left (last = field 4, second-to-last = field 3, ...)
    let num_components = components.len();
    let offset = if leading_slash {
        0
    } else {
        5usize.saturating_sub(num_components)
    };

    for (i, component) in components.into_iter().enumerate() {
        let field = offset + i;

        match field {
            0 => spec.model = component.into_single_pattern(),
            1 => spec.segi = component.into_single_pattern(),
            2 => spec.chain = component.into_single_pattern(),
            3 => assign_resn_resi(&mut spec, component)?,
            4 => assign_name_alt(&mut spec, component),
            _ => {} // Ignore extra components
        }
    }

    Ok(SelectionExpr::Macro(spec))
}

#[derive(Debug, Default)]
struct MacroPart {
    value: String,
    present: bool,
    quoted: bool,
}

impl MacroPart {
    fn push_unquoted(&mut self, value: &str) {
        self.value.push_str(value);
        self.present = true;
    }

    fn push_quoted(&mut self, value: &str) {
        self.value.push_str(value);
        self.present = true;
        self.quoted = true;
    }

    fn into_pattern(self) -> Option<Pattern> {
        if !self.present {
            return None;
        }

        if self.quoted || !self.value.contains(['*', '?']) {
            Some(Pattern::Exact(self.value))
        } else {
            Some(Pattern::Wildcard(self.value))
        }
    }
}

#[derive(Debug, Default)]
struct MacroComponent {
    left: MacroPart,
    right: Option<MacroPart>,
}

impl MacroComponent {
    fn active_part(&mut self) -> &mut MacroPart {
        self.right.as_mut().unwrap_or(&mut self.left)
    }

    fn push_unquoted(&mut self, value: &str) {
        self.active_part().push_unquoted(value);
    }

    fn push_quoted(&mut self, value: &str) {
        self.active_part().push_quoted(value);
    }

    fn push_backtick(&mut self) {
        if let Some(right) = &mut self.right {
            right.push_unquoted("`");
        } else {
            self.right = Some(MacroPart::default());
        }
    }

    fn is_present(&self) -> bool {
        self.left.present || self.right.is_some()
    }

    fn into_single_pattern(self) -> Option<Pattern> {
        let mut left = self.left;
        if let Some(right) = self.right {
            left.push_unquoted("`");
            if right.quoted {
                left.quoted = true;
            }
            if right.present {
                left.push_unquoted(&right.value);
            }
        }
        left.into_pattern()
    }

    fn into_patterns(self) -> (Option<Pattern>, Option<Pattern>) {
        (
            self.left.into_pattern(),
            self.right.and_then(MacroPart::into_pattern),
        )
    }
}

/// Parse a resi string that may contain ranges (e.g. "3-5", "3:5") or lists (e.g. "3+5+7")
fn parse_resi_string(s: &str) -> Result<ResiSpec, ParseError> {
    let mut items = Vec::new();
    for part in s.split('+') {
        // Find range separator: `-` (not at position 0, which is a negative sign) or `:`
        let range_pos = part.find(':').or_else(|| part.find('-').filter(|&p| p > 0));
        if let Some(sep_pos) = range_pos {
            // Range: "3-5", "3:5", "3A-5B"
            let left = &part[..sep_pos];
            let right = &part[sep_pos + 1..];
            let (start, si) = parse_resi_with_inscode(left)?;
            let (end, ei) = parse_resi_with_inscode(right)?;
            items.push(match (si, ei) {
                (Some(a), Some(b)) => ResiItem::InsCodeRange(start, a, end, b),
                _ => ResiItem::Range(start, end),
            });
        } else {
            let (num, ins) = parse_resi_with_inscode(part)?;
            items.push(match ins {
                Some(c) => ResiItem::InsCode(num, c),
                None => ResiItem::Single(num),
            });
        }
    }
    if items.is_empty() {
        Err(ParseError::InvalidResidue(s.to_string()))
    } else {
        Ok(ResiSpec { items })
    }
}

/// Assign field 3 of a macro spec: resn, resi, or resn`resi
fn assign_resn_resi(spec: &mut MacroSpec, component: MacroComponent) -> Result<(), ParseError> {
    let (left, right) = component.into_patterns();
    if let Some(right) = right {
        // resn`resi syntax
        spec.resn = left;
        spec.resi = Some(parse_resi_string(&right.to_string())?);
    } else if let Some(Pattern::Exact(ref s)) = left {
        if s.starts_with(|c: char| c.is_ascii_digit() || c == '-') {
            spec.resi = Some(parse_resi_string(s)?);
        } else {
            spec.resn = left;
        }
    } else {
        spec.resn = left;
    }
    Ok(())
}

/// Assign field 4 of a macro spec: name or name`alt
fn assign_name_alt(spec: &mut MacroSpec, component: MacroComponent) {
    let (left, right) = component.into_patterns();
    spec.name = left;
    spec.alt = right;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_all() {
        let expr = parse_selection("all").unwrap();
        assert!(matches!(expr, SelectionExpr::All));
    }

    #[test]
    fn test_parse_name() {
        let expr = parse_selection("name CA").unwrap();
        assert!(matches!(expr, SelectionExpr::Name(Pattern::Exact(ref s)) if s == "CA"));
    }

    #[test]
    fn test_parse_empty_chain_pattern() {
        let expr = parse_selection("chain \"\"").unwrap();
        assert!(matches!(expr, SelectionExpr::Chain(Pattern::Exact(ref s)) if s.is_empty()));
    }

    #[test]
    fn test_parse_name_wildcard() {
        let expr = parse_selection("name C*").unwrap();
        assert!(matches!(expr, SelectionExpr::Name(Pattern::Wildcard(ref s)) if s == "C*"));
    }

    #[test]
    fn test_parse_and() {
        let expr = parse_selection("name CA and chain A").unwrap();
        assert!(matches!(expr, SelectionExpr::And(_, _)));
    }

    #[test]
    fn test_parse_or() {
        let expr = parse_selection("name CA or name CB").unwrap();
        assert!(matches!(expr, SelectionExpr::Or(_, _)));
    }

    #[test]
    fn test_parse_not() {
        let expr = parse_selection("not hydrogens").unwrap();
        assert!(matches!(expr, SelectionExpr::Not(_)));
    }

    #[test]
    fn test_parse_parens() {
        let expr = parse_selection("(name CA)").unwrap();
        assert!(matches!(expr, SelectionExpr::Name(_)));
    }

    #[test]
    fn test_parse_complex() {
        let expr = parse_selection("(name CA and chain A) or name CB").unwrap();
        assert!(matches!(expr, SelectionExpr::Or(_, _)));
    }

    #[test]
    fn test_parse_parens_with_or() {
        let expr = parse_selection("(chain A and resi 29) or (chain A and resi 31)").unwrap();
        assert!(matches!(expr, SelectionExpr::Or(_, _)));
    }

    #[test]
    fn test_parse_resi_single() {
        let expr = parse_selection("resi 100").unwrap();
        if let SelectionExpr::Resi(spec) = expr {
            assert_eq!(spec.items.len(), 1);
            assert!(matches!(spec.items[0], ResiItem::Single(100)));
        } else {
            panic!("Expected Resi");
        }
    }

    #[test]
    fn test_parse_resi_range() {
        let expr = parse_selection("resi 100-200").unwrap();
        if let SelectionExpr::Resi(spec) = expr {
            assert_eq!(spec.items.len(), 1);
            assert!(matches!(spec.items[0], ResiItem::Range(100, 200)));
        } else {
            panic!("Expected Resi");
        }
    }

    #[test]
    fn test_parse_byres() {
        let expr = parse_selection("byres name CA").unwrap();
        assert!(matches!(expr, SelectionExpr::ByRes(_)));
    }

    #[test]
    fn test_parse_within_prefix() {
        // Prefix form: within 5 of X → Within(5, All, X)
        let expr = parse_selection("within 5 of name CA").unwrap();
        if let SelectionExpr::Within(dist, source, _target) = expr {
            assert!((dist - 5.0).abs() < 0.01);
            assert!(matches!(*source, SelectionExpr::All));
        } else {
            panic!("Expected Within, got {:?}", expr);
        }
    }

    #[test]
    fn test_parse_within_infix() {
        // Infix form: X within 5 of Y → Within(5, X, Y)
        let expr = parse_selection("name CA within 5 of name CB").unwrap();
        if let SelectionExpr::Within(dist, source, target) = expr {
            assert!((dist - 5.0).abs() < 0.01);
            assert!(matches!(*source, SelectionExpr::Name(_)));
            assert!(matches!(*target, SelectionExpr::Name(_)));
        } else {
            panic!("Expected Within, got {:?}", expr);
        }
    }

    #[test]
    fn test_parse_beyond_infix() {
        let expr = parse_selection("name CA beyond 8 of resi 100").unwrap();
        if let SelectionExpr::Beyond(dist, source, _target) = expr {
            assert!((dist - 8.0).abs() < 0.01);
            assert!(matches!(*source, SelectionExpr::Name(_)));
        } else {
            panic!("Expected Beyond, got {:?}", expr);
        }
    }

    #[test]
    fn test_parse_within_infix_precedence() {
        // "and" should bind tighter than "within" (within prec=0x30=48, and prec=0x40=64)
        // so: name CA and name CB within 5 of name N
        //  → (name CA and name CB) within 5 of name N
        let expr = parse_selection("name CA and name CB within 5 of name N").unwrap();
        if let SelectionExpr::Within(_, source, _) = expr {
            assert!(matches!(*source, SelectionExpr::And(_, _)));
        } else {
            panic!("Expected Within at top level, got {:?}", expr);
        }
    }

    #[test]
    fn test_parse_b_factor() {
        let expr = parse_selection("b > 50").unwrap();
        if let SelectionExpr::BFactor(op, val) = expr {
            assert_eq!(op, CompareOp::Gt);
            assert!((val - 50.0).abs() < 0.01);
        } else {
            panic!("Expected BFactor");
        }
    }

    #[test]
    fn test_parse_name_list() {
        let expr = parse_selection("name CA+CB+CD").unwrap();
        if let SelectionExpr::Name(Pattern::List(patterns)) = expr {
            assert_eq!(patterns.len(), 3);
        } else {
            panic!("Expected Name with List pattern");
        }
    }

    #[test]
    fn test_parse_selection_reference() {
        let expr = parse_selection("%sele1").unwrap();
        if let SelectionExpr::Selection(name) = expr {
            assert_eq!(name, "sele1");
        } else {
            panic!("Expected Selection");
        }
    }

    #[test]
    fn test_parse_macro_leading_slash_with_empty_fields() {
        // /protein///100/CA → model=protein, resi=100, name=CA
        let expr = parse_selection("/protein///100/CA").unwrap();
        if let SelectionExpr::Macro(spec) = expr {
            assert!(matches!(spec.model, Some(Pattern::Exact(ref s)) if s == "protein"));
            assert!(spec.segi.is_none());
            assert!(spec.chain.is_none());
            assert!(matches!(spec.resi, Some(ref r) if r.items == vec![ResiItem::Single(100)]));
            assert!(matches!(spec.name, Some(Pattern::Exact(ref s)) if s == "CA"));
        } else {
            panic!("Expected Macro");
        }
    }

    #[test]
    fn test_parse_macro_all_fields() {
        // /protein/SEG1/A/100/CA → all 5 fields populated
        let expr = parse_selection("/protein/SEG1/A/100/CA").unwrap();
        if let SelectionExpr::Macro(spec) = expr {
            assert!(matches!(spec.model, Some(Pattern::Exact(ref s)) if s == "protein"));
            assert!(matches!(spec.segi, Some(Pattern::Exact(ref s)) if s == "SEG1"));
            assert!(matches!(spec.chain, Some(Pattern::Exact(ref s)) if s == "A"));
            assert!(matches!(spec.resi, Some(ref r) if r.items == vec![ResiItem::Single(100)]));
            assert!(matches!(spec.name, Some(Pattern::Exact(ref s)) if s == "CA"));
        } else {
            panic!("Expected Macro");
        }
    }

    #[test]
    fn test_parse_macro_all_empty() {
        // ///// → all fields None
        let expr = parse_selection("/////").unwrap();
        if let SelectionExpr::Macro(spec) = expr {
            assert!(spec.model.is_none());
            assert!(spec.segi.is_none());
            assert!(spec.chain.is_none());
            assert!(spec.resn.is_none());
            assert!(spec.resi.is_none());
            assert!(spec.name.is_none());
        } else {
            panic!("Expected Macro");
        }
    }

    #[test]
    fn test_parse_macro_no_leading_slash_3_fields() {
        // A/100/CA → chain=A, resi=100, name=CA (right-to-left)
        let expr = parse_selection("A/100/CA").unwrap();
        if let SelectionExpr::Macro(spec) = expr {
            assert!(spec.model.is_none());
            assert!(spec.segi.is_none());
            assert!(matches!(spec.chain, Some(Pattern::Exact(ref s)) if s == "A"));
            assert!(matches!(spec.resi, Some(ref r) if r.items == vec![ResiItem::Single(100)]));
            assert!(matches!(spec.name, Some(Pattern::Exact(ref s)) if s == "CA"));
        } else {
            panic!("Expected Macro, got {:?}", expr);
        }
    }

    #[test]
    fn test_parse_macro_no_leading_slash_2_fields() {
        // 100/CA → resi=100, name=CA
        let expr = parse_selection("100/CA").unwrap();
        if let SelectionExpr::Macro(spec) = expr {
            assert!(spec.model.is_none());
            assert!(spec.segi.is_none());
            assert!(spec.chain.is_none());
            assert!(matches!(spec.resi, Some(ref r) if r.items == vec![ResiItem::Single(100)]));
            assert!(matches!(spec.name, Some(Pattern::Exact(ref s)) if s == "CA"));
        } else {
            panic!("Expected Macro, got {:?}", expr);
        }
    }

    #[test]
    fn test_parse_macro_resn_non_numeric() {
        // /protein//A/ALA/CA → resn=ALA, name=CA
        let expr = parse_selection("/protein//A/ALA/CA").unwrap();
        if let SelectionExpr::Macro(spec) = expr {
            assert!(matches!(spec.model, Some(Pattern::Exact(ref s)) if s == "protein"));
            assert!(matches!(spec.chain, Some(Pattern::Exact(ref s)) if s == "A"));
            assert!(matches!(spec.resn, Some(Pattern::Exact(ref s)) if s == "ALA"));
            assert!(spec.resi.is_none());
            assert!(matches!(spec.name, Some(Pattern::Exact(ref s)) if s == "CA"));
        } else {
            panic!("Expected Macro");
        }
    }

    #[test]
    fn test_parse_macro_resi_numeric() {
        // /protein//A/100/CA → resi=100, name=CA
        let expr = parse_selection("/protein//A/100/CA").unwrap();
        if let SelectionExpr::Macro(spec) = expr {
            assert!(matches!(spec.resi, Some(ref r) if r.items == vec![ResiItem::Single(100)]));
            assert!(spec.resn.is_none());
        } else {
            panic!("Expected Macro");
        }
    }

    #[test]
    fn test_parse_macro_wildcard() {
        // /protein//*/100/C* → chain=* (wildcard), name=C* (wildcard)
        let expr = parse_selection("/protein//*/100/C*").unwrap();
        if let SelectionExpr::Macro(spec) = expr {
            assert!(matches!(spec.chain, Some(Pattern::Wildcard(ref s)) if s == "*"));
            assert!(matches!(spec.name, Some(Pattern::Wildcard(ref s)) if s == "C*"));
        } else {
            panic!("Expected Macro");
        }
    }

    #[test]
    fn test_parse_macro_quoted_empty_and_escaped_components_are_exact() {
        let expr =
            parse_selection(r#"/"model/\"quoted\"\\tail"/""/""/"ALA"`"42A"/"CA"`"""#).unwrap();

        let SelectionExpr::Macro(spec) = expr else {
            panic!("Expected Macro");
        };
        assert!(matches!(spec.model, Some(Pattern::Exact(ref s)) if s == "model/\"quoted\"\\tail"));
        assert!(matches!(spec.segi, Some(Pattern::Exact(ref s)) if s.is_empty()));
        assert!(matches!(spec.chain, Some(Pattern::Exact(ref s)) if s.is_empty()));
        assert!(matches!(spec.resn, Some(Pattern::Exact(ref s)) if s == "ALA"));
        assert!(matches!(
            spec.resi,
            Some(ref r) if r.items == vec![ResiItem::InsCode(42, 'A')]
        ));
        assert!(matches!(spec.name, Some(Pattern::Exact(ref s)) if s == "CA"));
        assert!(matches!(spec.alt, Some(Pattern::Exact(ref s)) if s.is_empty()));
    }

    #[test]
    fn test_parse_macro_quoted_wildcards_are_literal() {
        let expr = parse_selection(r#"/"*"/"?"/*/?/"C*""#).unwrap();

        let SelectionExpr::Macro(spec) = expr else {
            panic!("Expected Macro");
        };
        assert!(matches!(spec.model, Some(Pattern::Exact(ref s)) if s == "*"));
        assert!(matches!(spec.segi, Some(Pattern::Exact(ref s)) if s == "?"));
        assert!(matches!(spec.chain, Some(Pattern::Wildcard(ref s)) if s == "*"));
        assert!(matches!(spec.resn, Some(Pattern::Wildcard(ref s)) if s == "?"));
        assert!(matches!(spec.name, Some(Pattern::Exact(ref s)) if s == "C*"));
    }

    #[test]
    fn test_parse_macro_compound_fields_preserve_blank_and_nonblank_alt() {
        for (input, expected_inscode, expected_alt) in [
            (r#"////"GLY"`"42 "/"CA"`" ""#, ' ', " "),
            (r#"////"GLY"`"42A"/"CA"`"B""#, 'A', "B"),
        ] {
            let expr = parse_selection(input).unwrap();
            let SelectionExpr::Macro(spec) = expr else {
                panic!("Expected Macro");
            };

            assert!(matches!(spec.resn, Some(Pattern::Exact(ref s)) if s == "GLY"));
            assert!(matches!(
                spec.resi,
                Some(ref r) if r.items == vec![ResiItem::InsCode(42, expected_inscode)]
            ));
            assert!(matches!(spec.name, Some(Pattern::Exact(ref s)) if s == "CA"));
            assert!(matches!(spec.alt, Some(Pattern::Exact(ref s)) if s == expected_alt));
        }
    }

    #[test]
    fn test_parse_macro_resi_range() {
        // ////3-5/CA → resi=3-5 (range), name=CA
        let expr = parse_selection("////3-5/CA").unwrap();
        if let SelectionExpr::Macro(spec) = expr {
            assert!(spec.model.is_none());
            assert!(spec.segi.is_none());
            assert!(spec.chain.is_none());
            assert!(matches!(spec.resi, Some(ref r) if r.items == vec![ResiItem::Range(3, 5)]));
            assert!(matches!(spec.name, Some(Pattern::Exact(ref s)) if s == "CA"));
        } else {
            panic!("Expected Macro, got {:?}", expr);
        }
    }

    #[test]
    fn test_parse_macro_resi_list() {
        // ///A/1+5/CA → chain=A, resi=1+5 (list), name=CA
        let expr = parse_selection("///A/1+5/CA").unwrap();
        if let SelectionExpr::Macro(spec) = expr {
            assert!(matches!(spec.chain, Some(Pattern::Exact(ref s)) if s == "A"));
            assert!(
                matches!(spec.resi, Some(ref r) if r.items == vec![ResiItem::Single(1), ResiItem::Single(5)])
            );
            assert!(matches!(spec.name, Some(Pattern::Exact(ref s)) if s == "CA"));
        } else {
            panic!("Expected Macro, got {:?}", expr);
        }
    }

    #[test]
    fn test_parse_macro_resi_colon_range() {
        // ////1:5/CA → resi=1-5 (colon range), name=CA
        let expr = parse_selection("////1:5/CA").unwrap();
        if let SelectionExpr::Macro(spec) = expr {
            assert!(matches!(spec.resi, Some(ref r) if r.items == vec![ResiItem::Range(1, 5)]));
            assert!(matches!(spec.name, Some(Pattern::Exact(ref s)) if s == "CA"));
        } else {
            panic!("Expected Macro, got {:?}", expr);
        }
    }

    #[test]
    fn test_parse_macro_bare_ca() {
        // CA → should parse as Selection("CA") (named selection reference)
        let expr = parse_selection("CA").unwrap();
        assert!(matches!(expr, SelectionExpr::Selection(ref s) if s == "CA"));
    }

    #[test]
    fn test_parse_macro_1_slash_ca() {
        // 1/CA → resi=1, name=CA (right-to-left macro)
        let expr = parse_selection("1/CA").unwrap();
        if let SelectionExpr::Macro(spec) = expr {
            assert!(matches!(spec.resi, Some(ref r) if r.items == vec![ResiItem::Single(1)]));
            assert!(matches!(spec.name, Some(Pattern::Exact(ref s)) if s == "CA"));
        } else {
            panic!("Expected Macro, got {:?}", expr);
        }
    }

    #[test]
    fn test_parse_macro_1_plus_5_slash_ca() {
        // 1+5/CA → resi=1+5, name=CA (right-to-left macro)
        let expr = parse_selection("1+5/CA").unwrap();
        if let SelectionExpr::Macro(spec) = expr {
            assert!(
                matches!(spec.resi, Some(ref r) if r.items == vec![ResiItem::Single(1), ResiItem::Single(5)])
            );
            assert!(matches!(spec.name, Some(Pattern::Exact(ref s)) if s == "CA"));
        } else {
            panic!("Expected Macro, got {:?}", expr);
        }
    }
}
