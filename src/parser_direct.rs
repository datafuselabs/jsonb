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

#[cfg(feature = "arbitrary_precision")]
use crate::constants::MAX_DECIMAL64_PRECISION;
#[cfg(feature = "arbitrary_precision")]
use crate::constants::{DECIMAL128_MAX, DECIMAL128_MIN, DECIMAL64_MAX, DECIMAL64_MIN};
use crate::constants::{
    INT64_MAX, INT64_MIN, MAX_DECIMAL128_PRECISION, MAX_DECIMAL256_PRECISION, UINT64_MAX,
    UINT64_MIN,
};
use crate::error::Error;
use crate::error::ParseErrorCode;
use crate::error::Result;
use crate::util::parse_string;
#[cfg(feature = "arbitrary_precision")]
use crate::Decimal128;
#[cfg(feature = "arbitrary_precision")]
use crate::Decimal256;
#[cfg(feature = "arbitrary_precision")]
use crate::Decimal64;
use crate::Number;
use crate::OwnedJsonb;
#[cfg(feature = "arbitrary_precision")]
use ethnum::i256;
use sonic_simd::u8x32;
use sonic_simd::Mask;
use sonic_simd::Simd;

const LANES: usize = <u8x32 as Simd>::LANES;

const ARRAY_CONTAINER_TAG: u32 = 0x8000_0000;
const OBJECT_CONTAINER_TAG: u32 = 0x4000_0000;
const SCALAR_CONTAINER_TAG: u32 = 0x2000_0000;

const NULL_TAG: u32 = 0x0000_0000;
const STRING_TAG: u32 = 0x1000_0000;
const NUMBER_TAG: u32 = 0x2000_0000;
const FALSE_TAG: u32 = 0x3000_0000;
const TRUE_TAG: u32 = 0x4000_0000;
const CONTAINER_TAG: u32 = 0x5000_0000;
const SMALL_OBJECT_SORT_LIMIT: usize = 32;

#[cfg(feature = "arbitrary_precision")]
static POWER_TABLE: std::sync::LazyLock<[i256; 39]> = std::sync::LazyLock::new(|| {
    [
        i256::from(1_i128),
        i256::from(10_i128),
        i256::from(100_i128),
        i256::from(1000_i128),
        i256::from(10000_i128),
        i256::from(100000_i128),
        i256::from(1000000_i128),
        i256::from(10000000_i128),
        i256::from(100000000_i128),
        i256::from(1000000000_i128),
        i256::from(10000000000_i128),
        i256::from(100000000000_i128),
        i256::from(1000000000000_i128),
        i256::from(10000000000000_i128),
        i256::from(100000000000000_i128),
        i256::from(1000000000000000_i128),
        i256::from(10000000000000000_i128),
        i256::from(100000000000000000_i128),
        i256::from(1000000000000000000_i128),
        i256::from(10000000000000000000_i128),
        i256::from(100000000000000000000_i128),
        i256::from(1000000000000000000000_i128),
        i256::from(10000000000000000000000_i128),
        i256::from(100000000000000000000000_i128),
        i256::from(1000000000000000000000000_i128),
        i256::from(10000000000000000000000000_i128),
        i256::from(100000000000000000000000000_i128),
        i256::from(1000000000000000000000000000_i128),
        i256::from(10000000000000000000000000000_i128),
        i256::from(100000000000000000000000000000_i128),
        i256::from(1000000000000000000000000000000_i128),
        i256::from(10000000000000000000000000000000_i128),
        i256::from(100000000000000000000000000000000_i128),
        i256::from(1000000000000000000000000000000000_i128),
        i256::from(10000000000000000000000000000000000_i128),
        i256::from(100000000000000000000000000000000000_i128),
        i256::from(1000000000000000000000000000000000000_i128),
        i256::from(10000000000000000000000000000000000000_i128),
        i256::from(100000000000000000000000000000000000000_i128),
    ]
});

#[derive(Clone, Copy)]
struct EncodedValue {
    jentry: u32,
    start: usize,
    len: usize,
}

struct ObjectEntry<'a> {
    key: Cow<'a, str>,
    value: EncodedValue,
    pos: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StringScan {
    end: usize,
    has_escape: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StringScanError {
    ControlCharacter(usize),
    Eof,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContainerDelimiter {
    Next,
    End,
}

struct IntegerScan {
    end: usize,
    value: u64,
    digits: usize,
    overflow: bool,
}

pub fn parse_owned_jsonb_direct(buf: &[u8]) -> Result<OwnedJsonb> {
    let mut parser = DirectParser::<false>::new(buf);
    parser.parse_owned()
}

pub fn parse_owned_jsonb_standard_mode_direct(buf: &[u8]) -> Result<OwnedJsonb> {
    let mut parser = DirectParser::<true>::new(buf);
    parser.parse_owned()
}

struct DirectParser<'a, const STANDARD: bool> {
    buf: &'a [u8],
    idx: usize,
    writer: JsonbWriter,
    object_entries: Vec<ObjectEntry<'a>>,
}

impl<'a, const STANDARD: bool> DirectParser<'a, STANDARD> {
    fn new(buf: &'a [u8]) -> Self {
        Self {
            buf,
            idx: 0,
            writer: JsonbWriter::with_capacity(buf.len()),
            object_entries: Vec::with_capacity(32),
        }
    }

    fn parse_owned(&mut self) -> Result<OwnedJsonb> {
        let value = self.parse_value()?;
        self.skip_unused();
        if self.idx < self.buf.len() {
            self.idx += 1;
            return Err(self.error(ParseErrorCode::UnexpectedTrailingCharacters));
        }

        if value.jentry & CONTAINER_TAG == CONTAINER_TAG {
            if value.start == 0 && value.len == self.writer.buf.len() {
                let out = std::mem::take(&mut self.writer.buf);
                return Ok(OwnedJsonb::new(out));
            }

            let mut out = Vec::with_capacity(value.len);
            out.extend_from_slice(self.writer.value_bytes(value));
            Ok(OwnedJsonb::new(out))
        } else {
            let mut out = Vec::with_capacity(8 + value.len);
            write_u32(&mut out, SCALAR_CONTAINER_TAG);
            write_u32(&mut out, value.jentry);
            out.extend_from_slice(self.writer.value_bytes(value));
            Ok(OwnedJsonb::new(out))
        }
    }

    fn parse_value(&mut self) -> Result<EncodedValue> {
        self.skip_unused();
        let Some(byte) = self.peek() else {
            if STANDARD {
                return Err(self.error(ParseErrorCode::InvalidEOF));
            }
            return Ok(EncodedValue {
                jentry: NULL_TAG,
                start: self.writer.buf.len(),
                len: 0,
            });
        };

        match byte {
            b'n' if STANDARD => self.parse_standard_null(),
            b't' if STANDARD => self.parse_standard_true(),
            b'f' if STANDARD => self.parse_standard_false(),
            b'n' | b'N' if !STANDARD => self.parse_extended_null_or_nan(),
            b't' | b'T' if !STANDARD => self.parse_extended_true(),
            b'f' | b'F' if !STANDARD => self.parse_extended_false(),
            b'i' | b'I' if !STANDARD => self.parse_extended_infinity(false),
            b'0'..=b'9' | b'-' if STANDARD => self.parse_number(),
            b'0'..=b'9' | b'-' | b'+' | b'.' if !STANDARD => self.parse_number(),
            b'"' => {
                self.idx += 1;
                self.parse_string(b'"')
            }
            b'\'' if !STANDARD => {
                self.idx += 1;
                self.parse_string(b'\'')
            }
            b'[' => self.parse_array(),
            b'{' => self.parse_object(),
            _ => {
                self.idx += 1;
                Err(self.error(ParseErrorCode::ExpectedSomeValue))
            }
        }
    }

    fn parse_array(&mut self) -> Result<EncodedValue> {
        self.expect_byte(b'[')?;
        let frame_start = self.writer.buf.len();
        let jentry_start = self.writer.jentries.len();

        self.skip_unused();
        if self.consume_if(b']') {
            return Ok(self.writer.close_array(frame_start, jentry_start));
        }

        loop {
            let value = if !STANDARD && matches!(self.peek(), Some(b',' | b']')) {
                EncodedValue {
                    jentry: NULL_TAG,
                    start: self.writer.buf.len(),
                    len: 0,
                }
            } else {
                self.parse_value()?
            };
            self.writer.jentries.push(value.jentry);

            match self.consume_array_delimiter()? {
                ContainerDelimiter::End => break,
                ContainerDelimiter::Next => {
                    self.skip_unused();
                }
            }
        }

        Ok(self.writer.close_array(frame_start, jentry_start))
    }

    fn parse_object(&mut self) -> Result<EncodedValue> {
        self.expect_byte(b'{')?;
        let frame_start = self.writer.buf.len();
        let entry_start = self.object_entries.len();

        self.skip_unused();
        if self.consume_if(b'}') {
            return self.close_object_frame(frame_start, entry_start);
        }

        loop {
            let key = self.parse_object_key()?;
            let pos = self.idx;

            self.skip_unused();
            self.expect_byte(b':')?;

            let value = self.parse_value()?;
            self.object_entries.push(ObjectEntry { key, value, pos });

            match self.consume_object_delimiter()? {
                ContainerDelimiter::End => break,
                ContainerDelimiter::Next => {
                    self.skip_unused();
                }
            }
        }

        self.close_object_frame(frame_start, entry_start)
    }

    fn close_object_frame(
        &mut self,
        frame_start: usize,
        entry_start: usize,
    ) -> Result<EncodedValue> {
        let value = self
            .writer
            .close_object(frame_start, &self.object_entries[entry_start..])?;
        self.object_entries.truncate(entry_start);
        Ok(value)
    }

    fn consume_array_delimiter(&mut self) -> Result<ContainerDelimiter> {
        self.skip_unused();
        if self.idx >= self.buf.len() {
            return Err(self.error(ParseErrorCode::InvalidEOF));
        }

        let pos = self.idx;
        let byte = self.buf[pos];
        if byte == b',' {
            self.idx = pos + 1;
            return Ok(ContainerDelimiter::Next);
        }
        if byte == b']' {
            self.idx = pos + 1;
            return Ok(ContainerDelimiter::End);
        }
        Err(Error::Syntax(ParseErrorCode::ExpectedArrayCommaOrEnd, pos))
    }

    fn consume_object_delimiter(&mut self) -> Result<ContainerDelimiter> {
        self.skip_unused();
        if self.idx >= self.buf.len() {
            return Err(self.error(ParseErrorCode::InvalidEOF));
        }

        let pos = self.idx;
        let byte = self.buf[pos];
        if byte == b',' {
            self.idx = pos + 1;
            return Ok(ContainerDelimiter::Next);
        }
        if byte == b'}' {
            self.idx = pos + 1;
            return Ok(ContainerDelimiter::End);
        }
        Err(Error::Syntax(ParseErrorCode::ExpectedObjectCommaOrEnd, pos))
    }

    fn parse_string(&mut self, end_quote: u8) -> Result<EncodedValue> {
        let value = self.parse_quoted_string(end_quote)?;
        Ok(self.writer.append_string(value.as_bytes()))
    }

    fn parse_number(&mut self) -> Result<EncodedValue> {
        let num = self.parse_number_value()?;
        self.writer.append_number(num)
    }

    fn parse_number_value(&mut self) -> Result<Number> {
        if STANDARD {
            self.parse_standard_number_value()
        } else {
            self.parse_extended_number_value()
        }
    }

    fn parse_standard_number_value(&mut self) -> Result<Number> {
        let start = self.idx;

        let mut negative = false;
        let mut has_fraction = false;
        let mut has_exponent = false;
        let int_value;
        let int_overflow;

        if self.peek() == Some(b'-') {
            negative = true;
            self.idx += 1;
        }

        if self.peek() == Some(b'0') {
            self.idx += 1;
            if self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                self.idx += 1;
                return Err(self.error(ParseErrorCode::InvalidNumberValue));
            }
            int_value = 0;
            int_overflow = false;
        } else {
            let scan = scan_integer_digits_u64(self.buf, self.idx);
            if scan.digits == 0 {
                return Err(self.error(ParseErrorCode::InvalidNumberValue));
            }
            self.idx = scan.end;
            int_value = scan.value;
            int_overflow = scan.overflow;
        }

        if self.peek() == Some(b'.') {
            has_fraction = true;
            self.idx += 1;
            if self.scan_digits() == 0 {
                self.idx += 1;
                return Err(self.error(ParseErrorCode::InvalidNumberValue));
            }
        }

        if matches!(self.peek(), Some(b'E' | b'e')) {
            has_exponent = true;
            self.idx += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.idx += 1;
            }
            if self.scan_digits() == 0 {
                return Err(self.error(ParseErrorCode::InvalidNumberValue));
            }
        }

        let s = self.number_str(start)?;
        if !has_fraction && !has_exponent {
            if !negative && !int_overflow {
                return Ok(Number::UInt64(int_value));
            }
            if negative && !int_overflow {
                if int_value <= i64::MAX as u64 {
                    return Ok(Number::Int64(-(int_value as i64)));
                }
                if int_value == i64::MAX as u64 + 1 {
                    return Ok(Number::Int64(i64::MIN));
                }
            }
        }

        fast_float2::parse(s)
            .map(Number::Float64)
            .map_err(|_| self.error(ParseErrorCode::InvalidNumberValue))
    }

    fn parse_extended_number_value(&mut self) -> Result<Number> {
        let start = self.idx;
        if let Some(number) = self.try_parse_extended_integer_fast()? {
            return Ok(number);
        }
        self.idx = start;

        let mut negative = false;
        let mut leading_zeros = 0;

        match self.peek() {
            Some(b'-') => {
                negative = true;
                self.idx += 1;
            }
            Some(b'+') => {
                self.idx += 1;
            }
            _ => {}
        }

        while self.peek() == Some(b'0') {
            leading_zeros += 1;
            self.idx += 1;
        }

        let mut hi_value = 0_i128;
        let mut lo_value = 0_i128;
        let mut scale = 0_u32;
        let mut precision = 0_usize;
        let mut has_fraction = false;
        let mut has_exponent = false;

        while precision < MAX_DECIMAL256_PRECISION {
            match self.peek() {
                Some(byte) if byte.is_ascii_digit() => {
                    let digit = (byte - b'0') as i128;
                    if precision < MAX_DECIMAL128_PRECISION {
                        hi_value = unsafe { hi_value.unchecked_mul(10_i128) };
                        hi_value = unsafe { hi_value.unchecked_add(digit) };
                    } else {
                        lo_value = unsafe { lo_value.unchecked_mul(10_i128) };
                        lo_value = unsafe { lo_value.unchecked_add(digit) };
                    }
                    self.idx += 1;
                    precision += 1;
                    if has_fraction {
                        scale += 1;
                    }
                }
                Some(b'.') => {
                    if has_fraction {
                        return Err(self.error(ParseErrorCode::InvalidNumberValue));
                    }
                    has_fraction = true;
                    self.idx += 1;
                }
                _ => break,
            }
        }

        if precision == MAX_DECIMAL256_PRECISION {
            if !has_fraction {
                let len = self.scan_digits();
                precision += len;
                if self.peek() == Some(b'.') {
                    has_fraction = true;
                    self.idx += 1;
                }
            }
            if has_fraction {
                let len = self.scan_digits();
                precision += len;
                scale += len as u32;
            }
        }

        if leading_zeros == 0 && precision == 0 {
            if !has_fraction {
                match self.peek() {
                    Some(b'i' | b'I') => {
                        self.expect_slice_ignore_ascii_case(b"infinity")?;
                        return Ok(Number::Float64(if negative {
                            f64::NEG_INFINITY
                        } else {
                            f64::INFINITY
                        }));
                    }
                    Some(b'n' | b'N') => {
                        self.expect_slice_ignore_ascii_case(b"nan")?;
                        if negative {
                            return Err(self.error(ParseErrorCode::InvalidNumberValue));
                        }
                        return Ok(Number::Float64(f64::NAN));
                    }
                    _ => {}
                }
            }
            return Err(self.error(ParseErrorCode::InvalidNumberValue));
        } else if leading_zeros == 1
            && precision == 0
            && !has_fraction
            && matches!(self.peek(), Some(b'x' | b'X'))
        {
            self.idx += 1;
            return self.parse_hex_number_value(negative);
        }

        if matches!(self.peek(), Some(b'E' | b'e')) {
            has_exponent = true;
            self.idx += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.idx += 1;
            }
            if self.scan_digits() == 0 {
                return Err(self.error(ParseErrorCode::InvalidNumberValue));
            }
        }

        if !has_exponent && precision <= MAX_DECIMAL128_PRECISION {
            let value = if negative { -hi_value } else { hi_value };
            if scale == 0 && (UINT64_MIN..=UINT64_MAX).contains(&value) {
                return Ok(Number::UInt64(u64::try_from(value).unwrap()));
            } else if scale == 0 && (INT64_MIN..=INT64_MAX).contains(&value) {
                return Ok(Number::Int64(i64::try_from(value).unwrap()));
            }

            #[cfg(feature = "arbitrary_precision")]
            {
                if (DECIMAL64_MIN..=DECIMAL64_MAX).contains(&value)
                    && precision <= MAX_DECIMAL64_PRECISION
                {
                    return Ok(Number::Decimal64(Decimal64 {
                        scale: scale as u8,
                        value: i64::try_from(value).unwrap(),
                    }));
                } else if (DECIMAL128_MIN..=DECIMAL128_MAX).contains(&value) {
                    return Ok(Number::Decimal128(Decimal128 {
                        scale: scale as u8,
                        value,
                    }));
                }
            }
        }

        #[cfg(feature = "arbitrary_precision")]
        if !has_exponent && precision <= MAX_DECIMAL256_PRECISION {
            let multiplier = POWER_TABLE[precision - MAX_DECIMAL128_PRECISION];
            let mut i256_value = i256::from(hi_value) * multiplier + i256::from(lo_value);
            if negative {
                i256_value *= -1;
            }
            return Ok(Number::Decimal256(Decimal256 {
                scale: scale as u8,
                value: i256_value,
            }));
        }

        let s = self.number_str(start)?;
        fast_float2::parse(s)
            .map(Number::Float64)
            .map_err(|_| self.error(ParseErrorCode::InvalidNumberValue))
    }

    fn try_parse_extended_integer_fast(&mut self) -> Result<Option<Number>> {
        let start = self.idx;
        let mut negative = false;

        match self.peek() {
            Some(b'-') => {
                negative = true;
                self.idx += 1;
            }
            Some(b'+') => {
                self.idx += 1;
            }
            _ => {}
        }

        let digit_start = self.idx;
        let scan = scan_integer_digits_u64(self.buf, digit_start);
        if scan.digits == 0 {
            self.idx = start;
            return Ok(None);
        }
        self.idx = scan.end;

        match self.peek() {
            Some(b'.' | b'E' | b'e') => {
                self.idx = start;
                return Ok(None);
            }
            Some(b'x' | b'X') if scan.digits == 1 && self.buf.get(digit_start) == Some(&b'0') => {
                self.idx = start;
                return Ok(None);
            }
            _ => {}
        }

        if scan.overflow {
            self.idx = start;
            return Ok(None);
        }

        if !negative {
            return Ok(Some(Number::UInt64(scan.value)));
        }
        if scan.value <= i64::MAX as u64 {
            return Ok(Some(Number::Int64(-(scan.value as i64))));
        }
        if scan.value == i64::MAX as u64 + 1 {
            return Ok(Some(Number::Int64(i64::MIN)));
        }

        self.idx = start;
        Ok(None)
    }

    fn parse_hex_number_value(&mut self, negative: bool) -> Result<Number> {
        let int_start = self.idx;
        let int_len = self.scan_hexdigits();
        if int_len == 0 {
            return Err(self.error(ParseErrorCode::InvalidNumberValue));
        }

        if self.peek() == Some(b'.') {
            self.idx += 1;
            let frac_start = self.idx;
            let frac_len = self.scan_hexdigits();
            if frac_len == 0 {
                return Err(self.error(ParseErrorCode::InvalidNumberValue));
            }

            let int_str = std::str::from_utf8(&self.buf[int_start..int_start + int_len])
                .map_err(|_| self.error(ParseErrorCode::InvalidNumberValue))?;
            let frac_str = std::str::from_utf8(&self.buf[frac_start..frac_start + frac_len])
                .map_err(|_| self.error(ParseErrorCode::InvalidNumberValue))?;
            let int_val = u128::from_str_radix(int_str, 16)
                .map_err(|_| self.error(ParseErrorCode::InvalidNumberValue))?;
            let frac_val = u128::from_str_radix(frac_str, 16)
                .map_err(|_| self.error(ParseErrorCode::InvalidNumberValue))?;
            let mut value = int_val as f64 + (frac_val as f64 / 16.0_f64.powi(frac_len as i32));
            if negative {
                value = -value;
            }
            return Ok(Number::Float64(value));
        }

        let int_str = std::str::from_utf8(&self.buf[int_start..self.idx])
            .map_err(|_| self.error(ParseErrorCode::InvalidNumberValue))?;
        let value = u128::from_str_radix(int_str, 16)
            .map_err(|_| self.error(ParseErrorCode::InvalidNumberValue))?;

        if negative {
            if value <= (i64::MAX as u128 + 1) {
                let value = if value == i64::MAX as u128 + 1 {
                    i64::MIN
                } else {
                    -(value as i64)
                };
                return Ok(Number::Int64(value));
            }
            #[cfg(feature = "arbitrary_precision")]
            {
                if value <= (DECIMAL128_MAX as u128 + 1) {
                    return Ok(Number::Decimal128(Decimal128 {
                        scale: 0,
                        value: -(value as i128),
                    }));
                }
                Ok(Number::Decimal256(Decimal256 {
                    scale: 0,
                    value: i256::from(value) * -1,
                }))
            }
            #[cfg(not(feature = "arbitrary_precision"))]
            {
                Ok(Number::Float64(-(value as f64)))
            }
        } else {
            if value <= u64::MAX as u128 {
                return Ok(Number::UInt64(value as u64));
            }
            #[cfg(feature = "arbitrary_precision")]
            {
                if value <= DECIMAL128_MAX as u128 {
                    return Ok(Number::Decimal128(Decimal128 {
                        scale: 0,
                        value: value as i128,
                    }));
                }
                Ok(Number::Decimal256(Decimal256 {
                    scale: 0,
                    value: i256::from(value),
                }))
            }
            #[cfg(not(feature = "arbitrary_precision"))]
            {
                Ok(Number::Float64(value as f64))
            }
        }
    }

    fn parse_object_key(&mut self) -> Result<Cow<'a, str>> {
        if STANDARD {
            self.expect_byte(b'"')?;
            self.parse_quoted_string(b'"')
        } else if let Some(end_quote @ (b'"' | b'\'')) = self.peek() {
            self.idx += 1;
            self.parse_quoted_string(end_quote)
        } else {
            self.parse_unquoted_string()
        }
    }

    fn parse_quoted_string(&mut self, end_quote: u8) -> Result<Cow<'a, str>> {
        let start_idx = self.idx;
        let scan = match scan_string::<false>(self.buf, start_idx, end_quote) {
            Ok(scan) => scan,
            Err(StringScanError::Eof) => {
                self.idx = self.buf.len();
                return Err(self.error(ParseErrorCode::InvalidEOF));
            }
            Err(StringScanError::ControlCharacter(pos)) => {
                return Err(Error::Syntax(ParseErrorCode::InvalidStringValue, pos));
            }
        };
        self.idx = scan.end + 1;

        let data = &self.buf[start_idx..scan.end];
        if scan.has_escape {
            let mut idx = start_idx + 1;
            let s = parse_string(data, data.len(), &mut idx)?;
            Ok(Cow::Owned(s))
        } else {
            std::str::from_utf8(data)
                .map(Cow::Borrowed)
                .map_err(|_| self.error(ParseErrorCode::InvalidStringValue))
        }
    }

    fn parse_unquoted_string(&mut self) -> Result<Cow<'a, str>> {
        let start_idx = self.idx;

        let Some(byte) = self.peek() else {
            return Err(self.error(ParseErrorCode::InvalidEOF));
        };
        if byte.is_ascii_digit() {
            self.idx += 1;
            return Err(self.error(ParseErrorCode::ObjectKeyInvalidNumber));
        }

        loop {
            let Some(byte) = self.peek() else {
                return Err(self.error(ParseErrorCode::InvalidEOF));
            };
            if byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$') {
                self.idx += 1;
            } else if byte >= 0x80 {
                let continuation_bytes = if byte >= 0xF0 {
                    4
                } else if byte >= 0xE0 {
                    3
                } else if byte >= 0xC0 {
                    2
                } else {
                    return Err(self.error(ParseErrorCode::ObjectKeyInvalidCharacter));
                };
                self.idx += continuation_bytes;
            } else {
                break;
            }
        }

        if self.idx == start_idx {
            return Err(self.error(ParseErrorCode::ObjectKeyInvalidCharacter));
        }

        let data = &self.buf[start_idx..self.idx];
        std::str::from_utf8(data)
            .map(Cow::Borrowed)
            .map_err(|_| self.error(ParseErrorCode::InvalidStringValue))
    }

    fn parse_standard_null(&mut self) -> Result<EncodedValue> {
        self.expect_slice(b"null")?;
        Ok(self.writer.empty_value(NULL_TAG))
    }

    fn parse_standard_true(&mut self) -> Result<EncodedValue> {
        self.expect_slice(b"true")?;
        Ok(self.writer.empty_value(TRUE_TAG))
    }

    fn parse_standard_false(&mut self) -> Result<EncodedValue> {
        self.expect_slice(b"false")?;
        Ok(self.writer.empty_value(FALSE_TAG))
    }

    fn parse_extended_true(&mut self) -> Result<EncodedValue> {
        self.expect_slice_ignore_ascii_case(b"true")?;
        Ok(self.writer.empty_value(TRUE_TAG))
    }

    fn parse_extended_false(&mut self) -> Result<EncodedValue> {
        self.expect_slice_ignore_ascii_case(b"false")?;
        Ok(self.writer.empty_value(FALSE_TAG))
    }

    fn parse_extended_null_or_nan(&mut self) -> Result<EncodedValue> {
        let idx = self.idx;
        if self.expect_slice_ignore_ascii_case(b"null").is_ok() {
            return Ok(self.writer.empty_value(NULL_TAG));
        }
        self.idx = idx;
        self.expect_slice_ignore_ascii_case(b"nan")?;
        self.writer.append_number(Number::Float64(f64::NAN))
    }

    fn parse_extended_infinity(&mut self, negative: bool) -> Result<EncodedValue> {
        self.expect_slice_ignore_ascii_case(b"infinity")?;
        let value = if negative {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
        self.writer.append_number(Number::Float64(value))
    }

    fn skip_unused(&mut self) {
        while self.idx < self.buf.len() {
            let next = skip_ascii_whitespace(self.buf, self.idx);
            if next != self.idx {
                self.idx = next;
                continue;
            }

            let byte = self.buf[self.idx];
            if byte == b'\\' && self.idx + 1 < self.buf.len() {
                let next = self.buf[self.idx + 1];
                if matches!(next, b'n' | b'r' | b't') {
                    self.idx += 2;
                    continue;
                }

                if self.idx + 3 < self.buf.len()
                    && next == b'x'
                    && self.buf[self.idx + 2] == b'0'
                    && self.buf[self.idx + 3] == b'C'
                {
                    self.idx += 4;
                    continue;
                }
            }

            break;
        }
    }

    fn expect_byte(&mut self, byte: u8) -> Result<()> {
        match self.peek() {
            Some(value) => {
                self.idx += 1;
                if value == byte {
                    Ok(())
                } else {
                    Err(self.error(ParseErrorCode::ExpectedSomeIdent))
                }
            }
            None => Err(self.error(ParseErrorCode::InvalidEOF)),
        }
    }

    fn consume_if(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            self.idx += 1;
            true
        } else {
            false
        }
    }

    fn expect_slice(&mut self, expected: &[u8]) -> Result<()> {
        for byte in expected {
            self.expect_byte(*byte)?;
        }
        Ok(())
    }

    fn expect_slice_ignore_ascii_case(&mut self, expected: &[u8]) -> Result<()> {
        for byte in expected {
            match self.peek() {
                Some(value) => {
                    self.idx += 1;
                    if !value.eq_ignore_ascii_case(byte) {
                        return Err(self.error(ParseErrorCode::ExpectedSomeIdent));
                    }
                }
                None => return Err(self.error(ParseErrorCode::InvalidEOF)),
            }
        }
        Ok(())
    }

    fn scan_digits(&mut self) -> usize {
        let start = self.idx;
        self.idx = scan_digits(self.buf, self.idx);
        self.idx - start
    }

    fn scan_hexdigits(&mut self) -> usize {
        let start = self.idx;
        while self.idx < self.buf.len() && self.buf[self.idx].is_ascii_hexdigit() {
            self.idx += 1;
        }
        self.idx - start
    }

    fn number_str(&self, start: usize) -> Result<&'a str> {
        std::str::from_utf8(&self.buf[start..self.idx])
            .map_err(|_| self.error(ParseErrorCode::InvalidNumberValue))
    }

    fn peek(&self) -> Option<u8> {
        self.buf.get(self.idx).copied()
    }

    fn error(&self, code: ParseErrorCode) -> Error {
        Error::Syntax(code, self.idx)
    }
}

struct JsonbWriter {
    buf: Vec<u8>,
    scratch: Vec<u8>,
    jentries: Vec<u32>,
    order: Vec<usize>,
}

impl JsonbWriter {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            buf: Vec::with_capacity(capacity),
            scratch: Vec::with_capacity(capacity.min(4096)),
            jentries: Vec::with_capacity(32),
            order: Vec::with_capacity(32),
        }
    }

    fn value_bytes(&self, value: EncodedValue) -> &[u8] {
        &self.buf[value.start..value.start + value.len]
    }

    fn empty_value(&self, jentry: u32) -> EncodedValue {
        EncodedValue {
            jentry,
            start: self.buf.len(),
            len: 0,
        }
    }

    fn append_string(&mut self, value: &[u8]) -> EncodedValue {
        let start = self.buf.len();
        self.buf.extend_from_slice(value);
        EncodedValue {
            jentry: STRING_TAG | value.len() as u32,
            start,
            len: value.len(),
        }
    }

    fn append_number(&mut self, num: Number) -> Result<EncodedValue> {
        let start = self.buf.len();
        let len = num.compact_encode(&mut self.buf)?;
        Ok(EncodedValue {
            jentry: NUMBER_TAG | len as u32,
            start,
            len,
        })
    }

    fn close_array(&mut self, frame_start: usize, jentry_start: usize) -> EncodedValue {
        let payload_end = self.buf.len();
        let len = self.jentries.len() - jentry_start;
        let payload_len = 4 + len * 4 + payload_end - frame_start;

        self.scratch.clear();
        self.scratch.reserve(payload_len);
        write_u32(&mut self.scratch, ARRAY_CONTAINER_TAG | len as u32);
        for jentry in &self.jentries[jentry_start..] {
            write_u32(&mut self.scratch, *jentry);
        }
        self.scratch
            .extend_from_slice(&self.buf[frame_start..payload_end]);

        self.buf.truncate(frame_start);
        let start = self.buf.len();
        self.buf.extend_from_slice(&self.scratch);
        self.jentries.truncate(jentry_start);

        EncodedValue {
            jentry: CONTAINER_TAG | payload_len as u32,
            start,
            len: payload_len,
        }
    }

    fn close_object(
        &mut self,
        frame_start: usize,
        entries: &[ObjectEntry<'_>],
    ) -> Result<EncodedValue> {
        if entries.len() <= SMALL_OBJECT_SORT_LIMIT {
            let mut order = [0; SMALL_OBJECT_SORT_LIMIT];
            for (idx, slot) in order[..entries.len()].iter_mut().enumerate() {
                *slot = idx;
            }
            let order = &mut order[..entries.len()];
            sort_small_object_order(entries, order);
            check_duplicate_keys(entries, order)?;
            return Ok(self.write_object_with_order(frame_start, entries, order));
        }

        self.order.clear();
        self.order.extend(0..entries.len());
        self.order
            .sort_by(|&left, &right| entries[left].key.cmp(&entries[right].key));
        check_duplicate_keys(entries, &self.order)?;
        Ok(self.write_object_with_reused_order(frame_start, entries))
    }

    fn write_object_with_order(
        &mut self,
        frame_start: usize,
        entries: &[ObjectEntry<'_>],
        order: &[usize],
    ) -> EncodedValue {
        let payload_len = 4
            + entries.len() * 8
            + entries
                .iter()
                .map(|entry| entry.key.len() + entry.value.len)
                .sum::<usize>();

        self.scratch.clear();
        self.scratch.reserve(payload_len);
        write_u32(
            &mut self.scratch,
            OBJECT_CONTAINER_TAG | entries.len() as u32,
        );
        for &idx in order {
            let entry = &entries[idx];
            write_u32(&mut self.scratch, STRING_TAG | entry.key.len() as u32);
        }
        for &idx in order {
            write_u32(&mut self.scratch, entries[idx].value.jentry);
        }
        for &idx in order {
            self.scratch.extend_from_slice(entries[idx].key.as_bytes());
        }
        for &idx in order {
            let value = entries[idx].value;
            self.scratch
                .extend_from_slice(&self.buf[value.start..value.start + value.len]);
        }

        self.buf.truncate(frame_start);
        let start = self.buf.len();
        self.buf.extend_from_slice(&self.scratch);

        EncodedValue {
            jentry: CONTAINER_TAG | payload_len as u32,
            start,
            len: payload_len,
        }
    }

    fn write_object_with_reused_order(
        &mut self,
        frame_start: usize,
        entries: &[ObjectEntry<'_>],
    ) -> EncodedValue {
        let payload_len = 4
            + entries.len() * 8
            + entries
                .iter()
                .map(|entry| entry.key.len() + entry.value.len)
                .sum::<usize>();

        self.scratch.clear();
        self.scratch.reserve(payload_len);
        write_u32(
            &mut self.scratch,
            OBJECT_CONTAINER_TAG | entries.len() as u32,
        );
        for pos in 0..self.order.len() {
            let entry = &entries[self.order[pos]];
            write_u32(&mut self.scratch, STRING_TAG | entry.key.len() as u32);
        }
        for pos in 0..self.order.len() {
            write_u32(&mut self.scratch, entries[self.order[pos]].value.jentry);
        }
        for pos in 0..self.order.len() {
            self.scratch
                .extend_from_slice(entries[self.order[pos]].key.as_bytes());
        }
        for pos in 0..self.order.len() {
            let value = entries[self.order[pos]].value;
            self.scratch
                .extend_from_slice(&self.buf[value.start..value.start + value.len]);
        }

        self.buf.truncate(frame_start);
        let start = self.buf.len();
        self.buf.extend_from_slice(&self.scratch);

        EncodedValue {
            jentry: CONTAINER_TAG | payload_len as u32,
            start,
            len: payload_len,
        }
    }
}

fn sort_small_object_order(entries: &[ObjectEntry<'_>], order: &mut [usize]) {
    for idx in 1..order.len() {
        let current = order[idx];
        let mut hole = idx;
        while hole > 0 && entries[current].key < entries[order[hole - 1]].key {
            order[hole] = order[hole - 1];
            hole -= 1;
        }
        order[hole] = current;
    }
}

fn check_duplicate_keys(entries: &[ObjectEntry<'_>], order: &[usize]) -> Result<()> {
    for pair in order.windows(2) {
        let previous = &entries[pair[0]];
        let current = &entries[pair[1]];
        if previous.key == current.key {
            return Err(Error::Syntax(
                ParseErrorCode::ObjectDuplicateKey(current.key.to_string()),
                current.pos,
            ));
        }
    }
    Ok(())
}

fn write_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

#[inline]
fn skip_ascii_whitespace(buf: &[u8], mut idx: usize) -> usize {
    if idx >= buf.len() || !is_json_whitespace(buf[idx]) {
        return idx;
    }

    while buf.len().saturating_sub(idx) >= LANES {
        let chunk = unsafe { u8x32::from_slice_unaligned_unchecked(&buf[idx..]) };
        let whitespace = chunk.eq(&u8x32::splat(b' '))
            | chunk.eq(&u8x32::splat(b'\n'))
            | chunk.eq(&u8x32::splat(b'\r'))
            | chunk.eq(&u8x32::splat(b'\t'))
            | chunk.eq(&u8x32::splat(0x0c));
        let mask = whitespace.bitmask();
        if mask != u32::MAX {
            return idx + (!mask).trailing_zeros() as usize;
        }
        idx += LANES;
    }

    while idx < buf.len() && buf[idx].is_ascii_whitespace() {
        idx += 1;
    }
    idx
}

#[inline(always)]
fn is_json_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\n' | b'\r' | b'\t' | 0x0c)
}

#[inline]
fn scan_digits(buf: &[u8], mut idx: usize) -> usize {
    while buf.len().saturating_sub(idx) >= LANES {
        let chunk = unsafe { u8x32::from_slice_unaligned_unchecked(&buf[idx..]) };
        let digit = chunk.gt(&u8x32::splat(b'0' - 1)) & chunk.le(&u8x32::splat(b'9'));
        let mask = digit.bitmask();
        if mask != u32::MAX {
            return idx + (!mask).trailing_zeros() as usize;
        }
        idx += LANES;
    }

    while idx < buf.len() && buf[idx].is_ascii_digit() {
        idx += 1;
    }
    idx
}

#[inline]
fn scan_integer_digits_u64(buf: &[u8], mut idx: usize) -> IntegerScan {
    let start = idx;
    let mut value = 0_u64;
    let mut overflow = false;

    while buf.len().saturating_sub(idx) >= 8
        && buf[idx + 7].is_ascii_digit()
        && is_eight_digits(&buf[idx..])
    {
        if !overflow {
            let chunk = parse_eight_digits(&buf[idx..]) as u64;
            match value
                .checked_mul(100_000_000)
                .and_then(|value| value.checked_add(chunk))
            {
                Some(next) => value = next,
                None => overflow = true,
            }
        }
        idx += 8;
    }

    while idx < buf.len() && buf[idx].is_ascii_digit() {
        if !overflow {
            let digit = (buf[idx] - b'0') as u64;
            match value
                .checked_mul(10)
                .and_then(|value| value.checked_add(digit))
            {
                Some(next) => value = next,
                None => overflow = true,
            }
        }
        idx += 1;
    }

    IntegerScan {
        end: idx,
        value,
        digits: idx - start,
        overflow,
    }
}

#[inline(always)]
fn is_eight_digits(src: &[u8]) -> bool {
    debug_assert!(src.len() >= 8);
    let val = u64::from_le_bytes(src[..8].try_into().unwrap());
    let lower = val.wrapping_sub(0x3030_3030_3030_3030);
    let upper = val.wrapping_add(0x4646_4646_4646_4646);
    (lower | upper) & 0x8080_8080_8080_8080 == 0
}

#[inline(always)]
fn parse_eight_digits(src: &[u8]) -> u32 {
    debug_assert!(src.len() >= 8);
    let mut val = u64::from_le_bytes(src[..8].try_into().unwrap());
    val -= 0x3030_3030_3030_3030;
    val = (val.wrapping_mul(10).wrapping_add(val >> 8)) & 0x00ff_00ff_00ff_00ff;
    val = (val.wrapping_mul(100).wrapping_add(val >> 16)) & 0x0000_ffff_0000_ffff;
    val = val.wrapping_mul(10000).wrapping_add(val >> 32);
    val as u32
}

#[inline]
fn scan_string<const CHECK_CONTROL: bool>(
    buf: &[u8],
    mut idx: usize,
    end_quote: u8,
) -> std::result::Result<StringScan, StringScanError> {
    let mut has_escape = false;
    while buf.len().saturating_sub(idx) >= LANES {
        let chunk = unsafe { u8x32::from_slice_unaligned_unchecked(&buf[idx..]) };
        let mut special = chunk.eq(&u8x32::splat(end_quote)) | chunk.eq(&u8x32::splat(b'\\'));
        if CHECK_CONTROL {
            special |= chunk.le(&u8x32::splat(0x1f));
        }
        let mask = special.bitmask();
        if mask == 0 {
            idx += LANES;
            continue;
        }

        idx += mask.trailing_zeros() as usize;
        match buf[idx] {
            byte if byte == end_quote => {
                return Ok(StringScan {
                    end: idx,
                    has_escape,
                })
            }
            b'\\' => {
                has_escape = true;
                idx = skip_escape(buf, idx)?;
            }
            _ => return Err(StringScanError::ControlCharacter(idx)),
        }
    }

    while idx < buf.len() {
        match buf[idx] {
            byte if byte == end_quote => {
                return Ok(StringScan {
                    end: idx,
                    has_escape,
                })
            }
            b'\\' => {
                has_escape = true;
                idx = skip_escape(buf, idx)?;
            }
            0x00..=0x1f if CHECK_CONTROL => return Err(StringScanError::ControlCharacter(idx)),
            _ => idx += 1,
        }
    }
    Err(StringScanError::Eof)
}

#[inline]
fn skip_escape(buf: &[u8], slash_idx: usize) -> std::result::Result<usize, StringScanError> {
    let escaped_idx = slash_idx + 1;
    if escaped_idx >= buf.len() {
        return Err(StringScanError::Eof);
    }

    let mut idx = escaped_idx + 1;
    if buf[escaped_idx] == b'u' {
        if idx >= buf.len() {
            return Err(StringScanError::Eof);
        }

        idx += if buf[idx] == b'{' { 6 } else { 4 };
        if idx > buf.len() {
            return Err(StringScanError::Eof);
        }
    }
    Ok(idx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_owned_jsonb;
    use crate::parse_owned_jsonb_standard_mode;

    fn assert_direct_matches_current(input: &str) {
        let current = parse_owned_jsonb(input.as_bytes()).unwrap();
        let direct = parse_owned_jsonb_direct(input.as_bytes()).unwrap();
        assert_eq!(
            direct.as_raw().to_value().unwrap(),
            current.as_raw().to_value().unwrap()
        );
        assert_eq!(direct.as_ref(), current.as_ref());
    }

    fn assert_direct_standard_matches_current(input: &str) {
        let current = parse_owned_jsonb_standard_mode(input.as_bytes()).unwrap();
        let direct = parse_owned_jsonb_standard_mode_direct(input.as_bytes()).unwrap();
        assert_eq!(
            direct.as_raw().to_value().unwrap(),
            current.as_raw().to_value().unwrap()
        );
        assert_eq!(direct.as_ref(), current.as_ref());
    }

    #[test]
    fn test_direct_writer_standard_values() {
        for input in [
            "null",
            "true",
            "false",
            "123",
            "-42",
            "1.25",
            r#""hello\nworld""#,
            r#""unicode:\u0041\uD834\uDD1E""#,
            r#"[1,2,3,{"b":true,"a":null}]"#,
            r#"{"z":1,"a":[true,false,null],"s":"text"}"#,
        ] {
            assert_direct_standard_matches_current(input);
        }
    }

    #[test]
    fn test_direct_writer_standard_numbers() {
        for input in [
            "0",
            "-0",
            "18446744073709551615",
            "18446744073709551616",
            "-9223372036854775808",
            "9223372036854775808",
            "1.2345",
            "-0.001",
            "1e10",
            "-2.5E-7",
            "[0,-0,10.5,1e2]",
        ] {
            assert_direct_standard_matches_current(input);
        }
    }

    #[test]
    fn test_direct_writer_simd_whitespace_and_delimiters() {
        let spaces = " \n\r\t\u{000c}".repeat(16);
        let input = format!(
            "{{{spaces}\"b\"{spaces}:{spaces}[1{spaces},{spaces}2]{spaces},{spaces}\"a\"{spaces}:{spaces}true{spaces}}}"
        );
        assert_direct_standard_matches_current(&input);
    }

    #[test]
    fn test_direct_writer_extended_values() {
        for input in [
            "",
            "NULL",
            "True",
            "NaN",
            "Infinity",
            "-Infinity",
            "+42",
            "0x7f",
            "-0x7f",
            "0x1a.f",
            ".125",
            "123.",
            "000123",
            "18446744073709551616",
            "-9223372036854775808",
            "-9223372036854775809",
            r#"'single quoted'"#,
            r#"[1,,3,]"#,
            r#"[1 \n, \t, 3 \x0C,]"#,
            r#"{z: 1, a: 'value', nested: {beta: true, alpha: null}}"#,
        ] {
            assert_direct_matches_current(input);
        }
    }

    #[test]
    fn test_direct_writer_large_object_ordering() {
        let mut input = String::from("{");
        for idx in (0..48).rev() {
            if idx != 47 {
                input.push(',');
            }
            input.push_str(&format!(r#""key_{idx:02}":{idx}"#));
        }
        input.push('}');
        assert_direct_standard_matches_current(&input);
    }

    #[test]
    fn test_direct_writer_duplicate_key_error() {
        assert!(parse_owned_jsonb_direct(br#"{"a":1,"a":2}"#).is_err());
        assert!(parse_owned_jsonb_direct(br#"{a:1,a:2}"#).is_err());
    }
}
