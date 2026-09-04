//! BinaryCIF column decoders
//!
//! Implements all 7 encoding types defined by the BinaryCIF specification.
//! Decoding applies encodings in reverse order (last encoding first).

use crate::error::{IoError, IoResult};

use super::types::{BcifData, BcifEncoding};

// Data type constants (from BinaryCIF spec)
const TYPE_INT8: i32 = 1;
const TYPE_INT16: i32 = 2;
const TYPE_INT32: i32 = 3;
const TYPE_UINT8: i32 = 4;
const TYPE_UINT16: i32 = 5;
const TYPE_UINT32: i32 = 6;
const TYPE_FLOAT32: i32 = 32;
const TYPE_FLOAT64: i32 = 33;

/// Result of decoding a bCIF column
pub enum DecodedColumn {
    /// Integer values
    Int(Vec<i32>),
    /// Float values
    Float(Vec<f32>),
    /// String values
    String(Vec<String>),
}

impl DecodedColumn {
    /// Get a string value at index, returning None for out-of-bounds
    pub fn str_at(&self, i: usize) -> Option<&str> {
        match self {
            DecodedColumn::String(v) => v.get(i).map(|s| s.as_str()),
            _ => None,
        }
    }

    /// Get an i32 value at index
    pub fn int_at(&self, i: usize) -> Option<i32> {
        match self {
            DecodedColumn::Int(v) => v.get(i).copied(),
            _ => None,
        }
    }

    /// Get an f32 value at index
    pub fn float_at(&self, i: usize) -> Option<f32> {
        match self {
            DecodedColumn::Float(v) => v.get(i).copied(),
            _ => None,
        }
    }
}

/// Mask values for a column
pub struct ColumnMask(pub Vec<u8>);

impl ColumnMask {
    /// Check if value at index is present (mask value 0)
    pub fn is_present(&self, i: usize) -> bool {
        self.0.get(i).copied().unwrap_or(0) == 0
    }
}

/// Intermediate buffer during multi-step decoding
enum DecodeBuffer {
    Bytes(Vec<u8>),
    I32(Vec<i32>),
    F32(Vec<f32>),
    F64(Vec<f64>),
    Strings(Vec<String>),
}

/// Decode a bCIF data blob into a typed column
pub fn decode_column(data: &BcifData) -> IoResult<DecodedColumn> {
    let mut buffer = DecodeBuffer::Bytes(data.data.clone());

    // Apply encodings in reverse order
    for encoding in data.encoding.iter().rev() {
        buffer = apply_encoding(buffer, encoding)?;
    }

    match buffer {
        DecodeBuffer::I32(v) => Ok(DecodedColumn::Int(v)),
        DecodeBuffer::F32(v) => Ok(DecodedColumn::Float(v)),
        DecodeBuffer::F64(v) => Ok(DecodedColumn::Float(
            v.into_iter().map(|x| x as f32).collect(),
        )),
        DecodeBuffer::Strings(v) => Ok(DecodedColumn::String(v)),
        DecodeBuffer::Bytes(v) => Ok(DecodedColumn::Int(
            v.into_iter().map(|b| b as i32).collect(),
        )),
    }
}

/// Decode a mask data blob
pub fn decode_mask(mask: &Option<BcifData>) -> IoResult<Option<ColumnMask>> {
    match mask {
        None => Ok(None),
        Some(data) => {
            let decoded = decode_column(data)?;
            match decoded {
                DecodedColumn::Int(v) => {
                    Ok(Some(ColumnMask(v.into_iter().map(|i| i as u8).collect())))
                }
                _ => Err(IoError::parse_msg("Invalid mask encoding")),
            }
        }
    }
}

/// Apply a single encoding step
fn apply_encoding(buffer: DecodeBuffer, encoding: &BcifEncoding) -> IoResult<DecodeBuffer> {
    match encoding {
        BcifEncoding::ByteArray { data_type } => decode_byte_array(buffer, *data_type),
        BcifEncoding::FixedPoint { factor, .. } => decode_fixed_point(buffer, *factor),
        BcifEncoding::IntervalQuantization {
            min,
            max,
            num_steps,
            ..
        } => decode_interval_quantization(buffer, *min, *max, *num_steps),
        BcifEncoding::RunLength { src_size, .. } => decode_run_length(buffer, *src_size),
        BcifEncoding::Delta { origin, .. } => decode_delta(buffer, *origin),
        BcifEncoding::IntegerPacking {
            byte_count,
            src_size,
            is_unsigned,
        } => decode_integer_packing(buffer, *byte_count, *src_size, *is_unsigned),
        BcifEncoding::StringArray {
            data_encoding,
            string_data,
            offset_encoding,
            offsets,
        } => {
            // StringArray is always the outermost encoding. The buffer at this point
            // contains the raw bytes from BcifData.data, which encode the indices
            // via the data_encoding chain.
            let raw_bytes = match buffer {
                DecodeBuffer::Bytes(v) => v,
                _ => return Err(IoError::parse_msg("StringArray expects raw bytes buffer")),
            };
            decode_string_array(
                &raw_bytes,
                data_encoding,
                string_data,
                offset_encoding,
                offsets,
            )
        }
    }
}

/// ByteArray: reinterpret raw bytes as a typed array (little-endian)
fn decode_byte_array(buffer: DecodeBuffer, data_type: i32) -> IoResult<DecodeBuffer> {
    let bytes = match buffer {
        DecodeBuffer::Bytes(v) => v,
        _ => return Err(IoError::parse_msg("ByteArray expects raw bytes")),
    };

    Ok(match data_type {
        TYPE_INT8 => DecodeBuffer::I32(bytes.iter().map(|&b| b as i8 as i32).collect()),
        TYPE_INT16 => DecodeBuffer::I32(
            bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&c| i16::from_le_bytes(c) as i32)
                .collect(),
        ),
        TYPE_INT32 => DecodeBuffer::I32(
            bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|&c| i32::from_le_bytes(c))
                .collect(),
        ),
        TYPE_UINT8 => DecodeBuffer::I32(bytes.iter().map(|&b| b as i32).collect()),
        TYPE_UINT16 => DecodeBuffer::I32(
            bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&c| u16::from_le_bytes(c) as i32)
                .collect(),
        ),
        TYPE_UINT32 => DecodeBuffer::I32(
            bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|&c| u32::from_le_bytes(c) as i32)
                .collect(),
        ),
        TYPE_FLOAT32 => DecodeBuffer::F32(
            bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|&c| f32::from_le_bytes(c))
                .collect(),
        ),
        TYPE_FLOAT64 => DecodeBuffer::F64(
            bytes
                .as_chunks::<8>()
                .0
                .iter()
                .map(|&c| f64::from_le_bytes(c))
                .collect(),
        ),
        _ => {
            return Err(IoError::parse_msg(format!(
                "Unknown ByteArray type: {}",
                data_type
            )))
        }
    })
}

/// FixedPoint: divide integers by factor to produce floats
fn decode_fixed_point(buffer: DecodeBuffer, factor: f64) -> IoResult<DecodeBuffer> {
    let ints = to_i32_vec(buffer)?;
    Ok(DecodeBuffer::F32(
        ints.into_iter()
            .map(|v| (v as f64 / factor) as f32)
            .collect(),
    ))
}

/// IntervalQuantization: map integers [0..numSteps] to floats [min..max]
fn decode_interval_quantization(
    buffer: DecodeBuffer,
    min: f64,
    max: f64,
    num_steps: i32,
) -> IoResult<DecodeBuffer> {
    let ints = to_i32_vec(buffer)?;
    let delta = (max - min) / num_steps as f64;
    Ok(DecodeBuffer::F32(
        ints.into_iter()
            .map(|v| (min + v as f64 * delta) as f32)
            .collect(),
    ))
}

/// RunLength: expand (value, count) pairs
fn decode_run_length(buffer: DecodeBuffer, src_size: i32) -> IoResult<DecodeBuffer> {
    let ints = to_i32_vec(buffer)?;
    let mut result = Vec::with_capacity(src_size as usize);
    for pair in ints.as_chunks::<2>().0 {
        let value = pair[0];
        let count = pair[1] as usize;
        result.extend(std::iter::repeat_n(value, count));
    }
    Ok(DecodeBuffer::I32(result))
}

/// Delta: cumulative sum from origin
fn decode_delta(buffer: DecodeBuffer, origin: i64) -> IoResult<DecodeBuffer> {
    let ints = to_i32_vec(buffer)?;
    let mut result = Vec::with_capacity(ints.len());
    let mut current = origin;
    for v in ints {
        current += v as i64;
        result.push(current as i32);
    }
    Ok(DecodeBuffer::I32(result))
}

/// IntegerPacking: expand packed 8/16-bit integers to 32-bit with overflow accumulation.
///
/// Values at the boundary of the packed range (e.g., 127/-128 for i8, 32767/-32768 for i16)
/// indicate overflow and are accumulated until a non-boundary value is reached.
fn decode_integer_packing(
    buffer: DecodeBuffer,
    byte_count: i32,
    src_size: i32,
    is_unsigned: bool,
) -> IoResult<DecodeBuffer> {
    let ints = to_i32_vec(buffer)?;
    let mut result = Vec::with_capacity(src_size as usize);

    // For signed types, both upper and lower boundaries indicate overflow.
    // For unsigned types, only the upper boundary indicates overflow — 0 is a valid value.
    let (upper_limit, lower_limit): (i32, i32) = match (byte_count, is_unsigned) {
        (1, true) => (0xFF, i32::MIN),
        (1, false) => (0x7F, -0x80),
        (2, true) => (0xFFFF, i32::MIN),
        (2, false) => (0x7FFF, -0x8000),
        _ => {
            return Err(IoError::parse_msg(format!(
                "Invalid byteCount: {}",
                byte_count
            )))
        }
    };

    let mut i = 0;
    while i < ints.len() {
        let mut value: i32 = 0;
        loop {
            let packed = ints[i];
            value += packed;
            i += 1;
            if packed != upper_limit && packed != lower_limit {
                break;
            }
            if i >= ints.len() {
                break;
            }
        }
        result.push(value);
    }

    Ok(DecodeBuffer::I32(result))
}

/// StringArray: decode indices + offsets + string table.
///
/// `index_bytes` are the raw bytes from BcifData.data (encoded indices).
/// `data_encoding` describes how to decode those bytes into integer indices.
/// `offset_encoding` + `offsets_bytes` decode to integer offsets into `string_data`.
fn decode_string_array(
    index_bytes: &[u8],
    data_encoding: &[BcifEncoding],
    string_data: &str,
    offset_encoding: &[BcifEncoding],
    offsets_bytes: &[u8],
) -> IoResult<DecodeBuffer> {
    // Decode offset array
    let offsets_data = BcifData {
        data: offsets_bytes.to_vec(),
        encoding: offset_encoding.to_vec(),
    };
    let offsets = match decode_column(&offsets_data)? {
        DecodedColumn::Int(v) => v,
        _ => return Err(IoError::parse_msg("StringArray offsets must be integers")),
    };

    // Build string table from offsets into string_data
    let string_bytes = string_data.as_bytes();
    let mut strings: Vec<&str> = Vec::with_capacity(offsets.len().saturating_sub(1));
    for w in offsets.windows(2) {
        let start = w[0] as usize;
        let end = w[1] as usize;
        let s = std::str::from_utf8(&string_bytes[start..end]).unwrap_or("");
        strings.push(s);
    }

    // Decode index array from the parent's raw bytes
    let indices_data = BcifData {
        data: index_bytes.to_vec(),
        encoding: data_encoding.to_vec(),
    };
    let indices = match decode_column(&indices_data)? {
        DecodedColumn::Int(v) => v,
        _ => return Err(IoError::parse_msg("StringArray indices must be integers")),
    };

    // Map indices to strings
    let result: Vec<String> = indices
        .into_iter()
        .map(|idx| {
            let idx = idx as usize;
            if idx < strings.len() {
                strings[idx].to_string()
            } else {
                String::new()
            }
        })
        .collect();

    Ok(DecodeBuffer::Strings(result))
}

/// Convert buffer to Vec<i32>
fn to_i32_vec(buffer: DecodeBuffer) -> IoResult<Vec<i32>> {
    match buffer {
        DecodeBuffer::I32(v) => Ok(v),
        DecodeBuffer::Bytes(v) => Ok(v.into_iter().map(|b| b as i32).collect()),
        _ => Err(IoError::parse_msg("Expected integer buffer")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_byte_array_int32() {
        let data = BcifData {
            data: vec![
                0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00,
            ],
            encoding: vec![BcifEncoding::ByteArray {
                data_type: TYPE_INT32,
            }],
        };
        let col = decode_column(&data).unwrap();
        assert!(matches!(col, DecodedColumn::Int(ref v) if v == &[1, 2, 3]));
    }

    #[test]
    fn test_byte_array_float32() {
        let data = BcifData {
            data: 1.5f32
                .to_le_bytes()
                .iter()
                .chain(2.5f32.to_le_bytes().iter())
                .copied()
                .collect(),
            encoding: vec![BcifEncoding::ByteArray {
                data_type: TYPE_FLOAT32,
            }],
        };
        let col = decode_column(&data).unwrap();
        match col {
            DecodedColumn::Float(v) => {
                assert_eq!(v.len(), 2);
                assert!((v[0] - 1.5).abs() < 1e-6);
                assert!((v[1] - 2.5).abs() < 1e-6);
            }
            _ => panic!("Expected Float"),
        }
    }

    #[test]
    fn test_fixed_point() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1234i32.to_le_bytes());
        bytes.extend_from_slice(&5678i32.to_le_bytes());
        let data = BcifData {
            data: bytes,
            encoding: vec![
                BcifEncoding::FixedPoint {
                    factor: 1000.0,
                    src_type: TYPE_FLOAT32,
                },
                BcifEncoding::ByteArray {
                    data_type: TYPE_INT32,
                },
            ],
        };
        let col = decode_column(&data).unwrap();
        match col {
            DecodedColumn::Float(v) => {
                assert_eq!(v.len(), 2);
                assert!((v[0] - 1.234).abs() < 1e-6);
                assert!((v[1] - 5.678).abs() < 1e-6);
            }
            _ => panic!("Expected Float"),
        }
    }

    #[test]
    fn test_delta() {
        let mut bytes = Vec::new();
        for v in [0i32, 3, -1] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        let data = BcifData {
            data: bytes,
            encoding: vec![
                BcifEncoding::Delta {
                    origin: 10,
                    src_type: TYPE_INT32,
                },
                BcifEncoding::ByteArray {
                    data_type: TYPE_INT32,
                },
            ],
        };
        let col = decode_column(&data).unwrap();
        assert!(matches!(col, DecodedColumn::Int(ref v) if v == &[10, 13, 12]));
    }

    #[test]
    fn test_run_length() {
        let mut bytes = Vec::new();
        for v in [3i32, 2, 5, 1] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        let data = BcifData {
            data: bytes,
            encoding: vec![
                BcifEncoding::RunLength {
                    src_type: TYPE_INT32,
                    src_size: 3,
                },
                BcifEncoding::ByteArray {
                    data_type: TYPE_INT32,
                },
            ],
        };
        let col = decode_column(&data).unwrap();
        assert!(matches!(col, DecodedColumn::Int(ref v) if v == &[3, 3, 5]));
    }

    #[test]
    fn test_integer_packing_signed_byte() {
        // Pack 130 as [127, 3] (127 is overflow boundary for signed i8)
        // Pack 50 as [50]
        let bytes: Vec<u8> = vec![127, 3, 50];
        let data = BcifData {
            data: bytes,
            encoding: vec![
                BcifEncoding::IntegerPacking {
                    byte_count: 1,
                    src_size: 2,
                    is_unsigned: false,
                },
                BcifEncoding::ByteArray {
                    data_type: TYPE_INT8,
                },
            ],
        };
        let col = decode_column(&data).unwrap();
        assert!(matches!(col, DecodedColumn::Int(ref v) if v == &[130, 50]));
    }

    #[test]
    fn test_integer_packing_negative() {
        // Pack -130 as [-128, -2] (-128 is negative overflow boundary for signed i8)
        let bytes: Vec<u8> = vec![0x80, 0xFE, 10]; // -128, -2, 10 as i8
        let data = BcifData {
            data: bytes,
            encoding: vec![
                BcifEncoding::IntegerPacking {
                    byte_count: 1,
                    src_size: 2,
                    is_unsigned: false,
                },
                BcifEncoding::ByteArray {
                    data_type: TYPE_INT8,
                },
            ],
        };
        let col = decode_column(&data).unwrap();
        assert!(matches!(col, DecodedColumn::Int(ref v) if v == &[-130, 10]));
    }

    #[test]
    fn test_interval_quantization() {
        let mut bytes = Vec::new();
        for v in [0i32, 5, 10] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        let data = BcifData {
            data: bytes,
            encoding: vec![
                BcifEncoding::IntervalQuantization {
                    min: 0.0,
                    max: 10.0,
                    num_steps: 10,
                    src_type: TYPE_FLOAT32,
                },
                BcifEncoding::ByteArray {
                    data_type: TYPE_INT32,
                },
            ],
        };
        let col = decode_column(&data).unwrap();
        match col {
            DecodedColumn::Float(v) => {
                assert_eq!(v.len(), 3);
                assert!((v[0] - 0.0).abs() < 1e-6);
                assert!((v[1] - 5.0).abs() < 1e-6);
                assert!((v[2] - 10.0).abs() < 1e-6);
            }
            _ => panic!("Expected Float"),
        }
    }

    #[test]
    fn test_integer_packing_unsigned_zero() {
        // Unsigned u16 packing: [0, 8443, 1, 1, 0, 795]
        // 0 is a valid value (NOT an overflow boundary for unsigned types).
        // Should produce 6 values: [0, 8443, 1, 1, 0, 795]
        let mut bytes = Vec::new();
        for v in [0u16, 8443, 1, 1, 0, 795] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        let data = BcifData {
            data: bytes,
            encoding: vec![
                BcifEncoding::IntegerPacking {
                    byte_count: 2,
                    src_size: 6,
                    is_unsigned: true,
                },
                BcifEncoding::ByteArray {
                    data_type: TYPE_UINT16,
                },
            ],
        };
        let col = decode_column(&data).unwrap();
        assert!(matches!(col, DecodedColumn::Int(ref v) if v == &[0, 8443, 1, 1, 0, 795]));
    }

    #[test]
    fn test_string_array() {
        // String table: "A\0B\0CA" -> offsets [0,1,2,4] -> ["A", "B", "CA"]
        // Indices: [0, 2, 1, 0] -> ["A", "CA", "B", "A"]
        let mut index_bytes = Vec::new();
        for v in [0i32, 2, 1, 0] {
            index_bytes.extend_from_slice(&v.to_le_bytes());
        }
        let mut offset_bytes = Vec::new();
        for v in [0i32, 1, 2, 4] {
            offset_bytes.extend_from_slice(&v.to_le_bytes());
        }

        let data = BcifData {
            data: index_bytes,
            encoding: vec![BcifEncoding::StringArray {
                data_encoding: vec![BcifEncoding::ByteArray {
                    data_type: TYPE_INT32,
                }],
                string_data: "ABCA".to_string(),
                offset_encoding: vec![BcifEncoding::ByteArray {
                    data_type: TYPE_INT32,
                }],
                offsets: offset_bytes,
            }],
        };
        let col = decode_column(&data).unwrap();
        match col {
            DecodedColumn::String(v) => {
                assert_eq!(v, vec!["A", "CA", "B", "A"]);
            }
            _ => panic!("Expected String"),
        }
    }
}
