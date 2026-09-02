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
use std::fmt::Display;
use std::fmt::Formatter;
use std::fmt::Write;

use nom::branch::alt;
use nom::character::complete::char;
use nom::character::complete::i32;
use nom::character::complete::multispace0;
use nom::combinator::map;
use nom::multi::separated_list1;
use nom::sequence::delimited;
use nom::sequence::preceded;
use nom::sequence::terminated;
use nom::IResult;
use nom::Parser;

use crate::jsonpath::raw_string;
use crate::jsonpath::string;
use crate::Error;

/// Represents a set of key path chains.
/// Compatible with PostgreSQL extracts JSON sub-object paths syntax.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct KeyPaths<'a> {
    pub paths: Vec<KeyPath<'a>>,
}

/// Represents a valid key path.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub enum KeyPath<'a> {
    /// represents the index of an Array, allow negative indexing.
    Index(i32),
    /// represents the quoted field name of an Object.
    QuotedName(Cow<'a, str>),
    /// represents the field name of an Object.
    Name(Cow<'a, str>),
}

/// Represents a set of owned key path chains.
#[derive(
    Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct OwnedKeyPaths {
    pub paths: Vec<OwnedKeyPath>,
}

/// Represents a valid owned key path. Quoting is a serialization detail, so
/// quoted and unquoted object keys share the same `Name` representation.
#[derive(
    Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum OwnedKeyPath {
    /// represents the index of an Array, allow negative indexing.
    Index(i32),
    /// represents the field name of an Object.
    Name(String),
}

impl Display for KeyPaths<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{{")?;
        for (i, path) in self.paths.iter().enumerate() {
            if i > 0 {
                write!(f, ",")?;
            }
            write!(f, "{path}")?;
        }
        write!(f, "}}")?;
        Ok(())
    }
}

impl Display for KeyPath<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyPath::Index(idx) => {
                write!(f, "{idx}")?;
            }
            KeyPath::QuotedName(name) => {
                write!(f, "\"{name}\"")?;
            }
            KeyPath::Name(name) => {
                write!(f, "{name}")?;
            }
        }
        Ok(())
    }
}

impl Display for OwnedKeyPaths {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{{")?;
        for (i, path) in self.paths.iter().enumerate() {
            if i > 0 {
                write!(f, ",")?;
            }
            write!(f, "{path}")?;
        }
        write!(f, "}}")?;
        Ok(())
    }
}

impl Display for OwnedKeyPath {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            OwnedKeyPath::Index(idx) => {
                write!(f, "{idx}")?;
            }
            OwnedKeyPath::Name(name) => {
                write!(f, "{name}")?;
            }
        }
        Ok(())
    }
}

impl<'a> KeyPaths<'a> {
    pub fn to_owned(&self) -> OwnedKeyPaths {
        OwnedKeyPaths {
            paths: self.paths.iter().map(KeyPath::to_owned).collect(),
        }
    }
}

impl OwnedKeyPaths {
    pub fn from_key_path_slice(key_paths: &[KeyPath<'_>]) -> Self {
        Self {
            paths: key_paths.iter().map(KeyPath::to_owned).collect(),
        }
    }

    pub fn as_key_paths(&self) -> KeyPaths<'_> {
        KeyPaths {
            paths: self.paths.iter().map(OwnedKeyPath::as_key_path).collect(),
        }
    }

    /// Encode this path into the compact canonical form used by virtual-column
    /// metadata. Identifier-like keys use dot notation, array indexes use
    /// brackets, and only keys containing special characters are quoted.
    ///
    /// Examples: `user.name`, `users[0].id`, `user.'profile.name'`.
    pub fn to_canonical_path(&self) -> String {
        let mut encoded = String::new();
        for path in &self.paths {
            match path {
                OwnedKeyPath::Index(index) => {
                    write!(encoded, "[{index}]").unwrap();
                }
                OwnedKeyPath::Name(name) => {
                    if !encoded.is_empty() {
                        encoded.push('.');
                    }
                    if is_ident(name) {
                        encoded.push_str(name);
                    } else {
                        encoded.push('\'');
                        for ch in name.chars() {
                            if ch == '\\' || ch == '\'' {
                                encoded.push('\\');
                            }
                            encoded.push(ch);
                        }
                        encoded.push('\'');
                    }
                }
            }
        }
        encoded
    }

    /// Decode the compact canonical representation produced by
    /// [`Self::to_canonical_path`].
    pub fn from_canonical_path(path: &str) -> Result<Self, Error> {
        decode_canonical_path(path).ok_or(Error::InvalidKeyPath)
    }
}

impl<'a> KeyPath<'a> {
    pub fn to_owned(&self) -> OwnedKeyPath {
        match self {
            KeyPath::Index(idx) => OwnedKeyPath::Index(*idx),
            KeyPath::QuotedName(name) | KeyPath::Name(name) => OwnedKeyPath::Name(name.to_string()),
        }
    }
}

impl OwnedKeyPath {
    pub fn as_key_path(&self) -> KeyPath<'_> {
        match self {
            OwnedKeyPath::Index(idx) => KeyPath::Index(*idx),
            OwnedKeyPath::Name(name) => KeyPath::Name(Cow::Borrowed(name.as_str())),
        }
    }
}

fn decode_canonical_path(path: &str) -> Option<OwnedKeyPaths> {
    let bytes = path.as_bytes();
    let mut index = 0;
    let mut paths = Vec::new();
    while index < bytes.len() {
        if bytes[index] == b'[' {
            index += 1;
            let start = index;
            if index < bytes.len() && bytes[index] == b'-' {
                index += 1;
            }
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
            if start == index || (bytes.get(start) == Some(&b'-') && start + 1 == index) {
                return None;
            }
            if index >= bytes.len() || bytes[index] != b']' {
                return None;
            }
            let value = std::str::from_utf8(&bytes[start..index])
                .ok()?
                .parse::<i32>()
                .ok()?;
            paths.push(OwnedKeyPath::Index(value));
            index += 1;
            continue;
        }

        if !paths.is_empty() {
            if bytes[index] != b'.' {
                return None;
            }
            index += 1;
            if index >= bytes.len() {
                return None;
            }
        }

        if bytes[index] == b'\'' {
            index += 1;
            let mut name = String::new();
            loop {
                let rest = path.get(index..)?;
                if rest.starts_with('\\') {
                    let escaped = rest.chars().nth(1)?;
                    if escaped != '\\' && escaped != '\'' {
                        return None;
                    }
                    name.push(escaped);
                    index += 1 + escaped.len_utf8();
                } else if rest.starts_with('\'') {
                    index += 1;
                    break;
                } else {
                    let ch = rest.chars().next()?;
                    name.push(ch);
                    index += ch.len_utf8();
                }
            }
            paths.push(OwnedKeyPath::Name(name));
            continue;
        }

        let rest = path.get(index..)?;
        let mut chars = rest.chars();
        let first = chars.next()?;
        if !is_ident_start(first) {
            return None;
        }
        let mut end = index + first.len_utf8();
        for ch in chars {
            if !is_ident_continue(ch) {
                break;
            }
            end += ch.len_utf8();
        }
        paths.push(OwnedKeyPath::Name(path[index..end].to_string()));
        index = end;
    }
    (!paths.is_empty()).then_some(OwnedKeyPaths { paths })
}

fn is_ident(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(is_ident_start) && chars.all(is_ident_continue)
}

fn is_ident_start(ch: char) -> bool {
    ch == '_' || ch.is_alphabetic()
}

fn is_ident_continue(ch: char) -> bool {
    ch == '_' || ch.is_alphanumeric()
}

/// Parsing the input string to key paths.
pub fn parse_key_paths(input: &[u8]) -> Result<KeyPaths<'_>, Error> {
    match key_paths(input) {
        Ok((rest, paths)) => {
            if !rest.is_empty() {
                return Err(Error::InvalidKeyPath);
            }
            let key_paths = KeyPaths { paths };
            Ok(key_paths)
        }
        Err(nom::Err::Error(_) | nom::Err::Failure(_)) => Err(Error::InvalidKeyPath),
        Err(nom::Err::Incomplete(_)) => unreachable!(),
    }
}

fn key_path(input: &[u8]) -> IResult<&[u8], KeyPath<'_>> {
    alt((
        map(i32, KeyPath::Index),
        map(string, KeyPath::QuotedName),
        map(raw_string, KeyPath::Name),
    ))
    .parse(input)
}

fn key_paths(input: &[u8]) -> IResult<&[u8], Vec<KeyPath<'_>>> {
    alt((
        delimited(
            preceded(multispace0, char('{')),
            separated_list1(char(','), delimited(multispace0, key_path, multispace0)),
            terminated(char('}'), multispace0),
        ),
        map(
            delimited(
                preceded(multispace0, char('{')),
                multispace0,
                terminated(char('}'), multispace0),
            ),
            |_| vec![],
        ),
    ))
    .parse(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_owned_key_paths_canonical_roundtrip() {
        let paths = OwnedKeyPaths {
            paths: vec![
                OwnedKeyPath::Name("user".to_string()),
                OwnedKeyPath::Name("profile.name".to_string()),
                OwnedKeyPath::Index(0),
                OwnedKeyPath::Name("it's".to_string()),
            ],
        };
        let encoded = paths.to_canonical_path();
        assert_eq!(encoded, "user.'profile.name'[0].'it\\'s'");
        assert_eq!(OwnedKeyPaths::from_canonical_path(&encoded).unwrap(), paths);
    }

    #[test]
    fn test_quoted_and_unquoted_paths_share_owned_identity() {
        let quoted = KeyPath::QuotedName(Cow::Borrowed("name")).to_owned();
        let unquoted = KeyPath::Name(Cow::Borrowed("name")).to_owned();
        assert_eq!(quoted, unquoted);
        assert_eq!(quoted, OwnedKeyPath::Name("name".to_string()));
    }
}
