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
use std::cmp::Ordering;
use std::fmt::Display;
use std::fmt::Formatter;

use crate::number::Number;

/// Represents a set of JSON Path chains.
#[derive(Debug, Clone, PartialEq)]
pub struct JsonPath<'a> {
    pub paths: Vec<Path<'a>>,
}

impl<'a> JsonPath<'a> {
    pub fn is_predicate(&self) -> bool {
        self.paths.len() == 1 && matches!(self.paths[0], Path::Predicate(_))
    }
}

/// Represents a valid JSON Path.
#[derive(Debug, Clone, PartialEq)]
pub enum Path<'a> {
    /// `$` represents the root node or element.
    Root,
    /// `@` represents the current node or element being processed in the filter expression.
    Current,
    /// `.*` represents selecting all elements in an Object.
    DotWildcard,
    /// `[*]` represents selecting all elements in an Array.
    BracketWildcard,
    /// `.<name>` represents selecting element that matched the name in an Object, like `$.event`.
    /// The name can also be written as a string literal, allowing the name to contain special characters, like `$." $price"`.
    DotField(Cow<'a, str>),
    /// `:<name>` represents selecting element that matched the name in an Object, like `$:event`.
    ColonField(Cow<'a, str>),
    /// `["<name>"]` represents selecting element that matched the name in an Object, like `$["event"]`.
    ObjectField(Cow<'a, str>),
    /// `[<index1>,<index2>,..]` represents selecting elements specified by the indices in an Array.
    /// There are several forms of index.
    /// 1. A single number representing the 0-based `n-th` element in the Array.
    ///    e.g. `$[0]` represents the first element in an Array.
    /// 2. The keyword `last` represents the last element in the Array,
    ///    and last minus a number represents the n-th element before the last element,
    ///    e.g. `$[last-1]` represents the penultimate element.
    /// 3. The keyword `to` between two numbers represent all elements of a range in an Array,
    ///    e.g. `$[1 to last]` represents all the elements in the Array from the second to the last.
    ///
    /// There can be more than one index, e.g. `$[0, last-1 to last, 5]` represents the first,
    /// the last two, and the sixth element in an Array.
    ArrayIndices(Vec<ArrayIndex>),
    /// `?(<expression>)` represents selecting all elements in an object or array that match the filter expression, like `$.book[?(@.price < 10)]`.
    FilterExpr(Box<Expr<'a>>),
    /// `<expression>` standalone filter expression, like `$.book[*].price > 10`.
    Predicate(Box<Expr<'a>>),
}

/// Represents the single index in an Array.
#[derive(Debug, Clone, PartialEq)]
pub enum Index {
    /// The 0-based index in an Array.
    Index(i32),
    /// The last n-th index in an Array.
    LastIndex(i32),
}

/// Represents the index in an Array.
#[derive(Debug, Clone, PartialEq)]
pub enum ArrayIndex {
    /// The single number index.
    Index(Index),
    /// The range index between two number.
    Slice((Index, Index)),
}

/// Represents a literal value used in filter expression.
#[derive(Debug, Clone, PartialEq, PartialOrd)]
pub enum PathValue<'a> {
    /// Null value.
    Null,
    /// Boolean value.
    Boolean(bool),
    /// Number value.
    Number(Number),
    /// UTF-8 string.
    String(Cow<'a, str>),
}

/// Represents the operators used in filter expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinaryOperator {
    /// `&&` represents logical And operation.
    And,
    /// `||` represents logical Or operation.
    Or,
    /// `==` represents left is equal to right.
    Eq,
    /// `!=` and `<>` represents left is not equal to right.
    NotEq,
    /// `<` represents left is less than right.
    Lt,
    /// `<=` represents left is less or equal to right.
    Lte,
    /// `>` represents left is greater than right.
    Gt,
    /// `>=` represents left is greater than or equal to right.
    Gte,
}

/// Represents a filter expression used to filter Array or Object.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr<'a> {
    /// JSON Path chains.
    Paths(Vec<Path<'a>>),
    /// Literal value.
    Value(Box<PathValue<'a>>),
    /// Filter expression that performs a binary operation, returns a boolean value.
    BinaryOp {
        op: BinaryOperator,
        left: Box<Expr<'a>>,
        right: Box<Expr<'a>>,
    },
    /// Filter function, returns a boolean value.
    FilterFunc(FilterFunc<'a>),
}

/// Represents filter function, returns a boolean value.
#[derive(Debug, Clone, PartialEq)]
pub enum FilterFunc<'a> {
    Exists(Vec<Path<'a>>),
    StartsWith(Cow<'a, str>),
}

impl<'a> Display for JsonPath<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        for path in &self.paths {
            write!(f, "{path}")?;
        }
        Ok(())
    }
}

impl Display for Index {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Index::Index(idx) => {
                write!(f, "{idx}")?;
            }
            Index::LastIndex(idx) => {
                write!(f, "last")?;
                match idx.cmp(&0) {
                    Ordering::Greater => {
                        write!(f, "+{idx}")?;
                    }
                    Ordering::Less => {
                        write!(f, "{idx}")?;
                    }
                    Ordering::Equal => {}
                }
            }
        }
        Ok(())
    }
}

impl Display for ArrayIndex {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ArrayIndex::Index(idx) => {
                write!(f, "{idx}")?;
            }
            ArrayIndex::Slice((start, end)) => {
                write!(f, "{start} to {end}")?;
            }
        }
        Ok(())
    }
}

impl<'a> Display for Path<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Path::Root => {
                write!(f, "$")?;
            }
            Path::Current => {
                write!(f, "@")?;
            }
            Path::DotWildcard => {
                write!(f, ".*")?;
            }
            Path::BracketWildcard => {
                write!(f, "[*]")?;
            }
            Path::ColonField(field) => {
                write!(f, ":{field}")?;
            }
            Path::DotField(field) => {
                write!(f, ".{field}")?;
            }
            Path::ObjectField(field) => {
                write!(f, "[\"{field}\"]")?;
            }
            Path::ArrayIndices(indices) => {
                write!(f, "[")?;
                for (i, index) in indices.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{index}")?;
                }
                write!(f, "]")?;
            }
            Path::FilterExpr(expr) => {
                write!(f, "?({expr})")?;
            }
            Path::Predicate(expr) => {
                write!(f, "{expr}")?;
            }
        }
        Ok(())
    }
}

impl<'a> Display for PathValue<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            PathValue::Null => {
                write!(f, "null")
            }
            PathValue::Boolean(v) => {
                if *v {
                    write!(f, "true")
                } else {
                    write!(f, "false")
                }
            }
            PathValue::Number(v) => {
                write!(f, "{v}")
            }
            PathValue::String(v) => {
                write!(f, "\"{v}\"")
            }
        }
    }
}

impl Display for BinaryOperator {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            BinaryOperator::And => {
                write!(f, "&&")
            }
            BinaryOperator::Or => {
                write!(f, "||")
            }
            BinaryOperator::Eq => {
                write!(f, "==")
            }
            BinaryOperator::NotEq => {
                write!(f, "!=")
            }
            BinaryOperator::Lt => {
                write!(f, "<")
            }
            BinaryOperator::Lte => {
                write!(f, "<=")
            }
            BinaryOperator::Gt => {
                write!(f, ">")
            }
            BinaryOperator::Gte => {
                write!(f, ">=")
            }
        }
    }
}

impl<'a> Display for Expr<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Expr::Paths(paths) => {
                for path in paths {
                    write!(f, "{path}")?;
                }
            }
            Expr::Value(v) => {
                write!(f, "{v}")?;
            }
            Expr::BinaryOp { op, left, right } => {
                if let Expr::BinaryOp { op: left_op, .. } = &**left {
                    if left_op == &BinaryOperator::And || left_op == &BinaryOperator::Or {
                        write!(f, "({left})")?;
                    } else {
                        write!(f, "{left}")?;
                    }
                } else {
                    write!(f, "{left}")?;
                }
                write!(f, " {op} ")?;
                if let Expr::BinaryOp { op: right_op, .. } = &**right {
                    if right_op == &BinaryOperator::And || right_op == &BinaryOperator::Or {
                        write!(f, "({right})")?;
                    } else {
                        write!(f, "{right}")?;
                    }
                } else {
                    write!(f, "{right}")?;
                }
            }
            Expr::FilterFunc(func) => match func {
                FilterFunc::Exists(paths) => {
                    f.write_str("exists(")?;
                    for path in paths {
                        write!(f, "{path}")?;
                    }
                    f.write_str(")")?;
                }
                FilterFunc::StartsWith(paths) => {
                    f.write_str("starts with ")?;
                    write!(f, "{paths}")?;
                }
            },
        }
        Ok(())
    }
}
