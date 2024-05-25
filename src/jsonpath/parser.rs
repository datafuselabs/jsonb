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

use nom::{
    branch::alt,
    bytes::complete::{tag, tag_no_case},
    character::complete::{char, i32, i64, multispace0, u64},
    combinator::{cond, map, map_res, opt, value},
    error::{Error as NomError, ErrorKind},
    multi::{many0, separated_list1},
    number::complete::double,
    sequence::{delimited, pair, preceded, separated_pair, terminated, tuple},
    IResult,
};

use crate::constants::UNICODE_LEN;
use crate::error::Error;
use crate::jsonpath::path::*;
use crate::number::Number;
use crate::util::parse_string;
use std::borrow::Cow;

/// Parsing the input string to JSON Path.
pub fn parse_json_path(input: &[u8]) -> Result<JsonPath<'_>, Error> {
    match json_path(input) {
        Ok((rest, json_path)) => {
            if !rest.is_empty() {
                return Err(Error::InvalidJsonPath);
            }
            Ok(json_path)
        }
        Err(nom::Err::Error(_) | nom::Err::Failure(_)) => Err(Error::InvalidJsonPath),
        Err(nom::Err::Incomplete(_)) => unreachable!(),
    }
}

fn json_path(input: &[u8]) -> IResult<&[u8], JsonPath<'_>> {
    map(
        delimited(multispace0, predicate_or_paths, multispace0),
        |paths| JsonPath { paths },
    )(input)
}

fn check_escaped(input: &[u8], i: &mut usize) -> bool {
    if *i + 1 >= input.len() {
        return false;
    }
    if input[*i + 1] == b'u' {
        if *i + 5 >= input.len() {
            return false;
        }
        if input[*i + 2] == b'{' {
            if *i + 7 >= input.len() {
                return false;
            }
            *i += UNICODE_LEN + 4;
        } else {
            *i += UNICODE_LEN + 2;
        }
    } else {
        *i += 2;
    }
    true
}

pub(crate) fn raw_string(input: &[u8]) -> IResult<&[u8], Cow<'_, str>> {
    let mut i = 0;
    let mut escapes = 0;
    while i < input.len() {
        let c = input[i];
        match c {
            b'\\' => {
                escapes += 1;
                if !check_escaped(input, &mut i) {
                    return Err(nom::Err::Error(NomError::new(input, ErrorKind::Char)));
                }
            }
            b' ' | b',' | b'.' | b':' | b'{' | b'}' | b'[' | b']' | b'(' | b')' | b'?' | b'@'
            | b'$' | b'|' | b'<' | b'>' | b'!' | b'=' | b'+' | b'-' | b'*' | b'/' | b'%' | b'"'
            | b'\'' => {
                break;
            }
            _ => {
                i += 1;
            }
        }
    }
    if i > 0 {
        if escapes == 0 {
            if let Ok(s) = std::str::from_utf8(&input[..i]) {
                return Ok((&input[i..], Cow::Borrowed(s)));
            }
        } else {
            let data = &input[..i];
            let len = i - escapes;
            let mut idx = 0;
            let s = parse_string(data, len, &mut idx)
                .map_err(|_| nom::Err::Error(NomError::new(input, ErrorKind::Char)))?;
            return Ok((&input[i..], Cow::Owned(s)));
        }
    }
    Err(nom::Err::Error(NomError::new(input, ErrorKind::Char)))
}

pub(crate) fn string(input: &[u8]) -> IResult<&[u8], Cow<'_, str>> {
    if input.is_empty() || input[0] != b'"' {
        return Err(nom::Err::Error(NomError::new(input, ErrorKind::Char)));
    }
    let mut i = 1;
    let mut escapes = 0;
    while i < input.len() {
        let c = input[i];
        match c {
            b'\\' => {
                escapes += 1;
                if !check_escaped(input, &mut i) {
                    return Err(nom::Err::Error(NomError::new(input, ErrorKind::Char)));
                }
            }
            b'"' => {
                break;
            }
            _ => {
                i += 1;
            }
        }
    }
    if i > 1 {
        if escapes == 0 {
            if let Ok(s) = std::str::from_utf8(&input[1..i]) {
                return Ok((&input[i + 1..], Cow::Borrowed(s)));
            }
        } else {
            let data = &input[1..i];
            let len = i - 1 - escapes;
            let mut idx = 1;
            let s = parse_string(data, len, &mut idx)
                .map_err(|_| nom::Err::Error(NomError::new(input, ErrorKind::Char)))?;
            return Ok((&input[i + 1..], Cow::Owned(s)));
        }
    }
    Err(nom::Err::Error(NomError::new(input, ErrorKind::Char)))
}

fn bracket_wildcard(input: &[u8]) -> IResult<&[u8], ()> {
    value(
        (),
        delimited(
            char('['),
            delimited(multispace0, char('*'), multispace0),
            char(']'),
        ),
    )(input)
}

fn colon_field(input: &[u8]) -> IResult<&[u8], Cow<'_, str>> {
    alt((preceded(char(':'), string), preceded(char(':'), raw_string)))(input)
}

fn dot_field(input: &[u8]) -> IResult<&[u8], Cow<'_, str>> {
    alt((preceded(char('.'), string), preceded(char('.'), raw_string)))(input)
}

fn object_field(input: &[u8]) -> IResult<&[u8], Cow<'_, str>> {
    delimited(
        terminated(char('['), multispace0),
        string,
        preceded(multispace0, char(']')),
    )(input)
}

fn index(input: &[u8]) -> IResult<&[u8], Index> {
    alt((
        map(i32, Index::Index),
        map(
            preceded(
                tuple((tag_no_case("last"), multispace0, char('-'), multispace0)),
                i32,
            ),
            |v| Index::LastIndex(v.saturating_neg()),
        ),
        map(
            preceded(
                tuple((tag_no_case("last"), multispace0, char('+'), multispace0)),
                i32,
            ),
            Index::LastIndex,
        ),
        map(tag_no_case("last"), |_| Index::LastIndex(0)),
    ))(input)
}

fn array_index(input: &[u8]) -> IResult<&[u8], ArrayIndex> {
    alt((
        map(
            separated_pair(
                index,
                delimited(multispace0, tag_no_case("to"), multispace0),
                index,
            ),
            |(s, e)| ArrayIndex::Slice((s, e)),
        ),
        map(index, ArrayIndex::Index),
    ))(input)
}

fn array_indices(input: &[u8]) -> IResult<&[u8], Vec<ArrayIndex>> {
    delimited(
        char('['),
        separated_list1(char(','), delimited(multispace0, array_index, multispace0)),
        char(']'),
    )(input)
}

fn inner_path(input: &[u8]) -> IResult<&[u8], Path<'_>> {
    alt((
        value(Path::DotWildcard, tag(".*")),
        value(Path::BracketWildcard, bracket_wildcard),
        map(colon_field, Path::ColonField),
        map(dot_field, Path::DotField),
        map(array_indices, Path::ArrayIndices),
        map(object_field, Path::ObjectField),
    ))(input)
}

// Compatible with Snowflake query syntax, the first field name does not require the leading period
fn pre_path(input: &[u8]) -> IResult<&[u8], Path<'_>> {
    alt((
        value(Path::Root, char('$')),
        map(delimited(multispace0, raw_string, multispace0), |v| {
            Path::DotField(v)
        }),
    ))(input)
}

fn path(input: &[u8]) -> IResult<&[u8], Path<'_>> {
    alt((
        map(delimited(multispace0, inner_path, multispace0), |v| v),
        map(delimited(multispace0, filter_expr, multispace0), |v| {
            Path::FilterExpr(Box::new(v))
        }),
    ))(input)
}

fn predicate_or_paths(input: &[u8]) -> IResult<&[u8], Vec<Path<'_>>> {
    alt((predicate, paths))(input)
}

fn predicate(input: &[u8]) -> IResult<&[u8], Vec<Path<'_>>> {
    map(
        delimited(multispace0, |i| expr_or(i, true), multispace0),
        |v| vec![Path::Predicate(Box::new(v))],
    )(input)
}

fn paths(input: &[u8]) -> IResult<&[u8], Vec<Path<'_>>> {
    map(
        pair(opt(pre_path), many0(path)),
        |(opt_pre_path, mut paths)| {
            if let Some(pre_path) = opt_pre_path {
                paths.insert(0, pre_path);
            }
            paths
        },
    )(input)
}

fn expr_paths(input: &[u8], root_predicate: bool) -> IResult<&[u8], Vec<Path<'_>>> {
    let parse_current = map_res(
        cond(!root_predicate, value(Path::Current, char('@'))),
        |res| match res {
            Some(v) => Ok(v),
            None => Err(NomError::new(input, ErrorKind::Char)),
        },
    );
    map(
        pair(
            alt((value(Path::Root, char('$')), parse_current)),
            many0(delimited(multispace0, inner_path, multispace0)),
        ),
        |(pre_path, mut paths)| {
            paths.insert(0, pre_path);
            paths
        },
    )(input)
}

fn filter_expr(input: &[u8]) -> IResult<&[u8], Expr<'_>> {
    map(
        delimited(
            delimited(char('?'), multispace0, char('(')),
            delimited(multispace0, |i| expr_or(i, false), multispace0),
            char(')'),
        ),
        |v| v,
    )(input)
}

fn op(input: &[u8]) -> IResult<&[u8], BinaryOperator> {
    alt((
        value(BinaryOperator::Eq, tag("==")),
        value(BinaryOperator::NotEq, tag("!=")),
        value(BinaryOperator::NotEq, tag("<>")),
        value(BinaryOperator::Lte, tag("<=")),
        value(BinaryOperator::Lt, char('<')),
        value(BinaryOperator::Gte, tag(">=")),
        value(BinaryOperator::Gt, char('>')),
    ))(input)
}

fn path_value(input: &[u8]) -> IResult<&[u8], PathValue<'_>> {
    alt((
        value(PathValue::Null, tag("null")),
        value(PathValue::Boolean(true), tag("true")),
        value(PathValue::Boolean(false), tag("false")),
        map(u64, |v| PathValue::Number(Number::UInt64(v))),
        map(i64, |v| PathValue::Number(Number::Int64(v))),
        map(double, |v| PathValue::Number(Number::Float64(v))),
        map(string, PathValue::String),
    ))(input)
}

fn inner_expr(input: &[u8], root_predicate: bool) -> IResult<&[u8], Expr<'_>> {
    alt((
        map(|i| expr_paths(i, root_predicate), Expr::Paths),
        map(path_value, |v| Expr::Value(Box::new(v))),
    ))(input)
}

fn expr_atom(input: &[u8], root_predicate: bool) -> IResult<&[u8], Expr<'_>> {
    // TODO, support arithmetic expressions.
    alt((
        map(
            tuple((
                delimited(multispace0, |i| inner_expr(i, root_predicate), multispace0),
                op,
                delimited(multispace0, |i| inner_expr(i, root_predicate), multispace0),
            )),
            |(left, op, right)| Expr::BinaryOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
            },
        ),
        map(
            delimited(
                terminated(char('('), multispace0),
                |i| expr_or(i, root_predicate),
                preceded(multispace0, char(')')),
            ),
            |expr| expr,
        ),
        map(filter_func, Expr::FilterFunc),
    ))(input)
}

fn filter_func(input: &[u8]) -> IResult<&[u8], FilterFunc<'_>> {
    alt((exists, starts_with))(input)
}

fn exists(input: &[u8]) -> IResult<&[u8], FilterFunc<'_>> {
    preceded(
        tag("exists"),
        preceded(
            multispace0,
            delimited(
                terminated(char('('), multispace0),
                map(exists_paths, FilterFunc::Exists),
                preceded(multispace0, char(')')),
            ),
        ),
    )(input)
}

fn exists_paths(input: &[u8]) -> IResult<&[u8], Vec<Path<'_>>> {
    map(
        pair(
            alt((
                value(Path::Root, char('$')),
                value(Path::Current, char('@')),
            )),
            many0(path),
        ),
        |(pre, mut paths)| {
            paths.insert(0, pre);
            paths
        },
    )(input)
}

fn starts_with(input: &[u8]) -> IResult<&[u8], FilterFunc<'_>> {
    preceded(
        tag("starts with"),
        preceded(multispace0, map(string, FilterFunc::StartsWith)),
    )(input)
}

fn expr_and(input: &[u8], root_predicate: bool) -> IResult<&[u8], Expr<'_>> {
    map(
        separated_list1(delimited(multispace0, tag("&&"), multispace0), |i| {
            expr_atom(i, root_predicate)
        }),
        |exprs| {
            let mut expr = exprs[0].clone();
            for right in exprs.iter().skip(1) {
                expr = Expr::BinaryOp {
                    op: BinaryOperator::And,
                    left: Box::new(expr),
                    right: Box::new(right.clone()),
                };
            }
            expr
        },
    )(input)
}

fn expr_or(input: &[u8], root_predicate: bool) -> IResult<&[u8], Expr<'_>> {
    map(
        separated_list1(delimited(multispace0, tag("||"), multispace0), |i| {
            expr_and(i, root_predicate)
        }),
        |exprs| {
            let mut expr = exprs[0].clone();
            for right in exprs.iter().skip(1) {
                expr = Expr::BinaryOp {
                    op: BinaryOperator::Or,
                    left: Box::new(expr),
                    right: Box::new(right.clone()),
                };
            }
            expr
        },
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_starts_with() {
        let input = r#"starts with "Nigel""#;
        let res = starts_with(input.as_bytes()).unwrap();
        assert_eq!(
            res,
            (&b""[..], FilterFunc::StartsWith(Cow::Borrowed("Nigel")))
        );
    }
}
