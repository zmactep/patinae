//! Deserialize the migrated PRS value tree without a second MessagePack parser.
//!
//! PRS staging already owns and validates this tree. Borrowing it keeps the
//! MessagePack parsing code independent of the number of session schema types.

use rmp_serde::decode::Error;
use rmpv::Value;
use serde::de::Error as _;
use serde::de::{self, DeserializeSeed, IntoDeserializer, Visitor};
use serde::Deserializer as _;

#[derive(Clone, Copy)]
pub(super) struct Deserializer<'de>(pub(super) &'de Value);

impl<'de> IntoDeserializer<'de, Error> for Deserializer<'de> {
    type Deserializer = Self;

    fn into_deserializer(self) -> Self {
        self
    }
}

impl<'de> de::Deserializer<'de> for Deserializer<'de> {
    type Error = Error;

    fn is_human_readable(&self) -> bool {
        false
    }

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.0 {
            Value::Nil => visitor.visit_unit(),
            Value::Boolean(value) => visitor.visit_bool(*value),
            Value::Integer(value) => {
                if let Some(value) = value.as_u64() {
                    visitor.visit_u64(value)
                } else {
                    visitor.visit_i64(value.as_i64().expect("negative MessagePack integer"))
                }
            }
            Value::F32(value) => visitor.visit_f32(*value),
            Value::F64(value) => visitor.visit_f64(*value),
            Value::String(value) => match std::str::from_utf8(value.as_bytes()) {
                Ok(value) => visitor.visit_borrowed_str(value),
                Err(error) => visitor
                    .visit_borrowed_bytes(value.as_bytes())
                    .map_err(|_: Error| Error::Utf8Error(error)),
            },
            Value::Binary(value) => visitor.visit_borrowed_bytes(value),
            Value::Array(values) => {
                let mut seq = de::value::SeqDeserializer::new(values.iter().map(Deserializer));
                let value = visitor.visit_seq(&mut seq)?;
                seq.end()?;
                Ok(value)
            }
            Value::Map(values) => {
                let mut map = de::value::MapDeserializer::new(
                    values
                        .iter()
                        .map(|(key, value)| (Deserializer(key), Deserializer(value))),
                );
                let value = visitor.visit_map(&mut map)?;
                map.end()?;
                Ok(value)
            }
            Value::Ext(tag, bytes) => visitor.visit_newtype_struct(extension(*tag, bytes)),
        }
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        if self.0.is_nil() {
            visitor.visit_none()
        } else {
            visitor.visit_some(self)
        }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Error> {
        if name == rmp_serde::MSGPACK_EXT_STRUCT_NAME {
            return match self.0 {
                Value::Ext(tag, bytes) => visitor.visit_newtype_struct(extension(*tag, bytes)),
                _ => Err(Error::custom("expected MessagePack extension")),
            };
        }
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Error> {
        if matches!(self.0, Value::Array(values) if values.is_empty()) {
            visitor.visit_unit()
        } else {
            self.deserialize_any(visitor)
        }
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        match self.0 {
            Value::Map(values) => match values.as_slice() {
                [(key, value)] => visitor.visit_enum(EnumAccess {
                    key: Deserializer(key),
                    value: Some(Deserializer(value)),
                }),
                _ => Err(Error::custom("expected a single-entry enum map")),
            },
            _ => visitor.visit_enum(EnumAccess {
                key: self,
                value: None,
            }),
        }
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        // rmp-serde also accepts MessagePack binary values for Vec<u8> and tuples.
        if let Value::Binary(bytes) = self.0 {
            visitor.visit_seq(de::value::SeqDeserializer::new(bytes.iter().copied()))
        } else {
            self.deserialize_any(visitor)
        }
    }

    fn deserialize_tuple<V: Visitor<'de>>(
        self,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, Error> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, Error> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_i128<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_i128(self.integer128()?)
    }

    fn deserialize_u128<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_u128(self.integer128()? as u128)
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        // The binary parser has already validated and consumed the entire value.
        visitor.visit_unit()
    }

    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.number(visitor)
    }

    fn deserialize_i8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.number(visitor)
    }

    fn deserialize_i16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.number(visitor)
    }

    fn deserialize_i32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.number(visitor)
    }

    fn deserialize_i64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.number(visitor)
    }

    fn deserialize_u8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.number(visitor)
    }

    fn deserialize_u16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.number(visitor)
    }

    fn deserialize_u32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.number(visitor)
    }

    fn deserialize_u64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.number(visitor)
    }

    fn deserialize_f32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.number(visitor)
    }

    fn deserialize_f64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.number(visitor)
    }

    serde::forward_to_deserialize_any! {
        char str string bytes byte_buf unit map identifier
    }
}

impl<'de> Deserializer<'de> {
    fn number<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.0 {
            Value::Nil => visitor.visit_unit(),
            Value::Boolean(value) => visitor.visit_bool(*value),
            Value::Integer(value) => {
                if let Some(value) = value.as_u64() {
                    visitor.visit_u64(value)
                } else {
                    visitor.visit_i64(value.as_i64().expect("negative MessagePack integer"))
                }
            }
            Value::F32(value) => visitor.visit_f32(*value),
            Value::F64(value) => visitor.visit_f64(*value),
            _ => Err(Error::custom("expected a MessagePack scalar")),
        }
    }

    fn integer128(self) -> Result<i128, Error> {
        match self.0 {
            Value::Integer(value) => value
                .as_u64()
                .map(i128::from)
                .or_else(|| value.as_i64().map(i128::from))
                .ok_or_else(|| Error::custom("invalid MessagePack integer")),
            Value::Binary(bytes) => {
                let bytes: [u8; 16] = bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_| Error::LengthMismatch(16))?;
                Ok(i128::from_be_bytes(bytes))
            }
            _ => Err(Error::custom("expected integer or 16-byte binary value")),
        }
    }
}

struct EnumAccess<'de> {
    key: Deserializer<'de>,
    value: Option<Deserializer<'de>>,
}

impl<'de> de::EnumAccess<'de> for EnumAccess<'de> {
    type Error = Error;
    type Variant = Self;

    fn variant_seed<V: DeserializeSeed<'de>>(self, seed: V) -> Result<(V::Value, Self), Error> {
        Ok((seed.deserialize(self.key)?, self))
    }
}

impl<'de> de::VariantAccess<'de> for EnumAccess<'de> {
    type Error = Error;

    fn unit_variant(self) -> Result<(), Error> {
        match self.value {
            None | Some(Deserializer(Value::Nil)) => Ok(()),
            Some(_) => Err(Error::custom("expected nil for unit variant")),
        }
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, Error> {
        seed.deserialize(
            self.value
                .ok_or_else(|| Error::custom("expected variant payload"))?,
        )
    }

    fn tuple_variant<V: Visitor<'de>>(self, len: usize, visitor: V) -> Result<V::Value, Error> {
        self.value
            .ok_or_else(|| Error::custom("expected tuple variant payload"))?
            .deserialize_tuple(len, visitor)
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        self.tuple_variant(fields.len(), visitor)
    }
}

// MessagePack extensions are represented to Serde as a newtype containing
// a two-element sequence (signed tag, borrowed byte payload).
fn extension(tag: i8, bytes: &[u8]) -> impl de::Deserializer<'_, Error = Error> {
    de::value::SeqDeserializer::new(
        [ExtensionPart::Tag(tag), ExtensionPart::Bytes(bytes)].into_iter(),
    )
}

enum ExtensionPart<'de> {
    Tag(i8),
    Bytes(&'de [u8]),
}

impl<'de> IntoDeserializer<'de, Error> for ExtensionPart<'de> {
    type Deserializer = Self;

    fn into_deserializer(self) -> Self {
        self
    }
}

impl<'de> de::Deserializer<'de> for ExtensionPart<'de> {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self {
            Self::Tag(tag) => visitor.visit_i8(tag),
            Self::Bytes(bytes) => visitor.visit_borrowed_bytes(bytes),
        }
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple tuple_struct
        map struct enum identifier ignored_any
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;
    use std::fmt::Debug;

    fn compare<T>(value: &Value)
    where
        T: de::DeserializeOwned + PartialEq + Debug,
    {
        let mut bytes = Vec::new();
        rmpv::encode::write_value(&mut bytes, value).unwrap();
        let expected = rmp_serde::from_slice::<T>(&bytes);
        let actual = T::deserialize(Deserializer(value));
        match (expected, actual) {
            (Ok(expected), Ok(actual)) => assert_eq!(actual, expected, "input: {value:?}"),
            (Err(_), Err(_)) => {}
            (expected, actual) => {
                panic!("input: {value:?}; binary: {expected:?}; tree: {actual:?}")
            }
        }
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    enum Choice {
        Unit,
        Newtype(String),
        Tuple(i32, bool),
        Struct { values: Vec<Option<u64>> },
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Record {
        name: String,
        choice: Choice,
        values: Vec<Option<i64>>,
    }

    #[test]
    fn named_and_positional_structs_and_all_enum_shapes_match_binary_decoder() {
        for choice in [
            Choice::Unit,
            Choice::Newtype("text".into()),
            Choice::Tuple(-7, true),
            Choice::Struct {
                values: vec![None, Some(u64::MAX)],
            },
        ] {
            let record = Record {
                name: "molecule".into(),
                choice,
                values: vec![None, Some(-1), Some(i64::MAX)],
            };
            for bytes in [
                rmp_serde::to_vec(&record).unwrap(),
                rmp_serde::to_vec_named(&record).unwrap(),
            ] {
                let value = rmpv::decode::read_value(&mut bytes.as_slice()).unwrap();
                compare::<Record>(&value);
            }
        }
    }

    #[test]
    fn scalar_ranges_containers_and_rejected_types_match_binary_decoder() {
        let mut values = vec![
            Value::Nil,
            Value::Boolean(true),
            Value::from("Unit"),
            Value::from("text"),
            Value::F32(1.5),
            Value::F64(-2.5),
            Value::Binary(vec![0, 255]),
            Value::Array(vec![]),
            Value::Array(vec![Value::from(1), Value::from(2)]),
            Value::Map(vec![]),
            Value::Map(vec![(Value::from("k"), Value::from(1))]),
            Value::Map(vec![(Value::from("Unit"), Value::Nil)]),
            Value::Map(vec![(Value::from("Unit"), Value::from(1))]),
            Value::Map(vec![
                (Value::from("Unit"), Value::Nil),
                (Value::from("k"), Value::Nil),
            ]),
            Value::Ext(7, vec![1, 2]),
        ];
        for value in [i64::MIN, -129, -128, -1, 0, 127, 128, 255, 256, i64::MAX] {
            values.push(Value::from(value));
        }
        values.push(Value::from(u64::MAX));
        for value in values {
            compare::<bool>(&value);
            compare::<i8>(&value);
            compare::<i64>(&value);
            compare::<u8>(&value);
            compare::<u64>(&value);
            compare::<i128>(&value);
            compare::<u128>(&value);
            compare::<String>(&value);
            compare::<Option<u64>>(&value);
            compare::<Vec<u8>>(&value);
            compare::<[u8; 2]>(&value);
            compare::<BTreeMap<String, u64>>(&value);
            compare::<Choice>(&value);
        }
    }

    #[test]
    fn wide_integers_and_borrowed_bytes_match_binary_decoder() {
        for value in [i128::MIN, -1, 0, i128::MAX] {
            let bytes = rmp_serde::to_vec(&value).unwrap();
            let tree = rmpv::decode::read_value(&mut bytes.as_slice()).unwrap();
            compare::<i128>(&tree);
            compare::<u128>(&tree);
        }
        let value = Value::Binary(vec![0, 128, 255]);
        let decoded = <&[u8]>::deserialize(Deserializer(&value)).unwrap();
        assert_eq!(decoded, &[0, 128, 255]);
        let invalid_utf8 = rmpv::decode::read_value(&mut [0xa1, 0xff].as_slice()).unwrap();
        compare::<String>(&invalid_utf8);
        assert_eq!(
            <&[u8]>::deserialize(Deserializer(&invalid_utf8)).unwrap(),
            &[255]
        );
    }

    #[test]
    fn extensions_unit_structs_and_unknown_fields_remain_supported() {
        #[derive(Debug, PartialEq, Serialize, Deserialize)]
        struct Unit;
        compare::<Unit>(&Value::Nil);
        compare::<Unit>(&Value::Array(vec![]));
        compare::<Unit>(&Value::Array(vec![Value::Nil]));

        #[derive(Debug, PartialEq, Deserialize)]
        #[serde(rename = "_ExtStruct")]
        struct Ext<'a>(#[serde(borrow)] (i8, &'a [u8]));
        let value = Value::Ext(-7, vec![1, 2, 3]);
        let mut bytes = Vec::new();
        rmpv::encode::write_value(&mut bytes, &value).unwrap();
        assert_eq!(
            Ext::deserialize(Deserializer(&value)).unwrap(),
            rmp_serde::from_slice::<Ext>(&bytes).unwrap()
        );

        #[derive(Debug, PartialEq, Deserialize)]
        struct Known {
            number: u64,
        }
        compare::<Known>(&Value::Map(vec![
            (Value::from("number"), Value::from(42)),
            (Value::from("unknown"), Value::Array(vec![value])),
        ]));
    }
}
