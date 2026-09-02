// Copyright 2023 Datafuse Labs.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::borrow::Cow;
use std::collections::VecDeque;

use byteorder::BigEndian;
use byteorder::WriteBytesExt;
use serde::ser;
use serde::ser::Serialize;
use serde::ser::SerializeMap;
use serde::ser::SerializeSeq;

use super::constants::*;
use super::jentry::JEntry;
use crate::core::ArrayBuilder;
use crate::core::ObjectBuilder;
use crate::error::*;
use crate::extension::ExtensionValue;
use crate::number::Number;
use crate::parser::JsonAst;
use crate::value::Object;
use crate::value::Value;
use crate::Error;
use crate::OwnedJsonb;
use crate::RawJsonb;

use crate::Decimal128;
use crate::Decimal256;
use crate::Decimal64;
use ethnum::i256;

use crate::constants::NUMBER_STRUCT_FIELD_HIGH_VALUE;
use crate::constants::NUMBER_STRUCT_FIELD_LOW_VALUE;
use crate::constants::NUMBER_STRUCT_FIELD_SCALE;
use crate::constants::NUMBER_STRUCT_FIELD_VALUE;
use crate::constants::NUMBER_STRUCT_TOKEN;

use crate::constants::DECIMAL128_MAX;
use crate::constants::DECIMAL128_MIN;
use crate::constants::DECIMAL64_MAX;
use crate::constants::DECIMAL64_MIN;

use crate::constants::MAX_DECIMAL128_PRECISION;
use crate::constants::MAX_DECIMAL256_PRECISION;
use crate::constants::MAX_DECIMAL64_PRECISION;

/// `Serializer` is a custom serializer for JSONB data, implementing the
/// `serde::ser::Serializer` trait. It allows serializing Rust data structures
/// into a `Vec<u8>` representing the JSONB data.
#[derive(Debug, Default)]
pub struct Serializer {
    buffer: Vec<u8>,
}

impl Serializer {
    /// Creates a new `Serializer` with an empty buffer.
    pub fn new() -> Serializer {
        Serializer { buffer: Vec::new() }
    }

    /// Consumes the `Serializer` and returns the underlying buffer containing the
    /// serialized JSONB data.
    ///
    /// This function returns the `OwnedJsonb` that has been populated with the
    /// serialized JSONB data during the serialization process. The `Serializer`
    /// is consumed in the process.
    pub fn to_owned_jsonb(self) -> OwnedJsonb {
        OwnedJsonb::new(self.buffer)
    }

    fn write_jentry(&mut self, jentry: JEntry) -> Result<()> {
        self.buffer.write_u32::<BigEndian>(SCALAR_CONTAINER_TAG)?;
        let jentry_bytes = jentry.encoded();
        self.buffer.write_u32::<BigEndian>(jentry_bytes)?;
        Ok(())
    }

    fn replace_jentry(&mut self, jentry: JEntry, jentry_index: &mut usize) {
        let jentry_bytes = jentry.encoded().to_be_bytes();
        for (i, b) in jentry_bytes.iter().enumerate() {
            self.buffer[*jentry_index + i] = *b;
        }
        *jentry_index += 4;
    }

    fn write_number(&mut self, v: Number) -> Result<()> {
        self.buffer.write_u32::<BigEndian>(SCALAR_CONTAINER_TAG)?;
        let mut jentry_index = self.buffer.len();
        let payload_index = jentry_index + 4;
        // resize buffer to keep space for jentry.
        self.buffer.resize(payload_index, 0);

        let _ = v.compact_encode(&mut self.buffer)?;
        let len = self.buffer.len() - payload_index;
        let jentry = JEntry::make_number_jentry(len);

        self.replace_jentry(jentry, &mut jentry_index);
        Ok(())
    }

    fn write_str(&mut self, s: &str) -> Result<()> {
        self.buffer.write_u32::<BigEndian>(SCALAR_CONTAINER_TAG)?;
        let mut jentry_index = self.buffer.len();
        let payload_index = jentry_index + 4;
        // resize buffer to keep space for jentry.
        self.buffer.resize(payload_index, 0);

        let len = s.len();
        self.buffer.extend_from_slice(s.as_bytes());
        let jentry = JEntry::make_string_jentry(len);

        self.replace_jentry(jentry, &mut jentry_index);
        Ok(())
    }
}

impl<'a> ser::Serializer for &'a mut Serializer {
    type Ok = ();

    type Error = Error;

    type SerializeSeq = ArraySerializer<'a>;

    type SerializeTuple = ArraySerializer<'a>;

    type SerializeTupleStruct = ArraySerializer<'a>;

    type SerializeTupleVariant = ArraySerializer<'a>;

    type SerializeMap = ObjectSerializer<'a>;

    type SerializeStruct = StructSerializer<'a>;

    type SerializeStructVariant = StructSerializer<'a>;

    fn serialize_bool(self, v: bool) -> Result<Self::Ok> {
        let jentry = if v {
            JEntry::make_true_jentry()
        } else {
            JEntry::make_false_jentry()
        };
        self.write_jentry(jentry)
    }

    fn serialize_i8(self, v: i8) -> Result<Self::Ok> {
        self.write_number(Number::Int64(v as i64))
    }

    fn serialize_i16(self, v: i16) -> Result<Self::Ok> {
        self.write_number(Number::Int64(v as i64))
    }

    fn serialize_i32(self, v: i32) -> Result<Self::Ok> {
        self.write_number(Number::Int64(v as i64))
    }

    fn serialize_i64(self, v: i64) -> Result<Self::Ok> {
        self.write_number(Number::Int64(v))
    }

    fn serialize_u8(self, v: u8) -> Result<Self::Ok> {
        self.write_number(Number::UInt64(v as u64))
    }

    fn serialize_u16(self, v: u16) -> Result<Self::Ok> {
        self.write_number(Number::UInt64(v as u64))
    }

    fn serialize_u32(self, v: u32) -> Result<Self::Ok> {
        self.write_number(Number::UInt64(v as u64))
    }

    fn serialize_u64(self, v: u64) -> Result<Self::Ok> {
        self.write_number(Number::UInt64(v))
    }

    fn serialize_f32(self, v: f32) -> Result<Self::Ok> {
        self.write_number(Number::Float64(v as f64))
    }

    fn serialize_f64(self, v: f64) -> Result<Self::Ok> {
        self.write_number(Number::Float64(v))
    }

    fn serialize_i128(self, v: i128) -> Result<Self::Ok> {
        let num = if (DECIMAL64_MIN..=DECIMAL64_MAX).contains(&v) {
            Number::Decimal64(Decimal64 {
                scale: 0,
                value: v as i64,
            })
        } else if (DECIMAL128_MIN..=DECIMAL128_MAX).contains(&v) {
            Number::Decimal128(Decimal128 { scale: 0, value: v })
        } else {
            Number::Decimal256(Decimal256 {
                scale: 0,
                value: i256::from(v),
            })
        };
        self.write_number(num)
    }

    fn serialize_u128(self, v: u128) -> Result<Self::Ok> {
        let num = if v <= DECIMAL64_MAX as u128 {
            Number::Decimal64(Decimal64 {
                scale: 0,
                value: v as i64,
            })
        } else if v <= DECIMAL128_MAX as u128 {
            Number::Decimal128(Decimal128 {
                scale: 0,
                value: v as i128,
            })
        } else {
            Number::Decimal256(Decimal256 {
                scale: 0,
                value: i256::from(v),
            })
        };
        self.write_number(num)
    }

    fn serialize_char(self, v: char) -> Result<Self::Ok> {
        let s: String = v.to_string();
        self.write_str(s.as_str())
    }

    fn serialize_str(self, v: &str) -> Result<Self::Ok> {
        self.write_str(v)
    }

    fn serialize_bytes(self, _v: &[u8]) -> Result<Self::Ok> {
        todo!()
    }

    fn serialize_none(self) -> Result<Self::Ok> {
        self.serialize_unit()
    }

    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<Self::Ok> {
        T::serialize(value, self)
    }

    fn serialize_unit(self) -> Result<Self::Ok> {
        let jentry = JEntry::make_null_jentry();
        self.write_jentry(jentry)
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok> {
        self.serialize_unit()
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<Self::Ok> {
        self.serialize_str(variant)
    }

    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Self::Ok> {
        T::serialize(value, self)
    }

    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Self::Ok> {
        let mut serializer = Serializer::new();
        value.serialize(&mut serializer)?;
        let value_jsonb = OwnedJsonb::new(serializer.buffer);

        let mut builder = ObjectBuilder::new();
        builder.push_owned_jsonb(variant, value_jsonb)?;
        let object_jsonb = builder.build()?;
        let mut buf = object_jsonb.to_vec();
        self.buffer.append(&mut buf);
        Ok(())
    }

    fn serialize_seq(self, len: Option<usize>) -> Result<Self::SerializeSeq> {
        Ok(ArraySerializer::new(None, &mut self.buffer, len))
    }

    fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleStruct> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleVariant> {
        Ok(ArraySerializer::new(
            Some(variant),
            &mut self.buffer,
            Some(len),
        ))
    }

    fn serialize_map(self, len: Option<usize>) -> Result<Self::SerializeMap> {
        Ok(ObjectSerializer::new(&mut self.buffer, len))
    }

    fn serialize_struct(self, name: &'static str, len: usize) -> Result<Self::SerializeStruct> {
        Ok(StructSerializer::new(name, None, &mut self.buffer, len))
    }

    fn serialize_struct_variant(
        self,
        name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStructVariant> {
        Ok(StructSerializer::new(
            name,
            Some(variant),
            &mut self.buffer,
            len,
        ))
    }

    fn is_human_readable(&self) -> bool {
        false
    }
}

pub struct ArraySerializer<'a> {
    variant: Option<&'static str>,
    buffer: &'a mut Vec<u8>,
    items: Vec<OwnedJsonb>,
}

impl<'a> ArraySerializer<'a> {
    pub fn new(variant: Option<&'static str>, buffer: &'a mut Vec<u8>, len: Option<usize>) -> Self {
        let len = len.unwrap_or_default();
        let items = Vec::with_capacity(len);

        Self {
            variant,
            buffer,
            items,
        }
    }
}

impl ser::SerializeSeq for ArraySerializer<'_> {
    type Ok = ();

    type Error = Error;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
        let mut serializer = Serializer::new();
        value.serialize(&mut serializer)?;
        let item_jsonb = OwnedJsonb::new(serializer.buffer);
        self.items.push(item_jsonb);
        Ok(())
    }

    fn end(self) -> Result<Self::Ok> {
        let mut builder = ArrayBuilder::with_capacity(self.items.len());
        for item in self.items.into_iter() {
            builder.push_owned_jsonb(item);
        }
        let array_jsonb = builder.build()?;
        let mut buf = array_jsonb.to_vec();
        self.buffer.append(&mut buf);
        Ok(())
    }
}

impl ser::SerializeTuple for ArraySerializer<'_> {
    type Ok = ();

    type Error = Error;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
        <Self as ser::SerializeSeq>::serialize_element(self, value)
    }

    fn end(self) -> Result<Self::Ok> {
        <Self as ser::SerializeSeq>::end(self)
    }
}

impl ser::SerializeTupleStruct for ArraySerializer<'_> {
    type Ok = ();

    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        value: &T,
    ) -> std::prelude::v1::Result<(), Self::Error> {
        <Self as ser::SerializeSeq>::serialize_element(self, value)
    }

    fn end(self) -> Result<Self::Ok> {
        <Self as ser::SerializeSeq>::end(self)
    }
}

impl ser::SerializeTupleVariant for ArraySerializer<'_> {
    type Ok = ();

    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
        <Self as ser::SerializeSeq>::serialize_element(self, value)
    }

    fn end(self) -> Result<Self::Ok> {
        let Some(variant) = self.variant else {
            return Err(ser::Error::custom("Variant can not be None".to_string()));
        };

        let mut builder = ArrayBuilder::with_capacity(self.items.len());
        for item in self.items.into_iter() {
            builder.push_owned_jsonb(item);
        }
        let array_jsonb = builder.build()?;

        let mut builder = ObjectBuilder::new();
        builder.push_owned_jsonb(variant, array_jsonb)?;
        let object_jsonb = builder.build()?;
        let mut buf = object_jsonb.to_vec();
        self.buffer.append(&mut buf);
        Ok(())
    }
}

pub struct ObjectSerializer<'a> {
    buffer: &'a mut Vec<u8>,
    keys: Vec<String>,
    values: Vec<OwnedJsonb>,
}

impl<'a> ObjectSerializer<'a> {
    fn new(buffer: &'a mut Vec<u8>, len: Option<usize>) -> Self {
        let len = len.unwrap_or_default();
        let keys = Vec::with_capacity(len);
        let values = Vec::with_capacity(len);

        Self {
            buffer,
            keys,
            values,
        }
    }
}

impl ser::SerializeMap for ObjectSerializer<'_> {
    type Ok = ();

    type Error = Error;

    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<()> {
        let mut serializer = Serializer::new();
        key.serialize(&mut serializer)?;
        let key_jsonb = OwnedJsonb::new(serializer.buffer);
        let raw_jsonb = key_jsonb.as_raw();
        let Ok(Some(key)) = raw_jsonb.as_str() else {
            return Err(ser::Error::custom("Invalid object key".to_string()));
        };
        self.keys.push(key.to_string());

        Ok(())
    }

    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
        let mut serializer = Serializer::new();
        value.serialize(&mut serializer)?;
        let value_jsonb = OwnedJsonb::new(serializer.buffer);
        self.values.push(value_jsonb);

        Ok(())
    }

    fn end(self) -> Result<Self::Ok> {
        if self.keys.len() != self.values.len() {
            return Err(ser::Error::custom(
                "Invalid object keys and values length".to_string(),
            ));
        }
        let mut builder = ObjectBuilder::new();
        for (key_str, value) in self.keys.iter().zip(self.values) {
            builder.push_owned_jsonb(key_str, value)?;
        }
        let object_jsonb = builder.build()?;
        let mut buf = object_jsonb.to_vec();
        self.buffer.append(&mut buf);
        Ok(())
    }
}

pub struct StructSerializer<'a> {
    name: &'static str,
    variant: Option<&'static str>,
    buffer: &'a mut Vec<u8>,
    values: Vec<(&'static str, OwnedJsonb)>,
}

impl<'a> StructSerializer<'a> {
    fn new(
        name: &'static str,
        variant: Option<&'static str>,
        buffer: &'a mut Vec<u8>,
        len: usize,
    ) -> Self {
        let values = Vec::with_capacity(len);

        Self {
            name,
            variant,
            buffer,
            values,
        }
    }
}

impl ser::SerializeStruct for StructSerializer<'_> {
    type Ok = ();

    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<()> {
        let mut serializer = Serializer::new();
        value.serialize(&mut serializer)?;
        let value_jsonb = OwnedJsonb::new(serializer.buffer);
        self.values.push((key, value_jsonb));
        Ok(())
    }

    fn end(mut self) -> Result<Self::Ok> {
        if self.name == NUMBER_STRUCT_TOKEN {
            let num = if self.values.len() == 2 {
                let (value_field_name, value_jsonb) = self.values.remove(1);
                let (scale_field_name, scale_jsonb) = self.values.remove(0);

                if scale_field_name != NUMBER_STRUCT_FIELD_SCALE
                    || value_field_name != NUMBER_STRUCT_FIELD_VALUE
                {
                    return Err(ser::Error::custom(format!(
                        "Invalid number struct field names: scale={}, value={}",
                        scale_field_name, value_field_name
                    )));
                }
                let scale = owned_jsonb_to_u64(scale_field_name, scale_jsonb)?;
                let value = owned_jsonb_to_i128(value_field_name, value_jsonb)?;

                if (DECIMAL64_MIN..=DECIMAL64_MAX).contains(&value)
                    && scale <= MAX_DECIMAL64_PRECISION as u64
                {
                    Number::Decimal64(Decimal64 {
                        scale: scale as u8,
                        value: value as i64,
                    })
                } else if (DECIMAL128_MIN..=DECIMAL128_MAX).contains(&value)
                    && scale <= MAX_DECIMAL128_PRECISION as u64
                {
                    Number::Decimal128(Decimal128 {
                        scale: scale as u8,
                        value,
                    })
                } else if scale <= MAX_DECIMAL256_PRECISION as u64 {
                    Number::Decimal256(Decimal256 {
                        scale: scale as u8,
                        value: i256::from(value),
                    })
                } else {
                    return Err(ser::Error::custom(format!(
                        "Invalid number struct scale={} value={}",
                        scale, value
                    )));
                }
            } else if self.values.len() == 3 {
                let (low_value_field_name, low_value_jsonb) = self.values.remove(2);
                let (high_value_field_name, high_value_jsonb) = self.values.remove(1);
                let (scale_field_name, scale_jsonb) = self.values.remove(0);

                if scale_field_name != NUMBER_STRUCT_FIELD_SCALE
                    || high_value_field_name != NUMBER_STRUCT_FIELD_HIGH_VALUE
                    || low_value_field_name != NUMBER_STRUCT_FIELD_LOW_VALUE
                {
                    return Err(ser::Error::custom(format!(
                        "Invalid number struct field names: scale={}, high_value={} low_value={}",
                        scale_field_name, high_value_field_name, low_value_field_name
                    )));
                }
                let scale = owned_jsonb_to_u64(scale_field_name, scale_jsonb)?;
                let high_value = owned_jsonb_to_i128(high_value_field_name, high_value_jsonb)?;
                let low_value = owned_jsonb_to_i128(low_value_field_name, low_value_jsonb)?;

                if scale <= MAX_DECIMAL256_PRECISION as u64 {
                    Number::Decimal256(Decimal256 {
                        scale: scale as u8,
                        value: i256::from_words(high_value, low_value),
                    })
                } else {
                    return Err(ser::Error::custom(format!(
                        "Invalid number struct scale={} high_value={} low_value={}",
                        scale, high_value, low_value
                    )));
                }
            } else {
                return Err(ser::Error::custom(format!(
                    "Invalid number of fields for number struct: {}",
                    self.values.len()
                )));
            };

            let mut serializer = Serializer::new();
            serializer.write_number(num)?;
            self.buffer.append(&mut serializer.buffer);
        } else {
            let mut builder = ObjectBuilder::new();
            for (key_str, value) in self.values.into_iter() {
                builder.push_owned_jsonb(key_str, value)?;
            }
            let object_jsonb = builder.build()?;
            let mut buf = object_jsonb.to_vec();
            self.buffer.append(&mut buf);
        }
        Ok(())
    }
}

fn owned_jsonb_to_u64(field: &str, owned_jsonb: OwnedJsonb) -> Result<u64> {
    let raw_jsonb = owned_jsonb.as_raw();
    let Ok(Some(num)) = raw_jsonb.as_number() else {
        return Err(ser::Error::custom(format!(
            "Invalid number struct field={} value={}",
            field,
            raw_jsonb.to_string()
        )));
    };
    let Some(num) = num.as_u64() else {
        return Err(ser::Error::custom(format!(
            "Invalid number struct to u64 field={} value={}",
            field,
            raw_jsonb.to_string()
        )));
    };
    Ok(num)
}

fn owned_jsonb_to_i128(field: &str, owned_jsonb: OwnedJsonb) -> Result<i128> {
    let raw_jsonb = owned_jsonb.as_raw();
    let Ok(Some(num)) = raw_jsonb.as_number() else {
        return Err(ser::Error::custom(format!(
            "Invalid number struct field={} value={}",
            field,
            raw_jsonb.to_string()
        )));
    };
    let Some(num) = num.as_i128() else {
        return Err(ser::Error::custom(format!(
            "Invalid number struct to i128 field={} value={}",
            field,
            raw_jsonb.to_string()
        )));
    };
    Ok(num)
}

impl ser::SerializeStructVariant for StructSerializer<'_> {
    type Ok = ();

    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<()> {
        <Self as ser::SerializeStruct>::serialize_field(self, key, value)
    }

    fn end(self) -> Result<Self::Ok> {
        let Some(variant) = self.variant else {
            return Err(ser::Error::custom("Variant can not be None".to_string()));
        };
        let mut builder = ObjectBuilder::new();
        for (key_str, value) in self.values.into_iter() {
            builder.push_owned_jsonb(key_str, value)?;
        }
        let object_jsonb = builder.build()?;

        let mut builder = ObjectBuilder::new();
        builder.push_owned_jsonb(variant, object_jsonb)?;
        let object_jsonb = builder.build()?;
        let mut buf = object_jsonb.to_vec();
        self.buffer.append(&mut buf);
        Ok(())
    }
}

impl Serialize for RawJsonb<'_> {
    #[inline]
    fn serialize<S>(&self, serializer: S) -> core::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut index = 0;
        let (header_type, header_len) = self
            .read_header(index)
            .map_err(|e| ser::Error::custom(format!("{e}")))?;
        index += 4;

        match header_type {
            SCALAR_CONTAINER_TAG => {
                let jentry = self
                    .read_jentry(index)
                    .map_err(|e| ser::Error::custom(format!("{e}")))?;
                index += 4;

                let payload_start = index;
                let payload_end = index + jentry.length as usize;
                match jentry.type_code {
                    NULL_TAG => serializer.serialize_unit(),
                    TRUE_TAG => serializer.serialize_bool(true),
                    FALSE_TAG => serializer.serialize_bool(false),
                    NUMBER_TAG => {
                        let num = Number::decode(&self.data[payload_start..payload_end])
                            .map_err(|e| ser::Error::custom(format!("{e}")))?;
                        num.serialize(serializer)
                    }
                    STRING_TAG => {
                        let s = unsafe {
                            std::str::from_utf8_unchecked(&self.data[payload_start..payload_end])
                        };
                        serializer.serialize_str(s)
                    }
                    EXTENSION_TAG => {
                        let val = ExtensionValue::decode(&self.data[payload_start..payload_end])
                            .map_err(|e| ser::Error::custom(format!("{e}")))?;
                        let s = format!("{}", val);
                        serializer.serialize_str(&s)
                    }
                    CONTAINER_TAG => {
                        // Scalar header can't have contianer jentry tag
                        Err(ser::Error::custom("Invalid jsonb".to_string()))
                    }
                    _ => Err(ser::Error::custom("Invalid jsonb".to_string())),
                }
            }
            ARRAY_CONTAINER_TAG => {
                let mut serialize_seq = serializer.serialize_seq(Some(header_len))?;

                let mut payload_start = index + 4 * header_len;
                for _ in 0..header_len {
                    let jentry = self
                        .read_jentry(index)
                        .map_err(|e| ser::Error::custom(format!("{e}")))?;
                    index += 4;

                    let payload_end = payload_start + jentry.length as usize;
                    match jentry.type_code {
                        NULL_TAG => serialize_seq.serialize_element(&())?,
                        TRUE_TAG => serialize_seq.serialize_element(&true)?,
                        FALSE_TAG => serialize_seq.serialize_element(&false)?,
                        NUMBER_TAG => {
                            let num = Number::decode(&self.data[payload_start..payload_end])
                                .map_err(|e| ser::Error::custom(format!("{e}")))?;
                            serialize_seq.serialize_element(&num)?;
                        }
                        STRING_TAG => {
                            let s = unsafe {
                                std::str::from_utf8_unchecked(
                                    &self.data[payload_start..payload_end],
                                )
                            };
                            serialize_seq.serialize_element(&s)?;
                        }
                        EXTENSION_TAG => {
                            let val =
                                ExtensionValue::decode(&self.data[payload_start..payload_end])
                                    .map_err(|e| ser::Error::custom(format!("{e}")))?;
                            let s = format!("{}", val);
                            serialize_seq.serialize_element(&s)?;
                        }
                        CONTAINER_TAG => {
                            let inner_raw_jsonb =
                                RawJsonb::new(&self.data[payload_start..payload_end]);
                            serialize_seq.serialize_element(&inner_raw_jsonb)?;
                        }
                        _ => {
                            return Err(ser::Error::custom("Invalid jsonb".to_string()));
                        }
                    }
                    payload_start = payload_end;
                }
                serialize_seq.end()
            }
            OBJECT_CONTAINER_TAG => {
                let mut serialize_map = serializer.serialize_map(Some(header_len))?;

                let mut keys = VecDeque::with_capacity(header_len);
                let mut payload_start = index + 8 * header_len;
                for _ in 0..header_len {
                    let jentry = self
                        .read_jentry(index)
                        .map_err(|e| ser::Error::custom(format!("{e}")))?;
                    index += 4;

                    let payload_end = payload_start + jentry.length as usize;
                    match jentry.type_code {
                        STRING_TAG => {
                            let s = unsafe {
                                std::str::from_utf8_unchecked(
                                    &self.data[payload_start..payload_end],
                                )
                            };
                            keys.push_back(s);
                        }
                        _ => {
                            return Err(ser::Error::custom("Invalid jsonb".to_string()));
                        }
                    }
                    payload_start = payload_end;
                }

                for _ in 0..header_len {
                    let jentry = self
                        .read_jentry(index)
                        .map_err(|e| ser::Error::custom(format!("{e}")))?;
                    index += 4;

                    let payload_end = payload_start + jentry.length as usize;
                    let k = keys.pop_front().unwrap();
                    match jentry.type_code {
                        NULL_TAG => serialize_map.serialize_entry(&k, &())?,
                        TRUE_TAG => serialize_map.serialize_entry(&k, &true)?,
                        FALSE_TAG => serialize_map.serialize_entry(&k, &false)?,
                        NUMBER_TAG => {
                            let num = Number::decode(&self.data[payload_start..payload_end])
                                .map_err(|e| ser::Error::custom(format!("{e}")))?;
                            serialize_map.serialize_entry(&k, &num)?;
                        }
                        STRING_TAG => {
                            let s = unsafe {
                                std::str::from_utf8_unchecked(
                                    &self.data[payload_start..payload_end],
                                )
                            };
                            serialize_map.serialize_entry(&k, &s)?;
                        }
                        EXTENSION_TAG => {
                            let val =
                                ExtensionValue::decode(&self.data[payload_start..payload_end])
                                    .map_err(|e| ser::Error::custom(format!("{e}")))?;
                            let s = format!("{}", val);
                            serialize_map.serialize_entry(&k, &s)?;
                        }
                        CONTAINER_TAG => {
                            let inner_raw_jsonb =
                                RawJsonb::new(&self.data[payload_start..payload_end]);
                            serialize_map.serialize_entry(&k, &inner_raw_jsonb)?;
                        }
                        _ => {
                            return Err(ser::Error::custom("Invalid jsonb".to_string()));
                        }
                    }
                    payload_start = payload_end;
                }
                serialize_map.end()
            }
            _ => Err(ser::Error::custom("Invalid jsonb".to_string())),
        }
    }
}

/// `BaseEncoder` provides common buffer management functionality for `JSONB` encoding.
/// It handles low-level operations like reserving space for JEntries, writing encoded
/// JEntries back to the buffer, and encoding container headers.
struct BaseEncoder<'a> {
    buf: &'a mut Vec<u8>,
}

impl<'a> BaseEncoder<'a> {
    /// Creates a new `BaseEncoder` with the given buffer.
    fn new(buf: &'a mut Vec<u8>) -> BaseEncoder<'a> {
        Self { buf }
    }

    /// Reserves space in the buffer for JEntries that will be filled in later.
    /// Returns the starting index where the JEntries will be placed.
    fn reserve_jentries(&mut self, len: usize) -> usize {
        let old_len = self.buf.len();
        let new_len = old_len + len;
        self.buf.resize(new_len, 0);
        old_len
    }

    /// Writes an encoded `JEntry` to the buffer at the specified index.
    /// Updates the index to point to the next `JEntry` position.
    fn replace_jentry(&mut self, jentry: JEntry, jentry_index: &mut usize) {
        let jentry_bytes = jentry.encoded().to_be_bytes();
        for (i, b) in jentry_bytes.iter().enumerate() {
            self.buf[*jentry_index + i] = *b;
        }
        *jentry_index += 4;
    }

    /// Encodes a scalar container header and reserves space for its `JEntry`.
    /// Returns the total length of the scalar container and the index where its JEntry will be placed.
    fn encode_scalar_header(&mut self) -> (usize, usize) {
        let header = SCALAR_CONTAINER_TAG;
        self.buf.write_u32::<BigEndian>(header).unwrap();

        // Scalar Value only has one JEntry
        let scalar_len = 4 + 4;
        let jentry_index = self.reserve_jentries(4);
        (scalar_len, jentry_index)
    }

    /// Encodes an array container header and reserves space for its JEntries.
    /// Returns the total length of the array container and the index where its JEntries will be placed.
    fn encode_array_header(&mut self, len: usize) -> (usize, usize) {
        let header = ARRAY_CONTAINER_TAG | len as u32;
        self.buf.write_u32::<BigEndian>(header).unwrap();

        // `Array` has N `JEntries`
        let array_len = 4 + len * 4;
        let jentry_index = self.reserve_jentries(len * 4);

        (array_len, jentry_index)
    }

    /// Encodes an object container header and reserves space for its JEntries.
    /// Returns the total length of the object container and the index where its JEntries will be placed.
    fn encode_object_header(&mut self, len: usize) -> (usize, usize) {
        let header = OBJECT_CONTAINER_TAG | len as u32;
        self.buf.write_u32::<BigEndian>(header).unwrap();

        // `Object` has 2 * N `JEntries`
        let object_len = 4 + len * 8;
        let jentry_index = self.reserve_jentries(len * 8);

        (object_len, jentry_index)
    }
}

/// Encoder for serializing Value types to `JSONB` binary format.
/// Uses `BaseEncoder` for common buffer management operations.
pub(crate) struct Encoder<'a> {
    base_encoder: BaseEncoder<'a>,
}

impl<'a> Encoder<'a> {
    /// Creates a new `Encoder` with the given buffer.
    pub(crate) fn new(buf: &'a mut Vec<u8>) -> Encoder<'a> {
        let base_encoder = BaseEncoder::new(buf);
        Self { base_encoder }
    }

    /// Encodes a `Value` into `JSONB` binary format.
    /// Dispatches to the appropriate encoding method based on the value type.
    pub(crate) fn encode(&mut self, value: &Value<'a>) {
        match value {
            Value::Array(array) => self.encode_array(array),
            Value::Object(obj) => self.encode_object(obj),
            _ => self.encode_scalar(value),
        };
    }

    /// Encodes a scalar `Value` (null, bool, number, string, or extension types).
    /// Returns the total length of the encoded scalar.
    fn encode_scalar(&mut self, value: &Value<'a>) -> usize {
        let (mut scalar_len, mut jentry_index) = self.base_encoder.encode_scalar_header();

        let jentry = self.encode_value(value);
        scalar_len += jentry.length as usize;
        self.base_encoder.replace_jentry(jentry, &mut jentry_index);

        scalar_len
    }

    /// Encodes an array of Values.
    /// Returns the total length of the encoded array.
    fn encode_array(&mut self, values: &[Value<'a>]) -> usize {
        let (mut array_len, mut jentry_index) = self.base_encoder.encode_array_header(values.len());

        // encode all values
        for value in values.iter() {
            let jentry = self.encode_value(value);
            array_len += jentry.length as usize;
            self.base_encoder.replace_jentry(jentry, &mut jentry_index);
        }

        array_len
    }

    /// Encodes an object of Values (map of string keys to Values).
    /// Returns the total length of the encoded object.
    fn encode_object(&mut self, obj: &Object<'a>) -> usize {
        let (mut object_len, mut jentry_index) = self.base_encoder.encode_object_header(obj.len());

        // encode all keys first
        for key in obj.keys() {
            let len = key.len();
            object_len += len;
            self.base_encoder.buf.extend_from_slice(key.as_bytes());
            let jentry = JEntry::make_string_jentry(len);
            self.base_encoder.replace_jentry(jentry, &mut jentry_index);
        }

        // encode all values
        for value in obj.values() {
            let jentry = self.encode_value(value);
            object_len += jentry.length as usize;
            self.base_encoder.replace_jentry(jentry, &mut jentry_index);
        }

        object_len
    }

    /// Encodes a single `Value` and returns its `JEntry`.
    /// The `JEntry` contains metadata about the encoded value.
    fn encode_value(&mut self, value: &Value<'a>) -> JEntry {
        let old_off = self.base_encoder.buf.len();
        let jentry = match value {
            Value::Null => JEntry::make_null_jentry(),
            Value::Bool(v) => {
                if *v {
                    JEntry::make_true_jentry()
                } else {
                    JEntry::make_false_jentry()
                }
            }
            Value::Number(v) => {
                let _ = v.compact_encode(&mut self.base_encoder.buf).unwrap();
                let len = self.base_encoder.buf.len() - old_off;
                JEntry::make_number_jentry(len)
            }
            Value::String(s) => {
                let len = s.len();
                self.base_encoder
                    .buf
                    .extend_from_slice(s.as_ref().as_bytes());
                JEntry::make_string_jentry(len)
            }
            Value::Binary(v) => {
                let val = ExtensionValue::Binary(v);
                let _ = val.compact_encode(&mut self.base_encoder.buf).unwrap();
                let len = self.base_encoder.buf.len() - old_off;
                JEntry::make_extension_jentry(len)
            }
            Value::Date(v) => {
                let val = ExtensionValue::Date(v.clone());
                let _ = val.compact_encode(&mut self.base_encoder.buf).unwrap();
                let len = self.base_encoder.buf.len() - old_off;
                JEntry::make_extension_jentry(len)
            }
            Value::Timestamp(v) => {
                let val = ExtensionValue::Timestamp(v.clone());
                let _ = val.compact_encode(&mut self.base_encoder.buf).unwrap();
                let len = self.base_encoder.buf.len() - old_off;
                JEntry::make_extension_jentry(len)
            }
            Value::TimestampTz(v) => {
                let val = ExtensionValue::TimestampTz(v.clone());
                let _ = val.compact_encode(&mut self.base_encoder.buf).unwrap();
                let len = self.base_encoder.buf.len() - old_off;
                JEntry::make_extension_jentry(len)
            }
            Value::Interval(v) => {
                let val = ExtensionValue::Interval(v.clone());
                let _ = val.compact_encode(&mut self.base_encoder.buf).unwrap();
                let len = self.base_encoder.buf.len() - old_off;
                JEntry::make_extension_jentry(len)
            }
            Value::Array(array) => {
                let len = self.encode_array(array);
                JEntry::make_container_jentry(len)
            }
            Value::Object(obj) => {
                let len = self.encode_object(obj);
                JEntry::make_container_jentry(len)
            }
        };

        jentry
    }
}

/// `JsonAstEncoder` for serializing JsonAst types to `JSONB` binary format.
/// Similar to `Encoder` but works with `JsonAst` instead of `Value` types.
/// Uses `BaseEncoder` for common buffer management operations.
pub(crate) struct JsonAstEncoder<'a> {
    base_encoder: BaseEncoder<'a>,
}

impl<'a> JsonAstEncoder<'a> {
    /// Creates a new `JsonAstEncoder` with the given buffer.
    pub(crate) fn new(buf: &'a mut Vec<u8>) -> JsonAstEncoder<'a> {
        let base_encoder = BaseEncoder::new(buf);
        Self { base_encoder }
    }

    /// Encodes a `JsonAst` into `JSONB` binary format.
    /// Dispatches to the appropriate encoding method based on the value type.
    pub(crate) fn encode(&mut self, value: &JsonAst<'a>) {
        match value {
            JsonAst::Array(array) => self.encode_array(array),
            JsonAst::Object(obj) => self.encode_object(obj),
            _ => self.encode_scalar(value),
        };
    }

    /// Encodes a scalar JsonAst (null, bool, number, or string).
    /// Returns the total length of the encoded scalar.
    fn encode_scalar(&mut self, value: &JsonAst<'a>) -> usize {
        let (mut scalar_len, mut jentry_index) = self.base_encoder.encode_scalar_header();

        let jentry = self.encode_value(value);
        scalar_len += jentry.length as usize;
        self.base_encoder.replace_jentry(jentry, &mut jentry_index);

        scalar_len
    }

    /// Encodes an array of `JsonAst` values.
    /// Returns the total length of the encoded array.
    fn encode_array(&mut self, values: &[JsonAst<'a>]) -> usize {
        let (mut array_len, mut jentry_index) = self.base_encoder.encode_array_header(values.len());

        // encode all values
        for value in values.iter() {
            let jentry = self.encode_value(value);
            array_len += jentry.length as usize;
            self.base_encoder.replace_jentry(jentry, &mut jentry_index);
        }

        array_len
    }

    /// Encodes an object of `JsonAst` values (vector of key-value pairs).
    /// Returns the total length of the encoded object.
    fn encode_object(&mut self, obj: &[(Cow<'a, str>, JsonAst<'a>, usize)]) -> usize {
        let (mut object_len, mut jentry_index) = self.base_encoder.encode_object_header(obj.len());

        // encode all keys first
        for (key, _, _) in obj.iter() {
            let len = key.len();
            object_len += len;
            self.base_encoder.buf.extend_from_slice(key.as_bytes());
            let jentry = JEntry::make_string_jentry(len);
            self.base_encoder.replace_jentry(jentry, &mut jentry_index);
        }
        // encode all values
        for (_, value, _) in obj.iter() {
            let jentry = self.encode_value(value);
            object_len += jentry.length as usize;
            self.base_encoder.replace_jentry(jentry, &mut jentry_index);
        }

        object_len
    }

    /// Encodes a single `JsonAst` value and returns its `JEntry`.
    /// The `JEntry` contains metadata about the encoded value.
    fn encode_value(&mut self, value: &JsonAst<'a>) -> JEntry {
        let jentry = match value {
            JsonAst::Null => JEntry::make_null_jentry(),
            JsonAst::Bool(v) => {
                if *v {
                    JEntry::make_true_jentry()
                } else {
                    JEntry::make_false_jentry()
                }
            }
            JsonAst::Number(v) => {
                let old_off = self.base_encoder.buf.len();
                let _ = v.compact_encode(&mut self.base_encoder.buf).unwrap();
                let len = self.base_encoder.buf.len() - old_off;
                JEntry::make_number_jentry(len)
            }
            JsonAst::String(s) => {
                let len = s.len();
                self.base_encoder
                    .buf
                    .extend_from_slice(s.as_ref().as_bytes());
                JEntry::make_string_jentry(len)
            }
            JsonAst::Array(array) => {
                let len = self.encode_array(array);
                JEntry::make_container_jentry(len)
            }
            JsonAst::Object(obj) => {
                let len = self.encode_object(obj);
                JEntry::make_container_jentry(len)
            }
        };

        jentry
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "arbitrary_precision")]
    use ethnum::i256;
    use serde::Serialize;
    use std::collections::HashMap;

    use crate::{core::databend::ser::Serializer, Value};
    #[cfg(feature = "arbitrary_precision")]
    use crate::{Decimal128, Decimal256, Decimal64, Number};

    // Basic struct with different field types
    #[derive(Serialize)]
    struct BasicStruct {
        int_field: i32,
        float_field: f64,
        string_field: String,
        bool_field: bool,
        option_field: Option<String>,
    }

    // Nested struct
    #[derive(Serialize)]
    struct NestedStruct {
        name: String,
        basic: BasicStruct,
        count: u32,
    }

    // Struct with array fields
    #[derive(Serialize)]
    struct ArrayStruct {
        names: Vec<String>,
        values: Vec<i32>,
    }

    // Struct with map fields
    #[derive(Serialize)]
    struct MapStruct {
        properties: HashMap<String, String>,
    }

    // Struct with tuple fields
    #[derive(Serialize)]
    struct TupleStruct {
        pair: (i32, String),
        triple: (bool, f64, String),
    }

    // Enum for testing struct variants
    #[derive(Serialize)]
    enum TestEnum {
        Simple(String),
        Struct { id: u32, name: String },
        Tuple(i32, String, bool),
    }

    #[test]
    fn test_basic_struct_serialization() {
        let basic = BasicStruct {
            int_field: 42,
            float_field: std::f64::consts::PI,
            string_field: "hello".to_string(),
            bool_field: true,
            option_field: Some("present".to_string()),
        };

        let mut serializer = Serializer::new();
        basic.serialize(&mut serializer).unwrap();
        let jsonb = serializer.to_owned_jsonb();
        let raw = jsonb.as_raw();

        // Verify the serialized structure
        let value = raw.to_value().unwrap();
        if let Value::Object(obj) = value {
            assert_eq!(obj.len(), 5);
            assert_eq!(obj.get("int_field").unwrap().as_i64().unwrap(), 42);
            assert_eq!(
                obj.get("float_field").unwrap().as_f64().unwrap(),
                std::f64::consts::PI
            );
            assert_eq!(obj.get("string_field").unwrap().as_str().unwrap(), "hello");
            assert!(obj.get("bool_field").unwrap().as_bool().unwrap());
            assert_eq!(
                obj.get("option_field").unwrap().as_str().unwrap(),
                "present"
            );
        } else {
            panic!("Expected object value");
        }

        // Test with None option
        let basic_none = BasicStruct {
            int_field: 100,
            float_field: 12.34f64,
            string_field: "world".to_string(),
            bool_field: false,
            option_field: None,
        };

        let mut serializer = Serializer::new();
        basic_none.serialize(&mut serializer).unwrap();
        let jsonb = serializer.to_owned_jsonb();
        let raw = jsonb.as_raw();

        let value = raw.to_value().unwrap();
        if let Value::Object(obj) = value {
            assert_eq!(obj.len(), 5);
            assert_eq!(obj.get("int_field").unwrap().as_i64().unwrap(), 100);
            assert_eq!(obj.get("float_field").unwrap().as_f64().unwrap(), 12.34f64);
            assert_eq!(obj.get("string_field").unwrap().as_str().unwrap(), "world");
            assert!(!obj.get("bool_field").unwrap().as_bool().unwrap());
            assert!(obj.get("option_field").unwrap().is_null());
        } else {
            panic!("Expected object value");
        }
    }

    #[test]
    fn test_nested_struct_serialization() {
        let nested = NestedStruct {
            name: "nested".to_string(),
            basic: BasicStruct {
                int_field: 100,
                float_field: 2.71f64,
                string_field: "world".to_string(),
                bool_field: false,
                option_field: None,
            },
            count: 5,
        };

        let mut serializer = Serializer::new();
        nested.serialize(&mut serializer).unwrap();
        let jsonb = serializer.to_owned_jsonb();
        let raw = jsonb.as_raw();

        let value = raw.to_value().unwrap();
        if let Value::Object(obj) = value {
            assert_eq!(obj.len(), 3);
            assert_eq!(obj.get("name").unwrap().as_str().unwrap(), "nested");
            assert_eq!(obj.get("count").unwrap().as_u64().unwrap(), 5);

            if let Value::Object(basic_obj) = obj.get("basic").unwrap() {
                assert_eq!(basic_obj.len(), 5);
                assert_eq!(basic_obj.get("int_field").unwrap().as_i64().unwrap(), 100);
                assert_eq!(
                    basic_obj.get("float_field").unwrap().as_f64().unwrap(),
                    2.71f64
                );
                assert_eq!(
                    basic_obj.get("string_field").unwrap().as_str().unwrap(),
                    "world"
                );
                assert!(!basic_obj.get("bool_field").unwrap().as_bool().unwrap());
                assert!(basic_obj.get("option_field").unwrap().is_null());
            } else {
                panic!("Expected object for basic field");
            }
        } else {
            panic!("Expected object value");
        }
    }

    #[test]
    fn test_array_struct_serialization() {
        let array_struct = ArrayStruct {
            names: vec![
                "Alice".to_string(),
                "Bob".to_string(),
                "Charlie".to_string(),
            ],
            values: vec![1, 2, 3, 4, 5],
        };

        let mut serializer = Serializer::new();
        array_struct.serialize(&mut serializer).unwrap();
        let jsonb = serializer.to_owned_jsonb();
        let raw = jsonb.as_raw();

        let value = raw.to_value().unwrap();
        if let Value::Object(obj) = value {
            assert_eq!(obj.len(), 2);

            if let Value::Array(names) = obj.get("names").unwrap() {
                assert_eq!(names.len(), 3);
                assert_eq!(names[0].as_str().unwrap(), "Alice");
                assert_eq!(names[1].as_str().unwrap(), "Bob");
                assert_eq!(names[2].as_str().unwrap(), "Charlie");
            } else {
                panic!("Expected array for names field");
            }

            if let Value::Array(values) = obj.get("values").unwrap() {
                assert_eq!(values.len(), 5);
                for (i, val) in values.iter().enumerate() {
                    assert_eq!(val.as_i64().unwrap(), (i + 1) as i64);
                }
            } else {
                panic!("Expected array for values field");
            }
        } else {
            panic!("Expected object value");
        }
    }

    #[test]
    fn test_map_struct_serialization() {
        let mut properties = HashMap::new();
        properties.insert("key1".to_string(), "value1".to_string());
        properties.insert("key2".to_string(), "value2".to_string());
        properties.insert("key3".to_string(), "value3".to_string());

        let map_struct = MapStruct { properties };

        let mut serializer = Serializer::new();
        map_struct.serialize(&mut serializer).unwrap();
        let jsonb = serializer.to_owned_jsonb();
        let raw = jsonb.as_raw();

        let value = raw.to_value().unwrap();
        if let Value::Object(obj) = value {
            assert_eq!(obj.len(), 1);

            if let Value::Object(props) = obj.get("properties").unwrap() {
                assert_eq!(props.len(), 3);
                assert_eq!(props.get("key1").unwrap().as_str().unwrap(), "value1");
                assert_eq!(props.get("key2").unwrap().as_str().unwrap(), "value2");
                assert_eq!(props.get("key3").unwrap().as_str().unwrap(), "value3");
            } else {
                panic!("Expected object for properties field");
            }
        } else {
            panic!("Expected object value");
        }
    }

    #[test]
    fn test_tuple_struct_serialization() {
        let tuple_struct = TupleStruct {
            pair: (42, "answer".to_string()),
            triple: (true, std::f64::consts::PI, "pi".to_string()),
        };

        let mut serializer = Serializer::new();
        tuple_struct.serialize(&mut serializer).unwrap();
        let jsonb = serializer.to_owned_jsonb();
        let raw = jsonb.as_raw();

        let value = raw.to_value().unwrap();
        if let Value::Object(obj) = value {
            assert_eq!(obj.len(), 2);

            if let Value::Array(pair) = obj.get("pair").unwrap() {
                assert_eq!(pair.len(), 2);
                assert_eq!(pair[0].as_i64().unwrap(), 42);
                assert_eq!(pair[1].as_str().unwrap(), "answer");
            } else {
                panic!("Expected array for pair field");
            }

            if let Value::Array(triple) = obj.get("triple").unwrap() {
                assert_eq!(triple.len(), 3);
                assert!(triple[0].as_bool().unwrap());
                assert_eq!(triple[1].as_f64().unwrap(), std::f64::consts::PI);
                assert_eq!(triple[2].as_str().unwrap(), "pi");
            } else {
                panic!("Expected array for triple field");
            }
        } else {
            panic!("Expected object value");
        }
    }

    #[test]
    #[cfg(feature = "arbitrary_precision")]
    fn test_number_struct_serialization() {
        // Test with a value that should be serialized as Decimal64
        let decimal64_struct = Number::Decimal64(Decimal64 {
            scale: 2,
            value: 1234,
        });

        let mut serializer = Serializer::new();
        decimal64_struct.serialize(&mut serializer).unwrap();
        let jsonb = serializer.to_owned_jsonb();
        let raw = jsonb.as_raw();

        let value = raw.to_value().unwrap();
        if let Value::Number(num) = value {
            match num {
                Number::Decimal64(dec) => {
                    assert_eq!(dec.scale, 2);
                    assert_eq!(dec.value, 1234);
                }
                _ => panic!("Expected Decimal64 number"),
            }
        } else {
            panic!("Expected number value");
        }

        // Test with a value that should be serialized as Decimal128
        let decimal128_struct = Number::Decimal128(Decimal128 {
            scale: 10,
            value: 1000000000000000000485i128,
        });

        let mut serializer = Serializer::new();
        decimal128_struct.serialize(&mut serializer).unwrap();
        let jsonb = serializer.to_owned_jsonb();
        let raw = jsonb.as_raw();

        let value = raw.to_value().unwrap();
        if let Value::Number(num) = value {
            match num {
                Number::Decimal128(dec) => {
                    assert_eq!(dec.scale, 10);
                    assert_eq!(dec.value, 1000000000000000000485i128);
                }
                _ => panic!("Expected Decimal128 number"),
            }
        } else {
            panic!("Expected number value");
        }

        // Test with a value that should be serialized as Decimal256
        let decimal256_struct = Number::Decimal256(Decimal256 {
            scale: 15,
            value: i256::from_words(
                999999999999999999999999999999999999i128,
                999999999999999999999999999999999999i128,
            ),
        });

        let mut serializer = Serializer::new();
        decimal256_struct.serialize(&mut serializer).unwrap();
        let jsonb = serializer.to_owned_jsonb();
        let raw = jsonb.as_raw();

        let value = raw.to_value().unwrap();
        if let Value::Number(num) = value {
            match num {
                Number::Decimal256(dec) => {
                    assert_eq!(dec.scale, 15);
                    assert_eq!(
                        dec.value,
                        i256::from_words(
                            999999999999999999999999999999999999i128,
                            999999999999999999999999999999999999i128
                        )
                    );
                }
                _ => panic!("Expected Decimal256 number"),
            }
        } else {
            panic!("Expected number value");
        }
    }

    #[test]
    fn test_enum_serialization() {
        // Test simple variant
        let simple = TestEnum::Simple("simple value".to_string());
        let mut serializer = Serializer::new();
        simple.serialize(&mut serializer).unwrap();
        let jsonb = serializer.to_owned_jsonb();
        let raw = jsonb.as_raw();

        let value = raw.to_value().unwrap();
        if let Value::Object(obj) = value {
            assert_eq!(obj.len(), 1);
            assert_eq!(obj.get("Simple").unwrap().as_str().unwrap(), "simple value");
        } else {
            panic!("Expected object");
        }

        // Test struct variant
        let struct_variant = TestEnum::Struct {
            id: 123,
            name: "test name".to_string(),
        };
        let mut serializer = Serializer::new();
        struct_variant.serialize(&mut serializer).unwrap();
        let jsonb = serializer.to_owned_jsonb();
        let raw = jsonb.as_raw();

        let value = raw.to_value().unwrap();
        if let Value::Object(obj) = value {
            assert_eq!(obj.len(), 1);

            if let Value::Object(struct_obj) = obj.get("Struct").unwrap() {
                assert_eq!(struct_obj.len(), 2);
                assert_eq!(struct_obj.get("id").unwrap().as_u64().unwrap(), 123);
                assert_eq!(
                    struct_obj.get("name").unwrap().as_str().unwrap(),
                    "test name"
                );
            } else {
                panic!("Expected object for Struct variant");
            }
        } else {
            panic!("Expected object value for Struct variant");
        }

        // Test tuple variant
        let tuple_variant = TestEnum::Tuple(42, "tuple value".to_string(), true);
        let mut serializer = Serializer::new();
        tuple_variant.serialize(&mut serializer).unwrap();
        let jsonb = serializer.to_owned_jsonb();
        let raw = jsonb.as_raw();

        let value = raw.to_value().unwrap();
        if let Value::Object(obj) = value {
            assert_eq!(obj.len(), 1);

            if let Value::Array(tuple_arr) = obj.get("Tuple").unwrap() {
                assert_eq!(tuple_arr.len(), 3);
                assert_eq!(tuple_arr[0].as_i64().unwrap(), 42);
                assert_eq!(tuple_arr[1].as_str().unwrap(), "tuple value");
                assert!(tuple_arr[2].as_bool().unwrap());
            } else {
                panic!("Expected array for Tuple variant");
            }
        } else {
            panic!("Expected object value for Tuple variant");
        }
    }
}
