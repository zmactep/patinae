use std::collections::HashMap;

use super::PickleValue;
use crate::SessionError;

/// Deserialize pickle bytes into a [`PickleValue`].
pub fn read(data: &[u8]) -> Result<PickleValue, SessionError> {
    let mut vm = Vm::new(data);
    vm.run()
}

struct Vm<'a> {
    data: &'a [u8],
    pos: usize,
    stack: Vec<StackItem>,
    memo: HashMap<u32, PickleValue>,
    memo_counter: u32,
}

#[derive(Debug)]
enum StackItem {
    Value(PickleValue),
    Mark,
}

impl StackItem {
    fn into_value(self) -> Result<PickleValue, SessionError> {
        match self {
            StackItem::Value(v) => Ok(v),
            StackItem::Mark => Err(SessionError::Pickle("unexpected mark on stack".into())),
        }
    }
}

impl<'a> Vm<'a> {
    fn new(data: &'a [u8]) -> Self {
        Vm {
            data,
            pos: 0,
            stack: Vec::new(),
            memo: HashMap::new(),
            memo_counter: 0,
        }
    }

    fn read_byte(&mut self) -> Result<u8, SessionError> {
        if self.pos >= self.data.len() {
            return Err(SessionError::Pickle("unexpected EOF".into()));
        }
        let b = self.data[self.pos];
        self.pos += 1;
        Ok(b)
    }

    fn read_bytes(&mut self, n: usize) -> Result<&'a [u8], SessionError> {
        if self.pos + n > self.data.len() {
            return Err(SessionError::Pickle("unexpected EOF".into()));
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn read_u16_le(&mut self) -> Result<u16, SessionError> {
        let b = self.read_bytes(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn read_u32_le(&mut self) -> Result<u32, SessionError> {
        let b = self.read_bytes(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn read_i32_le(&mut self) -> Result<i32, SessionError> {
        let b = self.read_bytes(4)?;
        Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn read_f64_be(&mut self) -> Result<f64, SessionError> {
        let b = self.read_bytes(8)?;
        Ok(f64::from_be_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    fn read_line(&mut self) -> Result<&'a [u8], SessionError> {
        let start = self.pos;
        while self.pos < self.data.len() && self.data[self.pos] != b'\n' {
            self.pos += 1;
        }
        if self.pos >= self.data.len() {
            return Err(SessionError::Pickle("unexpected EOF reading line".into()));
        }
        let line = &self.data[start..self.pos];
        self.pos += 1; // skip newline
        Ok(line)
    }

    fn push(&mut self, v: PickleValue) {
        self.stack.push(StackItem::Value(v));
    }

    fn pop(&mut self) -> Result<PickleValue, SessionError> {
        self.stack
            .pop()
            .ok_or_else(|| SessionError::Pickle("stack underflow".into()))?
            .into_value()
    }

    fn top(&self) -> Result<&PickleValue, SessionError> {
        match self.stack.last() {
            Some(StackItem::Value(v)) => Ok(v),
            Some(StackItem::Mark) => Err(SessionError::Pickle("mark at top of stack".into())),
            None => Err(SessionError::Pickle("stack underflow".into())),
        }
    }

    fn top_mut(&mut self) -> Result<&mut PickleValue, SessionError> {
        match self.stack.last_mut() {
            Some(StackItem::Value(v)) => Ok(v),
            Some(StackItem::Mark) => Err(SessionError::Pickle("mark at top of stack".into())),
            None => Err(SessionError::Pickle("stack underflow".into())),
        }
    }

    /// Pop items above the topmost mark, return them as Vec<PickleValue>.
    fn pop_mark(&mut self) -> Result<Vec<PickleValue>, SessionError> {
        let mut items = Vec::new();
        loop {
            match self.stack.pop() {
                Some(StackItem::Mark) => {
                    items.reverse();
                    return Ok(items);
                }
                Some(StackItem::Value(v)) => items.push(v),
                None => return Err(SessionError::Pickle("mark not found".into())),
            }
        }
    }

    fn run(&mut self) -> Result<PickleValue, SessionError> {
        loop {
            let opcode = self.read_byte()?;
            match opcode {
                // PROTO
                0x80 => {
                    let _version = self.read_byte()?;
                }
                // FRAME (protocol 4+) — skip 8 bytes
                0x95 => {
                    self.read_bytes(8)?;
                }
                // STOP
                0x2E => {
                    return self.pop();
                }
                // MARK
                0x28 => {
                    self.stack.push(StackItem::Mark);
                }
                // POP
                0x30 => {
                    self.stack.pop();
                }
                // POP_MARK
                0x31 => {
                    self.pop_mark()?;
                }
                // NONE
                0x4E => self.push(PickleValue::None),
                // NEWTRUE
                0x88 => self.push(PickleValue::Bool(true)),
                // NEWFALSE
                0x89 => self.push(PickleValue::Bool(false)),
                // INT — text line
                0x49 => {
                    let line = self.read_line()?;
                    let s = std::str::from_utf8(line)
                        .map_err(|e| SessionError::Pickle(format!("invalid INT line: {e}")))?;
                    let s = s.trim();
                    if s == "00" {
                        self.push(PickleValue::Bool(false));
                    } else if s == "01" {
                        self.push(PickleValue::Bool(true));
                    } else {
                        let v: i64 = s
                            .parse()
                            .map_err(|e| SessionError::Pickle(format!("invalid INT: {e}")))?;
                        self.push(PickleValue::Int(v));
                    }
                }
                // LONG — text line ending with 'L'
                0x4C => {
                    let line = self.read_line()?;
                    let s = std::str::from_utf8(line)
                        .map_err(|e| SessionError::Pickle(format!("invalid LONG line: {e}")))?;
                    let s = s.trim().trim_end_matches('L');
                    let v: i64 = s
                        .parse()
                        .map_err(|e| SessionError::Pickle(format!("invalid LONG: {e}")))?;
                    self.push(PickleValue::Int(v));
                }
                // FLOAT — text line
                0x46 => {
                    let line = self.read_line()?;
                    let s = std::str::from_utf8(line)
                        .map_err(|e| SessionError::Pickle(format!("invalid FLOAT line: {e}")))?;
                    let v: f64 = s
                        .trim()
                        .parse()
                        .map_err(|e| SessionError::Pickle(format!("invalid FLOAT: {e}")))?;
                    self.push(PickleValue::Float(v));
                }
                // BININT
                0x4A => {
                    let v = self.read_i32_le()?;
                    self.push(PickleValue::Int(v as i64));
                }
                // BININT1
                0x4B => {
                    let v = self.read_byte()?;
                    self.push(PickleValue::Int(v as i64));
                }
                // BININT2
                0x4D => {
                    let v = self.read_u16_le()?;
                    self.push(PickleValue::Int(v as i64));
                }
                // BINFLOAT
                0x47 => {
                    let v = self.read_f64_be()?;
                    self.push(PickleValue::Float(v));
                }
                // STRING — quoted text line
                0x53 => {
                    let line = self.read_line()?;
                    let s = std::str::from_utf8(line)
                        .map_err(|e| SessionError::Pickle(format!("invalid STRING: {e}")))?;
                    let s = s.trim();
                    // Strip surrounding quotes
                    let inner = if (s.starts_with('\'') && s.ends_with('\''))
                        || (s.starts_with('"') && s.ends_with('"'))
                    {
                        &s[1..s.len() - 1]
                    } else {
                        s
                    };
                    self.push(PickleValue::String(inner.to_string()));
                }
                // SHORT_BINSTRING
                0x55 => {
                    let len = self.read_byte()? as usize;
                    let b = self.read_bytes(len)?;
                    let s = String::from_utf8_lossy(b).into_owned();
                    self.push(PickleValue::String(s));
                }
                // BINSTRING
                0x54 => {
                    let len = self.read_i32_le()? as usize;
                    let b = self.read_bytes(len)?;
                    let s = String::from_utf8_lossy(b).into_owned();
                    self.push(PickleValue::String(s));
                }
                // BINUNICODE
                0x58 => {
                    let len = self.read_u32_le()? as usize;
                    let b = self.read_bytes(len)?;
                    let s = String::from_utf8_lossy(b).into_owned();
                    self.push(PickleValue::String(s));
                }
                // SHORT_BINUNICODE
                0x8C => {
                    let len = self.read_byte()? as usize;
                    let b = self.read_bytes(len)?;
                    let s = String::from_utf8_lossy(b).into_owned();
                    self.push(PickleValue::String(s));
                }
                // BINBYTES
                0x42 => {
                    let len = self.read_u32_le()? as usize;
                    let b = self.read_bytes(len)?;
                    self.push(PickleValue::Bytes(b.to_vec()));
                }
                // SHORT_BINBYTES
                0x43 => {
                    let len = self.read_byte()? as usize;
                    let b = self.read_bytes(len)?;
                    self.push(PickleValue::Bytes(b.to_vec()));
                }
                // EMPTY_LIST
                0x5D => self.push(PickleValue::List(Vec::new())),
                // APPEND
                0x61 => {
                    let val = self.pop()?;
                    let top = self.top_mut()?;
                    if let PickleValue::List(ref mut list) = top {
                        list.push(val);
                    } else {
                        return Err(SessionError::Pickle("APPEND on non-list".into()));
                    }
                }
                // APPENDS
                0x65 => {
                    let items = self.pop_mark()?;
                    let top = self.top_mut()?;
                    if let PickleValue::List(ref mut list) = top {
                        list.extend(items);
                    } else {
                        return Err(SessionError::Pickle("APPENDS on non-list".into()));
                    }
                }
                // LIST
                0x6C => {
                    let items = self.pop_mark()?;
                    self.push(PickleValue::List(items));
                }
                // EMPTY_DICT
                0x7D => self.push(PickleValue::Dict(Vec::new())),
                // SETITEM
                0x73 => {
                    let val = self.pop()?;
                    let key = self.pop()?;
                    let top = self.top_mut()?;
                    if let PickleValue::Dict(ref mut dict) = top {
                        dict.push((key, val));
                    } else {
                        return Err(SessionError::Pickle("SETITEM on non-dict".into()));
                    }
                }
                // SETITEMS
                0x75 => {
                    let items = self.pop_mark()?;
                    if items.len() % 2 != 0 {
                        return Err(SessionError::Pickle("SETITEMS odd item count".into()));
                    }
                    let top = self.top_mut()?;
                    if let PickleValue::Dict(ref mut dict) = top {
                        let mut iter = items.into_iter();
                        while let (Some(k), Some(v)) = (iter.next(), iter.next()) {
                            dict.push((k, v));
                        }
                    } else {
                        return Err(SessionError::Pickle("SETITEMS on non-dict".into()));
                    }
                }
                // DICT
                0x64 => {
                    let items = self.pop_mark()?;
                    if items.len() % 2 != 0 {
                        return Err(SessionError::Pickle("DICT odd item count".into()));
                    }
                    let mut dict = Vec::new();
                    let mut iter = items.into_iter();
                    while let (Some(k), Some(v)) = (iter.next(), iter.next()) {
                        dict.push((k, v));
                    }
                    self.push(PickleValue::Dict(dict));
                }
                // EMPTY_TUPLE
                0x29 => self.push(PickleValue::Tuple(Vec::new())),
                // TUPLE
                0x74 => {
                    let items = self.pop_mark()?;
                    self.push(PickleValue::Tuple(items));
                }
                // TUPLE1
                0x85 => {
                    let a = self.pop()?;
                    self.push(PickleValue::Tuple(vec![a]));
                }
                // TUPLE2
                0x86 => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    self.push(PickleValue::Tuple(vec![a, b]));
                }
                // TUPLE3
                0x87 => {
                    let c = self.pop()?;
                    let b = self.pop()?;
                    let a = self.pop()?;
                    self.push(PickleValue::Tuple(vec![a, b, c]));
                }
                // PUT (text)
                0x70 => {
                    let line = self.read_line()?;
                    let s = std::str::from_utf8(line)
                        .map_err(|e| SessionError::Pickle(format!("invalid PUT index: {e}")))?;
                    let idx: u32 = s
                        .trim()
                        .parse()
                        .map_err(|e| SessionError::Pickle(format!("invalid PUT index: {e}")))?;
                    let val = self.top()?.clone();
                    self.memo.insert(idx, val);
                }
                // BINPUT
                0x71 => {
                    let idx = self.read_byte()? as u32;
                    let val = self.top()?.clone();
                    self.memo.insert(idx, val);
                }
                // LONG_BINPUT
                0x72 => {
                    let idx = self.read_u32_le()?;
                    let val = self.top()?.clone();
                    self.memo.insert(idx, val);
                }
                // GET (text)
                0x67 => {
                    let line = self.read_line()?;
                    let s = std::str::from_utf8(line)
                        .map_err(|e| SessionError::Pickle(format!("invalid GET index: {e}")))?;
                    let idx: u32 = s
                        .trim()
                        .parse()
                        .map_err(|e| SessionError::Pickle(format!("invalid GET index: {e}")))?;
                    let val = self
                        .memo
                        .get(&idx)
                        .ok_or_else(|| SessionError::Pickle(format!("memo key {idx} not found")))?
                        .clone();
                    self.push(val);
                }
                // BINGET
                0x68 => {
                    let idx = self.read_byte()? as u32;
                    let val = self
                        .memo
                        .get(&idx)
                        .ok_or_else(|| SessionError::Pickle(format!("memo key {idx} not found")))?
                        .clone();
                    self.push(val);
                }
                // LONG_BINGET
                0x6A => {
                    let idx = self.read_u32_le()?;
                    let val = self
                        .memo
                        .get(&idx)
                        .ok_or_else(|| SessionError::Pickle(format!("memo key {idx} not found")))?
                        .clone();
                    self.push(val);
                }
                // MEMOIZE (protocol 4)
                0x94 => {
                    let idx = self.memo_counter;
                    self.memo_counter += 1;
                    let val = self.top()?.clone();
                    self.memo.insert(idx, val);
                }
                // GLOBAL
                0x63 => {
                    let _module = self.read_line()?;
                    let _name = self.read_line()?;
                    // Push a placeholder — we don't instantiate Python classes
                    self.push(PickleValue::String("<global>".into()));
                }
                // REDUCE
                0x52 => {
                    let args = self.pop()?;
                    let _callable = self.pop()?;
                    // Return args (typically a tuple) as best-effort
                    self.push(args);
                }
                // BUILD
                0x62 => {
                    let _state = self.pop()?;
                    // no-op: the object being built is already on the stack
                }
                // LONG1
                0x8A => {
                    let n = self.read_byte()? as usize;
                    if n == 0 {
                        self.push(PickleValue::Int(0));
                    } else {
                        let bytes = self.read_bytes(n)?;
                        let val = long_from_le_bytes(bytes);
                        self.push(PickleValue::Int(val));
                    }
                }
                // LONG4
                0x8B => {
                    let n = self.read_i32_le()? as usize;
                    if n == 0 {
                        self.push(PickleValue::Int(0));
                    } else {
                        let bytes = self.read_bytes(n)?;
                        let val = long_from_le_bytes(bytes);
                        self.push(PickleValue::Int(val));
                    }
                }
                // BINPERSID
                0x51 => {
                    let _pid = self.pop()?;
                    self.push(PickleValue::None);
                }
                // DUP
                0x32 => {
                    let val = self.top()?.clone();
                    self.push(val);
                }
                other => {
                    return Err(SessionError::Pickle(format!(
                        "unsupported opcode 0x{other:02X} at position {}",
                        self.pos - 1
                    )));
                }
            }
        }
    }
}

/// Convert little-endian signed bytes to i64 (two's complement, up to 8 bytes).
fn long_from_le_bytes(bytes: &[u8]) -> i64 {
    let mut val: i64 = 0;
    for (i, &b) in bytes.iter().enumerate().take(8) {
        val |= (b as i64) << (i * 8);
    }
    // Sign-extend if highest bit set
    if !bytes.is_empty() && (bytes[bytes.len().min(8) - 1] & 0x80) != 0 {
        let used = bytes.len().min(8);
        for i in used..8 {
            val |= 0xFFi64 << (i * 8);
        }
    }
    val
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pickle::PickleValue;

    #[test]
    fn test_empty_dict() {
        // Protocol 2: \x80\x02 }q\x00. => empty dict
        let data = b"\x80\x02}q\x00.";
        let val = read(data).unwrap();
        assert_eq!(val, PickleValue::Dict(vec![]));
    }

    #[test]
    fn integer_opcodes_preserve_values_and_signedness() {
        for (name, bytes, expected) in [
            ("INT", b"I42\n.".as_slice(), 42),
            ("BININT1", b"\x4b\xff.", 255),
            ("LONG1 positive", b"\x8a\x02\x00\x01.", 256),
            ("LONG1 negative", b"\x8a\x01\xff.", -1),
        ] {
            assert_eq!(read(bytes).unwrap(), PickleValue::Int(expected), "{name}");
        }
    }

    #[test]
    fn test_bool_int() {
        let data = b"I00\n.";
        let val = read(data).unwrap();
        assert_eq!(val, PickleValue::Bool(false));

        let data = b"I01\n.";
        let val = read(data).unwrap();
        assert_eq!(val, PickleValue::Bool(true));
    }

    #[test]
    fn test_binfloat() {
        // BINFLOAT: 0x47 + 8 bytes big-endian f64 for 1.5
        let mut data = vec![0x47u8];
        data.extend_from_slice(&1.5f64.to_be_bytes());
        data.push(0x2E); // STOP
        let val = read(&data).unwrap();
        assert_eq!(val, PickleValue::Float(1.5));
    }

    #[test]
    fn test_short_binunicode() {
        // SHORT_BINUNICODE: 0x8C, len, bytes, STOP
        let s = b"hello";
        let mut data = vec![0x8Cu8, s.len() as u8];
        data.extend_from_slice(s);
        data.push(0x2E);
        let val = read(&data).unwrap();
        assert_eq!(val, PickleValue::String("hello".into()));
    }

    #[test]
    fn test_list_with_appends() {
        // ] ( K\x01 K\x02 K\x03 e .
        // EMPTY_LIST, MARK, BININT1(1), BININT1(2), BININT1(3), APPENDS, STOP
        let data = [0x5D, 0x28, 0x4B, 0x01, 0x4B, 0x02, 0x4B, 0x03, 0x65, 0x2E];
        let val = read(&data).unwrap();
        assert_eq!(
            val,
            PickleValue::List(vec![
                PickleValue::Int(1),
                PickleValue::Int(2),
                PickleValue::Int(3),
            ])
        );
    }

    #[test]
    fn tuple_opcodes_preserve_arity_and_order() {
        for (name, bytes, expected) in [
            ("EMPTY_TUPLE", b").".as_slice(), vec![]),
            ("TUPLE1", b"K\x05\x85.", vec![5]),
            ("TUPLE2", b"K\x01K\x02\x86.", vec![1, 2]),
            ("TUPLE3", b"K\x01K\x02K\x03\x87.", vec![1, 2, 3]),
        ] {
            assert_eq!(
                read(bytes).unwrap(),
                PickleValue::Tuple(expected.into_iter().map(PickleValue::Int).collect()),
                "{name}"
            );
        }
    }

    #[test]
    fn test_dict_with_setitems() {
        // } ( 0x8C 0x01 'a' K\x01 0x8C 0x01 'b' K\x02 u .
        let data = [
            0x7D, // EMPTY_DICT
            0x28, // MARK
            0x8C, 0x01, b'a', // SHORT_BINUNICODE "a"
            0x4B, 0x01, // BININT1 1
            0x8C, 0x01, b'b', // SHORT_BINUNICODE "b"
            0x4B, 0x02, // BININT1 2
            0x75, // SETITEMS
            0x2E, // STOP
        ];
        let val = read(&data).unwrap();
        assert_eq!(
            val,
            PickleValue::Dict(vec![
                (PickleValue::String("a".into()), PickleValue::Int(1)),
                (PickleValue::String("b".into()), PickleValue::Int(2)),
            ])
        );
    }

    #[test]
    fn test_none() {
        let data = [0x4E, 0x2E]; // NONE, STOP
        let val = read(&data).unwrap();
        assert_eq!(val, PickleValue::None);
    }

    #[test]
    fn test_newtrue_newfalse() {
        let data = [0x88, 0x2E]; // NEWTRUE, STOP
        assert_eq!(read(&data).unwrap(), PickleValue::Bool(true));
        let data = [0x89, 0x2E]; // NEWFALSE, STOP
        assert_eq!(read(&data).unwrap(), PickleValue::Bool(false));
    }

    #[test]
    fn test_memo_put_get() {
        // BININT1(42), BINPUT(0), POP, BINGET(0), STOP
        let data = [0x4B, 42, 0x71, 0x00, 0x30, 0x68, 0x00, 0x2E];
        let val = read(&data).unwrap();
        assert_eq!(val, PickleValue::Int(42));
    }

    #[test]
    fn test_pickle_value_accessors() {
        let v = PickleValue::Int(42);
        assert_eq!(v.as_int(), Some(42));
        assert_eq!(v.as_float(), Some(42.0));
        assert_eq!(v.as_str(), None);

        let v = PickleValue::String("hello".into());
        assert_eq!(v.as_str(), Some("hello"));
        assert_eq!(v.as_int(), None);

        let v = PickleValue::Dict(vec![(
            PickleValue::String("key".into()),
            PickleValue::Int(1),
        )]);
        assert_eq!(v.get("key"), Some(&PickleValue::Int(1)));
        assert_eq!(v.get("missing"), None);
    }

    #[test]
    #[allow(clippy::approx_constant)]
    fn test_float_text() {
        let data = b"F3.14\n.";
        let val = read(data).unwrap();
        assert_eq!(val, PickleValue::Float(3.14));
    }

    #[test]
    fn test_short_binbytes() {
        let data = [0x43, 0x03, 0xDE, 0xAD, 0xBE, 0x2E];
        let val = read(&data).unwrap();
        assert_eq!(val, PickleValue::Bytes(vec![0xDE, 0xAD, 0xBE]));
    }
}
