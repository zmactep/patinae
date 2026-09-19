//! Space group symmetry operation lookup and parsing
//!
//! Maps space group names to lists of 4×4 transformation matrices (in fractional
//! coordinates). Data derived from International Tables for Crystallography, Vol. A.

use lin_alg::f32::Mat4;

use super::space_groups_data::{ALIASES, SYM_OPS};

/// Get the symmetry operation matrices for a space group.
///
/// Each matrix is a 4×4 Mat4 (column-major, lin_alg convention) representing a symmetry
/// operation in fractional coordinates. Returns `None` if the space group is unknown.
pub fn get_symops(space_group: &str) -> Option<Vec<Mat4>> {
    let canonical = canonicalize(space_group);
    let ops = SYM_OPS.get(canonical.as_str())?;
    Some(ops.iter().map(|op| parse_symop(op)).collect())
}

/// Canonicalize a space group name: normalize whitespace, uppercase, resolve aliases
pub fn canonicalize(sg: &str) -> String {
    // Normalize whitespace and uppercase
    let normalized: String = sg
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_uppercase();

    // Check alias table
    if let Some(canonical) = ALIASES.get(normalized.as_str()) {
        return canonical.to_string();
    }

    // Try with spaces stripped (e.g., "P212121" -> check alias)
    let no_spaces: String = normalized.chars().filter(|c| *c != ' ').collect();
    if no_spaces != normalized {
        if let Some(canonical) = ALIASES.get(no_spaces.as_str()) {
            return canonical.to_string();
        }
    }

    normalized
}

/// Parse a symmetry operation string like `"-x+1/2,y,-z"` into a 4×4 column-major Mat4
///
/// Each comma-separated component defines one row of the 3×3 rotation part
/// plus the translation. For example, `-x+1/2` means coefficient -1 for x,
/// 0 for y, 0 for z, and translation 1/2.
pub fn parse_symop(op: &str) -> Mat4 {
    let mut data = [0.0f32; 16];
    data[15] = 1.0;

    for (row, expr) in op.split(',').enumerate() {
        if row >= 3 {
            break;
        }
        let (coeffs, trans) = parse_expr(expr.trim());
        // Column-major: data[col*4 + row]
        data[row] = coeffs[0]; // x coefficient → col 0
        data[4 + row] = coeffs[1]; // y coefficient → col 1
        data[8 + row] = coeffs[2]; // z coefficient → col 2
        data[12 + row] = trans; // translation → col 3
    }

    Mat4::new(data)
}

/// Parse a single symmetry expression component like `-x+1/2` or `y-z`
///
/// Returns ([coeff_x, coeff_y, coeff_z], translation)
fn parse_expr(expr: &str) -> ([f32; 3], f32) {
    let mut coeffs = [0.0f32; 3];
    let mut trans = 0.0f32;

    let chars: Vec<char> = expr.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        let c = chars[i];

        // Determine sign
        let sign = if c == '-' {
            i += 1;
            -1.0
        } else if c == '+' {
            i += 1;
            1.0
        } else {
            1.0
        };

        if i >= len {
            break;
        }

        let c = chars[i];

        match c {
            'x' | 'X' => {
                coeffs[0] = sign;
                i += 1;
            }
            'y' | 'Y' => {
                coeffs[1] = sign;
                i += 1;
            }
            'z' | 'Z' => {
                coeffs[2] = sign;
                i += 1;
            }
            '0'..='9' => {
                // Parse a number, possibly a fraction like "1/2" or "1/3"
                let (val, consumed) = parse_number(&chars[i..]);
                trans += sign * val;
                i += consumed;
            }
            ' ' => {
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    (coeffs, trans)
}

/// Parse a number from a character slice. Handles integers and fractions like "1/2".
/// Returns (value, chars_consumed).
fn parse_number(chars: &[char]) -> (f32, usize) {
    let mut i = 0;
    let mut numerator = 0.0f32;

    // Parse integer part
    while i < chars.len() && chars[i].is_ascii_digit() {
        numerator = numerator * 10.0 + (chars[i] as u32 - '0' as u32) as f32;
        i += 1;
    }

    // Check for fraction
    if i < chars.len() && chars[i] == '/' {
        i += 1;
        let mut denominator = 0.0f32;
        while i < chars.len() && chars[i].is_ascii_digit() {
            denominator = denominator * 10.0 + (chars[i] as u32 - '0' as u32) as f32;
            i += 1;
        }
        if denominator != 0.0 {
            return (numerator / denominator, i);
        }
    }

    (numerator, i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_identity() {
        let mat = parse_symop("x,y,z");
        // Column-major: M[row,col] = data[col*4 + row]
        // Diagonal: M[0,0]=data[0], M[1,1]=data[5], M[2,2]=data[10]
        assert_eq!(mat.data[0], 1.0); // x -> x
        assert_eq!(mat.data[5], 1.0); // y -> y
        assert_eq!(mat.data[10], 1.0); // z -> z
                                       // No translation (col 3): data[12], data[13], data[14]
        assert_eq!(mat.data[12], 0.0);
        assert_eq!(mat.data[13], 0.0);
        assert_eq!(mat.data[14], 0.0);
    }

    #[test]
    fn test_parse_negative_with_translation() {
        let mat = parse_symop("-x+1/2,-y+1/2,z");
        // Column-major: M[row,col] = data[col*4 + row]
        assert_eq!(mat.data[0], -1.0); // M[0,0] = -x
        assert_eq!(mat.data[12], 0.5); // M[0,3] = +1/2
        assert_eq!(mat.data[5], -1.0); // M[1,1] = -y
        assert_eq!(mat.data[13], 0.5); // M[1,3] = +1/2
        assert_eq!(mat.data[10], 1.0); // M[2,2] = z
        assert_eq!(mat.data[14], 0.0); // M[2,3] = 0
    }

    #[test]
    fn test_parse_mixed() {
        let mat = parse_symop("-x,y+1/2,-z+1/2");
        assert_eq!(mat.data[0], -1.0); // M[0,0]
        assert_eq!(mat.data[12], 0.0); // M[0,3]
        assert_eq!(mat.data[5], 1.0); // M[1,1]
        assert_eq!(mat.data[13], 0.5); // M[1,3]
        assert_eq!(mat.data[10], -1.0); // M[2,2]
        assert_eq!(mat.data[14], 0.5); // M[2,3]
    }

    #[test]
    fn test_parse_third_fraction() {
        let mat = parse_symop("x+1/3,y+2/3,z+2/3");
        assert_eq!(mat.data[0], 1.0); // M[0,0]
        assert!((mat.data[12] - 1.0 / 3.0).abs() < 1e-6); // M[0,3]
        assert_eq!(mat.data[5], 1.0); // M[1,1]
        assert!((mat.data[13] - 2.0 / 3.0).abs() < 1e-6); // M[1,3]
        assert_eq!(mat.data[10], 1.0); // M[2,2]
        assert!((mat.data[14] - 2.0 / 3.0).abs() < 1e-6); // M[2,3]
    }

    #[test]
    fn test_get_symops_p1() {
        let ops = get_symops("P 1").unwrap();
        assert_eq!(ops.len(), 1);
        // Identity: diagonal elements
        assert_eq!(ops[0].data[0], 1.0);
        assert_eq!(ops[0].data[5], 1.0);
        assert_eq!(ops[0].data[10], 1.0);
    }

    #[test]
    fn space_group_aliases_preserve_all_operators() {
        let canonical = get_symops("P 21 21 21").unwrap();
        assert_eq!(canonical.len(), 4);
        for name in ["P212121", "p 21 21 21"] {
            let actual = get_symops(name).unwrap();
            assert_eq!(actual.len(), canonical.len(), "{name}");
            for (actual, expected) in actual.iter().zip(&canonical) {
                assert_eq!(actual.data, expected.data, "{name}");
            }
        }
    }

    #[test]
    fn test_get_symops_unknown() {
        assert!(get_symops("NONEXISTENT").is_none());
    }

    #[test]
    fn test_canonicalize() {
        assert_eq!(canonicalize("P212121"), "P 21 21 21");
        assert_eq!(canonicalize("p 1"), "P 1");
        assert_eq!(canonicalize("  P  21  21  21  "), "P 21 21 21");
    }

    #[test]
    fn test_p_minus_1() {
        let ops = get_symops("P -1").unwrap();
        assert_eq!(ops.len(), 2);
        // Second op should be -x,-y,-z (inversion)
        assert_eq!(ops[1].data[0], -1.0);
        assert_eq!(ops[1].data[5], -1.0);
        assert_eq!(ops[1].data[10], -1.0);
    }
}
