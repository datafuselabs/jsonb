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

//! `jsonb` is a binary format `JSON` representation inspired by [PostgreSQL](https://www.postgresql.org/docs/current/datatype-json.html) and [CockroachDB](https://www.cockroachlabs.com/docs/stable/jsonb). It provides a fast, lightweight and easy-to-use API for working with `JSON` data.
//!
//! ## Features
//!
//! - Good compatibility: `jsonb` fully supports the `JSON` standard and can be used to store complex data structures.
//! - Fast performance: `jsonb` is designed for high performance, allowing you to work with large `JSON` data sets with ease.
//! - Easy to use: `jsonb` provides a number of built-in functions to support various operations, and also supports the `JSONPath` syntax for selecting and extracting subset elements.
//! - Safe and secure: `jsonb` is written in Rust, which provides memory and thread safety guarantees, making it a safe choice for handling sensitive data.
//!
//! ## Encoding format
//!
//! The `jsonb` encoding format is a tree-like structure. Each node contains a container header, a number of JEntry headers, and nested encoding values.
//!
//! - 32-bit container header. 3 bits identify the type of value, including `scalar`, `object` and `array`, and 29 bits identify the number of JEntries in the `array` or `object`. The root node of the `jsonb` value is always a container header.
//!   - `scalar` container header: `0x20000000`
//!   - `object` container header: `0x40000000`
//!   - `array` container header: `0x80000000`
//! - 32-bit JEntry header. 1 bit identifies whether the JEntry stores a length or an offset, 3 bits identify the type of value, including `null`, `string`, `number`, `false`, `true` and `container`, and the remaining 28 bits identify the length or offset of the encoding value.
//!   - `null` JEntry header: `0x00000000`
//!   - `string` JEntry header: `0x10000000`
//!   - `number` JEntry header: `0x20000000`
//!   - `false` JEntry header: `0x30000000`
//!   - `true` JEntry header: `0x40000000`
//!   - `container` JEntry header `0x50000000`
//! - Encoding value. Different types of JEntry header have different encoding values.
//!   - `null`, `true`, `false`: no encoding value, identified by the JEntry header.
//!   - `string`: a normal UTF-8 string.
//!   - `number`: an encoded number to represent uint64s, int64s and float64s.
//!   - `container`: a nested `json` value with a recursive structure.
//!
//! #### An encoding example
//!
//! ```text
//! // JSON value
//! [false, 10, {"k":"v"}]
//!
//! // JSONB encoding
//! 0x80000003    array container header (3 JEntries)
//! 0x30000000    false JEntry header (no encoding value)
//! 0x20000002    number JEntry header (encoding value length 2)
//! 0x5000000e    container JEntry header (encoding value length 14)
//! 0x500a        number encoding value (10)
//! 0x40000001    object container header (1 JEntry)
//! 0x10000001    string key JEntry header (encoding value length 1)
//! 0x10000001    string value JEntry header (encoding value length 1)
//! 0x6b          string encoding value ("k")
//! 0x76          string encoding value ("v")
//! ```

#![allow(clippy::uninlined_format_args)]

mod constants;
pub mod core;
mod error;
mod extension;
mod from;
mod functions;
pub mod jaq;
pub mod jsonpath;
pub mod keypath;
mod number;
mod owned;
mod parser;
#[allow(dead_code)]
mod parser_direct;
mod raw;
mod util;
mod value;

pub use error::Error;
pub use extension::*;
#[allow(unused_imports)]
pub use from::*;
pub use number::Decimal128;
pub use number::Decimal256;
pub use number::Decimal64;
pub use number::Number;
pub use owned::to_owned_jsonb;
pub use owned::OwnedJsonb;
pub use parser::from_slice;
pub use parser::parse_owned_jsonb;
pub use parser::parse_owned_jsonb_standard_mode;
pub use parser::parse_owned_jsonb_standard_mode_with_buf;
pub use parser::parse_owned_jsonb_with_buf;
pub use parser::parse_value;
pub use parser::parse_value_standard_mode;
pub use parser_direct::parse_owned_jsonb_direct;
pub use parser_direct::parse_owned_jsonb_standard_mode_direct;
pub use raw::from_raw_jsonb;
pub use raw::RawJsonb;
pub use value::*;
