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

// This file contains functions that dealing with path-based access to JSONB data.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::collections::VecDeque;

use crate::core::ArrayBuilder;
use crate::core::ArrayIterator;
use crate::core::JsonbItem;
use crate::core::JsonbItemType;
use crate::core::ObjectBuilder;
use crate::core::ObjectIterator;
use crate::core::ObjectKeyIterator;
use crate::error::*;
use crate::jsonpath::JsonPath;
use crate::jsonpath::Selector;
use crate::keypath::KeyPath;
use crate::keypath::KeyPaths;
use crate::ExtensionValue;
use crate::OwnedJsonb;
use crate::RawJsonb;
use crate::Value;

impl RawJsonb<'_> {
    /// Gets the element at the specified index in a JSONB array.
    ///
    /// If the JSONB value is an array, this function returns the element at the given `index` as an `OwnedJsonb`.
    /// If the `index` is out of bounds, it returns `Ok(None)`.
    /// If the JSONB value is not an array (e.g., it's an object or a scalar), this function also returns `Ok(None)`.
    ///
    /// # Arguments
    ///
    /// * `self` - The JSONB value.
    /// * `index` - The index of the desired element.
    ///
    /// # Returns
    ///
    /// * `Ok(Some(OwnedJsonb))` - The element at the specified index as an `OwnedJsonb` if the input is an array and the index is valid.
    /// * `Ok(None)` - If the input is not an array, or if the index is out of bounds.
    /// * `Err(Error)` - If an error occurred during decoding (e.g., invalid JSONB data).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use jsonb::OwnedJsonb;
    ///
    /// let arr_jsonb = r#"[1, "hello", {"a": 1}]"#.parse::<OwnedJsonb>().unwrap();
    /// let raw_jsonb = arr_jsonb.as_raw();
    ///
    /// let element0 = raw_jsonb.get_by_index(0).unwrap();
    /// assert_eq!(element0.unwrap().to_string(), "1");
    ///
    /// let element1 = raw_jsonb.get_by_index(1).unwrap();
    /// assert_eq!(element1.unwrap().to_string(), r#""hello""#);
    ///
    /// let element2 = raw_jsonb.get_by_index(2).unwrap();
    /// assert_eq!(element2.unwrap().to_string(), r#"{"a":1}"#);
    ///
    /// let element3 = raw_jsonb.get_by_index(3).unwrap();
    /// assert!(element3.is_none()); // Index out of bounds
    ///
    /// let obj_jsonb = r#"{"a": 1}"#.parse::<OwnedJsonb>().unwrap();
    /// let raw_jsonb = obj_jsonb.as_raw();
    /// let element = raw_jsonb.get_by_index(0).unwrap();
    /// assert!(element.is_none()); // Not an array
    /// ```
    pub fn get_by_index(&self, index: usize) -> Result<Option<OwnedJsonb>> {
        let array_iter_opt = ArrayIterator::new(*self)?;
        if let Some(mut array_iter) = array_iter_opt {
            if let Some(item_result) = array_iter.nth(index) {
                let item = item_result?;
                let value = OwnedJsonb::from_item(item)?;
                return Ok(Some(value));
            }
        }
        Ok(None)
    }

    /// Gets the value associated with a given key in a JSONB object.
    ///
    /// If the JSONB value is an object, this function searches for a key matching the provided `name`
    /// and returns the associated value as an `OwnedJsonb`.
    /// The `ignore_case` parameter controls whether the key search is case-sensitive.
    /// If the key is not found, it returns `Ok(None)`.
    /// If the JSONB value is not an object (e.g., it's an array or a scalar), this function also returns `Ok(None)`.
    ///
    /// # Arguments
    ///
    /// * `self` - The JSONB value.
    /// * `name` - The key to search for.
    /// * `ignore_case` - Whether the key search should be case-insensitive.
    ///
    /// # Returns
    ///
    /// * `Ok(Some(OwnedJsonb))` - The value associated with the key as an `OwnedJsonb`, if the input is an object and the key is found.
    /// * `Ok(None)` - If the input is not an object, or if the key is not found.
    /// * `Err(Error)` - If an error occurred during decoding (e.g., invalid JSONB data).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use jsonb::OwnedJsonb;
    ///
    /// let obj_jsonb = r#"{"a": 1, "b": "hello", "c": [1, 2]}"#.parse::<OwnedJsonb>().unwrap();
    /// let raw_jsonb = obj_jsonb.as_raw();
    ///
    /// let value_a = raw_jsonb.get_by_name("a", false).unwrap();
    /// assert_eq!(value_a.unwrap().to_string(), "1");
    ///
    /// let value_b = raw_jsonb.get_by_name("b", false).unwrap();
    /// assert_eq!(value_b.unwrap().to_string(), r#""hello""#);
    ///
    /// let value_c = raw_jsonb.get_by_name("c", false).unwrap();
    /// assert_eq!(value_c.unwrap().to_string(), "[1,2]");
    ///
    /// let value_d = raw_jsonb.get_by_name("d", false).unwrap();
    /// assert!(value_d.is_none()); // Key not found
    ///
    /// // Case-insensitive search
    /// let value_a_case_insensitive = raw_jsonb.get_by_name("A", true).unwrap();
    /// assert_eq!(value_a_case_insensitive.unwrap().to_string(), "1");
    ///
    /// let arr_jsonb = "[1, 2, 3]".parse::<OwnedJsonb>().unwrap();
    /// let raw_jsonb = arr_jsonb.as_raw();
    /// let value = raw_jsonb.get_by_name("a", false).unwrap();
    /// assert!(value.is_none()); // Not an object
    /// ```
    pub fn get_by_name(&self, name: &str, ignore_case: bool) -> Result<Option<OwnedJsonb>> {
        let key_name = Cow::Borrowed(name);
        if let Some(val_item) =
            self.get_object_value_by_key_name(&key_name, |name, key| key.eq(name))?
        {
            let value = OwnedJsonb::from_item(val_item)?;
            return Ok(Some(value));
        }
        if ignore_case {
            if let Some(val_item) = self.get_object_value_by_key_name(&key_name, |name, key| {
                key.eq_ignore_ascii_case(name)
            })? {
                let value = OwnedJsonb::from_item(val_item)?;
                return Ok(Some(value));
            }
        }
        Ok(None)
    }

    /// Gets the value at the specified key path in a JSONB value.
    ///
    /// This function traverses the JSONB value according to the provided key path
    /// and returns the value at the final path element as an `OwnedJsonb`.
    /// The key path is an iterator of `KeyPath` elements, which can be
    /// either named keys (for objects) or array indices.
    ///
    /// If any element in the key path does not exist or if the type of the current value
    /// does not match the key path element (e.g., trying to access a named key in an array),
    /// the function returns `Ok(None)`.
    /// If the key path is empty, the function returns the original `RawJsonb` value wrapped in `Some`.
    ///
    /// # Arguments
    ///
    /// * `self` - The JSONB value.
    /// * `keypaths` - An iterator of `KeyPath` elements representing the path to traverse.
    ///
    /// # Returns
    ///
    /// * `Ok(Some(OwnedJsonb))` - The value at the specified key path as an `OwnedJsonb`, if found.
    /// * `Ok(None)` - If the key path is invalid or leads to a non-existent value.
    /// * `Err(Error)` - If an error occurred during decoding (e.g., invalid JSONB data).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use std::borrow::Cow;
    /// use jsonb::{keypath::KeyPath, OwnedJsonb, RawJsonb};
    ///
    /// let jsonb_value = r#"{"a": {"b": [1, 2, 3], "c": "hello"}, "d": [4, 5]}"#.parse::<OwnedJsonb>().unwrap();
    /// let raw_jsonb = jsonb_value.as_raw();
    ///
    /// // Accessing nested values
    /// let path = [KeyPath::Name(Cow::Borrowed("a")), KeyPath::Name(Cow::Borrowed("b")), KeyPath::Index(1)];
    /// let value = raw_jsonb.get_by_keypath(path.iter()).unwrap();
    /// assert_eq!(value.unwrap().to_string(), "2");
    ///
    /// let path = [KeyPath::Name(Cow::Borrowed("a")), KeyPath::Name(Cow::Borrowed("c"))];
    /// let value = raw_jsonb.get_by_keypath(path.iter()).unwrap();
    /// assert_eq!(value.unwrap().to_string(), r#""hello""#);
    ///
    /// let path = [KeyPath::Name(Cow::Borrowed("d")), KeyPath::Index(0)];
    /// let value = raw_jsonb.get_by_keypath(path.iter()).unwrap();
    /// assert_eq!(value.unwrap().to_string(), "4");
    ///
    /// // Invalid key path
    /// let path = [KeyPath::Name(Cow::Borrowed("a")), KeyPath::Name(Cow::Borrowed("x"))]; // "x" doesn't exist
    /// let value = raw_jsonb.get_by_keypath(path.iter()).unwrap();
    /// assert!(value.is_none());
    ///
    /// let path = [KeyPath::Name(Cow::Borrowed("a")), KeyPath::Index(0)]; // "a" is an object, not an array
    /// let value = raw_jsonb.get_by_keypath(path.iter()).unwrap();
    /// assert!(value.is_none());
    ///
    /// // Empty key path - returns the original value
    /// let value = raw_jsonb.get_by_keypath([].iter()).unwrap();
    /// assert_eq!(value.unwrap().to_string(), r#"{"a":{"b":[1,2,3],"c":"hello"},"d":[4,5]}"#);
    ///
    /// // KeyPath with quoted name
    /// let jsonb_value = r#"{"a b": 1}"#.parse::<OwnedJsonb>().unwrap();
    /// let raw_jsonb = jsonb_value.as_raw();
    /// let path = [KeyPath::QuotedName(Cow::Borrowed("a b"))];
    /// let value = raw_jsonb.get_by_keypath(path.iter()).unwrap();
    /// assert_eq!(value.unwrap().to_string(), r#"1"#);
    /// ```
    pub fn get_by_keypath<'a, I: Iterator<Item = &'a KeyPath<'a>>>(
        &self,
        keypaths: I,
    ) -> Result<Option<OwnedJsonb>> {
        let mut current_item = JsonbItem::Raw(*self);
        for path in keypaths {
            let Some(current) = current_item.as_raw_jsonb() else {
                return Ok(None);
            };
            let jsonb_item_type = current.jsonb_item_type()?;
            match jsonb_item_type {
                JsonbItemType::Array(_) => {
                    if let KeyPath::Index(index) = path {
                        let array_iter_opt = ArrayIterator::new(current)?;
                        if let Some(mut array_iter) = array_iter_opt {
                            let length = array_iter.len() as i32;
                            if *index > length || length + *index < 0 {
                                return Ok(None);
                            }
                            let index = if *index >= 0 {
                                *index as usize
                            } else {
                                (length + *index) as usize
                            };
                            if let Some(item_result) = array_iter.nth(index) {
                                let item = item_result?;
                                current_item = item;
                                continue;
                            }
                        }
                    }
                    return Ok(None);
                }
                JsonbItemType::Object(_) => {
                    let name: Cow<'a, str> = match path {
                        KeyPath::Index(index) => Cow::Owned(index.to_string()),
                        KeyPath::Name(name) | KeyPath::QuotedName(name) => Cow::Borrowed(name),
                    };
                    if let Some(val_item) =
                        current.get_object_value_by_key_name(&name, |name, key| key.eq(name))?
                    {
                        current_item = val_item;
                    } else {
                        return Ok(None);
                    }
                }
                _ => {
                    return Ok(None);
                }
            }
        }
        let value = OwnedJsonb::from_item(current_item)?;
        Ok(Some(value))
    }

    /// Selects elements from the `RawJsonb` by the given `JsonPath`.
    ///
    /// This function returns all matching elements as a `Vec<OwnedJsonb>`.
    ///
    /// # Arguments
    ///
    /// * `self` - The JSONB value.
    /// * `json_path` - The JSONPath expression.
    ///
    /// # Returns
    ///
    /// * `Ok(Vec<OwnedJsonb>)` - A vector containing the selected `OwnedJsonb` values.
    /// * `Err(Error)` - If the JSONB data is invalid or if an error occurs during path evaluation.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use jsonb::OwnedJsonb;
    /// use jsonb::jsonpath::parse_json_path;
    ///
    /// let jsonb_value = r#"{"a": {"b": [1, 2, 3]}, "c": 4}"#.parse::<OwnedJsonb>().unwrap();
    /// let raw_jsonb = jsonb_value.as_raw();
    ///
    /// let path = parse_json_path("$.a.b[*]".as_bytes()).unwrap();
    /// let result = raw_jsonb.select_by_path(&path).unwrap();
    /// assert_eq!(result.len(), 3);
    /// assert_eq!(result[0].to_string(), "1");
    /// assert_eq!(result[1].to_string(), "2");
    /// assert_eq!(result[2].to_string(), "3");
    /// ```
    pub fn select_by_path<'a>(&self, json_path: &'a JsonPath<'a>) -> Result<Vec<OwnedJsonb>> {
        let mut selector = Selector::new(*self);
        selector.select_values(json_path)
    }

    /// Selects elements from the `RawJsonb` by the given `JsonPath` and wraps them in a JSON array.
    ///
    /// This function returns all matching elements as a single `OwnedJsonb` representing a JSON array.
    ///
    /// # Arguments
    ///
    /// * `self` - The JSONB value.
    /// * `json_path` - The JSONPath expression.
    ///
    /// # Returns
    ///
    /// * `Ok(OwnedJsonb)` - A single `OwnedJsonb` (a JSON array) containing the selected values.
    /// * `Err(Error)` - If the JSONB data is invalid or if an error occurs during path evaluation.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use jsonb::OwnedJsonb;
    /// use jsonb::jsonpath::parse_json_path;
    ///
    /// let jsonb_value = r#"{"a": {"b": [1, 2, 3]}, "c": 4}"#.parse::<OwnedJsonb>().unwrap();
    /// let raw_jsonb = jsonb_value.as_raw();
    ///
    /// let path = parse_json_path("$.a.b[*]".as_bytes()).unwrap();
    /// let result = raw_jsonb.select_array_by_path(&path).unwrap();
    /// assert_eq!(result.to_string(), "[1,2,3]");
    /// ```
    pub fn select_array_by_path<'a>(&self, json_path: &'a JsonPath<'a>) -> Result<OwnedJsonb> {
        let mut selector = Selector::new(*self);
        selector.select_array(json_path)
    }

    /// Selects the first matching element from the `RawJsonb` by the given `JsonPath`.
    ///
    /// This function returns the first matched element wrapped in `Some`, or `None` if no element matches the path.
    ///
    /// # Arguments
    ///
    /// * `self` - The JSONB value.
    /// * `json_path` - The JSONPath expression.
    ///
    /// # Returns
    ///
    /// * `Ok(Some(OwnedJsonb))` - A single `OwnedJsonb` containing the first matched value.
    /// * `Ok(None)` - The path does not match any values.
    /// * `Err(Error)` - If the JSONB data is invalid or if an error occurs during path evaluation.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use jsonb::OwnedJsonb;
    /// use jsonb::jsonpath::parse_json_path;
    ///
    /// let jsonb_value = r#"{"a": [{"b": 1}, {"b": 2}], "c": 3}"#.parse::<OwnedJsonb>().unwrap();
    /// let raw_jsonb = jsonb_value.as_raw();
    ///
    /// let path = parse_json_path("$.a[0].b".as_bytes()).unwrap(); // Matches multiple values.
    /// let result = raw_jsonb.select_first_by_path(&path).unwrap();
    /// assert_eq!(result.unwrap().to_string(), "1");
    ///
    /// let path = parse_json_path("$.d".as_bytes()).unwrap(); // No match.
    /// let result = raw_jsonb.select_first_by_path(&path).unwrap();
    /// assert!(result.is_none());
    /// ```
    pub fn select_first_by_path<'a>(
        &self,
        json_path: &'a JsonPath<'a>,
    ) -> Result<Option<OwnedJsonb>> {
        let mut selector = Selector::new(*self);
        selector.select_first(json_path)
    }

    /// Selects a value (or an array of values) from the `RawJsonb` by the given `JsonPath`.
    ///
    /// If exactly one element matches, it is returned directly (wrapped in `Some`).
    /// If multiple elements match, they are returned as a JSON array (wrapped in `Some`).
    /// If no elements match, `None` is returned.
    ///
    /// # Arguments
    ///
    /// * `self` - The JSONB value.
    /// * `json_path` - The JSONPath expression.
    ///
    /// # Returns
    ///
    /// * `Ok(Some(OwnedJsonb))` - A single `OwnedJsonb` containing the matched values.
    /// * `Ok(None)` - The path does not match any values.
    /// * `Err(Error)` - If the JSONB data is invalid or if an error occurs during path evaluation.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use jsonb::OwnedJsonb;
    /// use jsonb::jsonpath::parse_json_path;
    ///
    /// let jsonb_value = r#"{"a": [{"b": 1}, {"b": 2}], "c": 3}"#.parse::<OwnedJsonb>().unwrap();
    /// let raw_jsonb = jsonb_value.as_raw();
    ///
    /// let path = parse_json_path("$.c".as_bytes()).unwrap(); // Matches a single value.
    /// let result = raw_jsonb.select_value_by_path(&path).unwrap();
    /// assert_eq!(result.unwrap().to_string(), "3");
    ///
    /// let path = parse_json_path("$.a[*].b".as_bytes()).unwrap(); // Matches multiple values.
    /// let result = raw_jsonb.select_value_by_path(&path).unwrap();
    /// assert_eq!(result.unwrap().to_string(), "[1,2]");
    ///
    /// let path = parse_json_path("$.x".as_bytes()).unwrap(); // No match.
    /// let result = raw_jsonb.select_value_by_path(&path).unwrap();
    /// assert!(result.is_none());
    /// ```
    pub fn select_value_by_path<'a>(
        &self,
        json_path: &'a JsonPath<'a>,
    ) -> Result<Option<OwnedJsonb>> {
        let mut selector = Selector::new(*self);
        selector.select_value(json_path)
    }

    /// Checks if a JSON path exists within the JSONB value.
    ///
    /// # Arguments
    ///
    /// * `self` - The JSONB value.
    /// * `json_path` - The JSONPath expression.
    ///
    /// # Returns
    ///
    /// * `Ok(true)` - If the JSON path exists.
    /// * `Ok(false)` - If the JSON path does not exist.
    /// * `Err(Error)` - If the JSONB data is invalid or if an error occurs during path evaluation.
    ///   This could also indicate issues with the `json_path` itself.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use jsonb::OwnedJsonb;
    /// use jsonb::jsonpath::parse_json_path;
    ///
    /// let jsonb_value = r#"{"a": {"b": [1, 2, 3]}, "c": 4}"#.parse::<OwnedJsonb>().unwrap();
    /// let raw_jsonb = jsonb_value.as_raw();
    ///
    /// // Valid paths
    /// let path1 = parse_json_path("$.a.b[1]".as_bytes()).unwrap();
    /// assert!(raw_jsonb.path_exists(&path1).unwrap());
    ///
    /// let path2 = parse_json_path("$.c".as_bytes()).unwrap();
    /// assert!(raw_jsonb.path_exists(&path2).unwrap());
    ///
    /// // Invalid paths
    /// let path3 = parse_json_path("$.a.x".as_bytes()).unwrap(); // "x" does not exist
    /// assert!(!raw_jsonb.path_exists(&path3).unwrap());
    /// ```
    pub fn path_exists<'a>(&self, json_path: &'a JsonPath<'a>) -> Result<bool> {
        let mut selector = Selector::new(*self);
        selector.exists(json_path)
    }

    /// Checks if a JSON path matches the JSONB value using a predicate.
    ///
    /// This function checks if a given JSON Path, along with an associated predicate, matches the JSONB value.
    /// The predicate determines the conditions that the selected value(s) must satisfy for the match to be considered successful.
    ///
    /// # Arguments
    ///
    /// * `self` - The JSONB value.
    /// * `json_path` - The JSONPath expression with a predicate.
    ///   The predicate is specified within the `json_path` using the standard JSONPath syntax.
    ///   For example, `$.store.book[?(@.price < 10)]` selects books with a price less than 10.
    ///
    /// # Returns
    ///
    /// * `Ok(Some(true))` - If the JSON path with its predicate matches at least one value in the JSONB data.
    /// * `Ok(Some(false))` - If the JSON path with its predicate does not match any values.
    /// * `Ok(None)` - If the JSON path is not a predicate expr or predicate result is not a boolean value.
    /// * `Err(Error)` - If the JSONB data is invalid or if an error occurs during path evaluation or predicate checking.
    ///   This could also indicate issues with the `json_path` itself (invalid syntax, etc.).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use jsonb::jsonpath::parse_json_path;
    /// use jsonb::OwnedJsonb;
    ///
    /// let jsonb_value = r#"[
    ///     {"price": 12, "title": "Book A"},
    ///     {"price": 8, "title": "Book B"},
    ///     {"price": 5, "title": "Book C"}
    /// ]"#
    /// .parse::<OwnedJsonb>()
    /// .unwrap();
    /// let raw_jsonb = jsonb_value.as_raw();
    ///
    /// // Path with predicate (select books with price < 10)
    /// let path = parse_json_path("$[*].price < 10".as_bytes()).unwrap();
    /// assert_eq!(raw_jsonb.path_match(&path).unwrap(), Some(true)); // True because Book B and Book C match.
    ///
    /// // Path with predicate (select books with title "Book D")
    /// let path = parse_json_path("$[*].title == \"Book D\"".as_bytes()).unwrap();
    /// assert_eq!(raw_jsonb.path_match(&path).unwrap(), Some(false)); // False because no book has this title.
    ///
    /// // Path is not a predicate expr
    /// let path = parse_json_path("$[*].title".as_bytes()).unwrap();
    /// assert_eq!(raw_jsonb.path_match(&path).unwrap(), None);
    /// ```
    pub fn path_match<'a>(&self, json_path: &'a JsonPath<'a>) -> Result<Option<bool>> {
        let mut selector = Selector::new(*self);
        selector.predicate_match(json_path)
    }

    /// Deletes the element at the specified index from a JSONB array.
    ///
    /// This function removes the element at the given `index` from a JSONB array.
    /// The `index` can be positive or negative:
    ///
    /// * **Positive index:**  0-based index from the beginning of the array.
    /// * **Negative index:**  1-based index from the end of the array (e.g., -1 refers to the last element).
    ///
    /// If the `index` is out of bounds, the original JSONB array is returned unchanged.
    /// If the input JSONB value is not an array (e.g., it's an object or a scalar), an `Error::InvalidJsonType` is returned.
    /// Other invalid JSONB data results in an `Error::InvalidJsonb`.
    ///
    /// # Arguments
    ///
    /// * `self` - The JSONB value.
    /// * `index` - The index of the element to delete.
    ///
    /// # Returns
    ///
    /// * `Ok(OwnedJsonb)` - The JSONB array with the element at the specified index removed, or the original array if the index is out of bounds.
    /// * `Err(Error)` - If the input JSONB value is not an array, or if the JSONB data is invalid.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use jsonb::OwnedJsonb;
    ///
    /// let arr_jsonb = r#"[1, "hello", 3, 4]"#.parse::<OwnedJsonb>().unwrap();
    /// let raw_jsonb = arr_jsonb.as_raw();
    ///
    /// // Delete element at index 1
    /// let deleted = raw_jsonb.delete_by_index(1).unwrap();
    /// assert_eq!(deleted.to_string(), "[1,3,4]");
    ///
    /// // Delete last element using negative index
    /// let deleted = raw_jsonb.delete_by_index(-1).unwrap();
    /// assert_eq!(deleted.to_string(), "[1,\"hello\",3]");
    ///
    /// // Out of bounds index (positive)
    /// let deleted = raw_jsonb.delete_by_index(4).unwrap();
    /// assert_eq!(deleted.to_string(), "[1,\"hello\",3,4]"); // Original array returned
    ///
    /// // Out of bounds index (negative)
    /// let deleted = raw_jsonb.delete_by_index(-5).unwrap();
    /// assert_eq!(deleted.to_string(), "[1,\"hello\",3,4]"); // Original array returned
    ///
    /// let obj_jsonb = r#"{"a": 1}"#.parse::<OwnedJsonb>().unwrap();
    /// let raw_jsonb = obj_jsonb.as_raw();
    /// let result = raw_jsonb.delete_by_index(0);
    /// assert!(result.is_err()); // Error because input is not an array
    pub fn delete_by_index(&self, index: i32) -> Result<OwnedJsonb> {
        let array_iter_opt = ArrayIterator::new(*self)?;
        if let Some(array_iter) = array_iter_opt {
            let len = array_iter.len() as i32;
            let index = if index < 0 { len - index.abs() } else { index };
            if index < 0 || index >= len {
                Ok(self.to_owned())
            } else {
                let index = index as usize;
                let mut builder = ArrayBuilder::with_capacity(array_iter.len());
                for (i, item_result) in &mut array_iter.enumerate() {
                    let item = item_result?;
                    if i != index {
                        builder.push_jsonb_item(item);
                    }
                }
                Ok(builder.build()?)
            }
        } else {
            Err(Error::InvalidJsonType)
        }
    }

    /// Deletes a key-value pair from a JSONB object or an element from a JSONB array.
    ///
    /// This function removes a key-value pair from a JSONB object if the key matches the given `name`
    /// or removes an element from a JSONB array if the element is a string that matches the given `name`.
    ///
    /// * **Object:** If the input is an object, the key-value pair with the matching key is removed.  The key comparison is case-sensitive.
    /// * **Array:** If the input is an array, elements that are strings and match `name` (case-sensitive) are removed.  Other array elements remain unchanged.
    /// * **Invalid input:** If the input JSONB value is a scalar value, an `Error::InvalidJsonType` is returned. Other invalid JSONB data results in an `Error::InvalidJsonb`.
    ///
    /// # Arguments
    ///
    /// * `self` - The JSONB value.
    /// * `name` - The key (for objects) or string value (for arrays) to match.
    ///
    /// # Returns
    ///
    /// * `Ok(OwnedJsonb)` - The modified JSONB value with the matching key-value pair or element removed.
    /// * `Err(Error)` - If the input JSONB value is a scalar, or if the JSONB data is invalid.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use jsonb::OwnedJsonb;
    ///
    /// // Deleting from an object
    /// let obj_jsonb = r#"{"a": 1, "b": "hello", "c": 3}"#.parse::<OwnedJsonb>().unwrap();
    /// let raw_jsonb = obj_jsonb.as_raw();
    ///
    /// let deleted = raw_jsonb.delete_by_name("b").unwrap();
    /// assert_eq!(deleted.to_string(), r#"{"a":1,"c":3}"#);
    ///
    /// // Deleting from an array (string elements only)
    /// let arr_jsonb = r#"[1, "hello", 3, "world"]"#.parse::<OwnedJsonb>().unwrap();
    /// let raw_jsonb = arr_jsonb.as_raw();
    /// let deleted = raw_jsonb.delete_by_name("hello").unwrap();
    /// assert_eq!(deleted.to_string(), "[1,3,\"world\"]");
    ///
    /// // Non-matching key in object
    /// let deleted = raw_jsonb.delete_by_name("x").unwrap(); // "x" doesn't exist
    /// assert_eq!(deleted.to_string(), r#"[1,"hello",3,"world"]"#); // Original array returned
    ///
    /// // Non-matching value in array
    /// let deleted = arr_jsonb.as_raw().delete_by_name("xyz").unwrap(); // "xyz" doesn't exist
    /// assert_eq!(deleted.to_string(), r#"[1,"hello",3,"world"]"#); // Original array returned
    ///
    /// // Attempting to delete from a scalar
    /// let scalar_jsonb = "1".parse::<OwnedJsonb>().unwrap();
    /// let raw_jsonb = scalar_jsonb.as_raw();
    /// let result = raw_jsonb.delete_by_name("a");
    /// assert!(result.is_err()); // Returns an error
    /// ```
    pub fn delete_by_name(&self, name: &str) -> Result<OwnedJsonb> {
        let jsonb_item_type = self.jsonb_item_type()?;
        match jsonb_item_type {
            JsonbItemType::Object(_) => {
                let mut object_iter = ObjectIterator::new(*self)?.unwrap();
                let mut builder = ObjectBuilder::new();
                for result in &mut object_iter {
                    let (key, val_item) = result?;
                    if !key.eq(name) {
                        builder.push_jsonb_item(key, val_item)?;
                    }
                }
                Ok(builder.build()?)
            }
            JsonbItemType::Array(_) => {
                let mut array_iter = ArrayIterator::new(*self)?.unwrap();
                let mut builder = ArrayBuilder::with_capacity(array_iter.len());
                for item_result in &mut array_iter {
                    let item = item_result?;
                    if let Some(s) = item.as_str() {
                        if s.eq(name) {
                            continue;
                        }
                    }
                    builder.push_jsonb_item(item);
                }
                Ok(builder.build()?)
            }
            _ => Err(Error::InvalidJsonType),
        }
    }

    /// Deletes a value from a JSONB array or object based on a key path.
    ///
    /// This function removes a value from a JSONB array or object using a key path.
    /// The key path is an iterator of `KeyPath` elements specifying the path to the element to delete.
    ///
    /// * **Array:** If the JSONB value is an array, the key path must consist of array indices (`KeyPath::Index`).
    ///   A negative index counts from the end of the array (e.g., -1 is the last element).
    ///   If the index is out of bounds, the original JSONB value is returned unchanged.
    /// * **Object:** If the JSONB value is an object, the key path can be a mix of object keys (`KeyPath::Name` or `KeyPath::QuotedName`) and array indices.
    ///   If any part of the path is invalid (e.g., trying to access an index in a non-array or a key in a non-object), the original JSONB value is returned unchanged.
    /// * **Invalid input:** If the input is neither an array nor an object, or if the JSONB data is otherwise invalid,
    ///   an error (`Error::InvalidJsonType` or `Error::InvalidJsonb`) is returned.
    ///
    /// # Arguments
    ///
    /// * `self` - The JSONB value.
    /// * `keypath` - An iterator of `KeyPath` elements specifying the path to the element to delete.
    ///
    /// # Returns
    ///
    /// * `Ok(OwnedJsonb)` - The JSONB value with the specified element deleted. Returns the original value if the keypath is invalid or leads to a non-existent value.
    /// * `Err(Error)` - If the input JSONB value is neither an array nor an object, or if the JSONB data is invalid.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use std::borrow::Cow;
    ///
    /// use jsonb::OwnedJsonb;
    /// use jsonb::keypath::KeyPath;
    ///
    /// // Deleting from an array
    /// let arr_jsonb = r#"[1, 2, 3]"#.parse::<OwnedJsonb>().unwrap();
    /// let raw_jsonb = arr_jsonb.as_raw();
    /// let keypath = [KeyPath::Index(1)]; // Delete element at index 1
    /// let deleted = raw_jsonb.delete_by_keypath(keypath.iter()).unwrap();
    /// assert_eq!(deleted.to_string(), "[1,3]");
    ///
    /// let keypath = [KeyPath::Index(-1)]; // Delete last element
    /// let deleted = raw_jsonb.delete_by_keypath(keypath.iter()).unwrap();
    /// assert_eq!(deleted.to_string(), "[1,2]");
    ///
    /// // Deleting from an object
    /// let obj_jsonb = r#"{"a": {"b": [1, 2, 3]}, "c": 4}"#.parse::<OwnedJsonb>().unwrap();
    /// let raw_jsonb = obj_jsonb.as_raw();
    /// let keypath = [
    ///     KeyPath::Name(Cow::Borrowed("a")),
    ///     KeyPath::Name(Cow::Borrowed("b")),
    ///     KeyPath::Index(1),
    /// ];
    /// let deleted = raw_jsonb.delete_by_keypath(keypath.iter()).unwrap();
    /// assert_eq!(deleted.to_string(), r#"{"a":{"b":[1,3]},"c":4}"#);
    ///
    /// // Invalid keypath (index out of bounds)
    /// let keypath = [KeyPath::Index(3)];
    /// let deleted = raw_jsonb.delete_by_keypath(keypath.iter()).unwrap();
    /// assert_eq!(deleted.to_string(), r#"{"a":{"b":[1,2,3]},"c":4}"#); // Original value returned
    ///
    /// // Invalid keypath (wrong type)
    /// let keypath = [
    ///     KeyPath::Name(Cow::Borrowed("a")),
    ///     KeyPath::Name(Cow::Borrowed("x")),
    /// ]; // "x" doesn't exist under "a"
    /// let deleted = raw_jsonb.delete_by_keypath(keypath.iter()).unwrap();
    /// assert_eq!(deleted.to_string(), r#"{"a":{"b":[1,2,3]},"c":4}"#); // Original value returned
    ///
    /// // Attempting to delete from a scalar
    /// let scalar_jsonb = "1".parse::<OwnedJsonb>().unwrap();
    /// let raw_jsonb = scalar_jsonb.as_raw();
    /// let result = raw_jsonb.delete_by_keypath([].iter());
    /// assert!(result.is_err()); // Returns an error
    /// ```
    pub fn delete_by_keypath<'a, I: Iterator<Item = &'a KeyPath<'a>>>(
        &self,
        keypaths: I,
    ) -> Result<OwnedJsonb> {
        let jsonb_item_type = self.jsonb_item_type()?;
        if matches!(
            jsonb_item_type,
            JsonbItemType::Null
                | JsonbItemType::Boolean
                | JsonbItemType::Number
                | JsonbItemType::String
        ) {
            return Err(Error::InvalidJsonType);
        }
        // collect item indics need to delete in each object or array.
        let root_item = JsonbItem::Raw(*self);
        let mut items = VecDeque::new();
        items.push_back((root_item, 0));
        for path in keypaths {
            let Some((current_item, _)) = items.back() else {
                items.clear();
                break;
            };
            let Some(current) = current_item.as_raw_jsonb() else {
                items.clear();
                break;
            };

            let jsonb_item_type = current.jsonb_item_type()?;
            match jsonb_item_type {
                JsonbItemType::Array(_) => {
                    if let KeyPath::Index(index) = path {
                        let array_iter_opt = ArrayIterator::new(current)?;
                        if let Some(mut array_iter) = array_iter_opt {
                            let length = array_iter.len() as i32;
                            if *index > length || length + *index < 0 {
                                items.clear();
                                break;
                            }
                            let index = if *index >= 0 {
                                *index as usize
                            } else {
                                (length + *index) as usize
                            };
                            if let Some(item_result) = array_iter.nth(index) {
                                let item = item_result?;
                                items.push_back((item, index));
                                continue;
                            }
                        }
                    }
                    items.clear();
                    break;
                }
                JsonbItemType::Object(_) => {
                    let name = match path {
                        KeyPath::Index(index) => format!("{index}"),
                        KeyPath::Name(name) | KeyPath::QuotedName(name) => format!("{name}"),
                    };
                    let object_iter_opt = ObjectIterator::new(current)?;
                    if let Some(object_iter) = object_iter_opt {
                        let mut matched = false;
                        for (index, result) in &mut object_iter.enumerate() {
                            let (key, val_item) = result?;
                            if key.eq(&name) {
                                matched = true;
                                items.push_back((val_item, index));
                                break;
                            }
                        }
                        if matched {
                            continue;
                        }
                    }
                    items.clear();
                    break;
                }
                _ => {
                    items.clear();
                    break;
                }
            }
        }
        if items.len() <= 1 {
            return Ok(self.to_owned());
        }

        let mut child_jsonb: Option<OwnedJsonb> = None;
        let (_, mut del_index) = items.pop_back().unwrap();

        // Recursively builds an array or object for paths, and remove elements that need to be deleted.
        while let Some((current_item, next_del_index)) = items.pop_back() {
            let current_raw_jsonb = current_item.as_raw_jsonb().unwrap();

            let jsonb_item_type = current_raw_jsonb.jsonb_item_type()?;
            let current_del_jsonb = match jsonb_item_type {
                JsonbItemType::Array(_) => {
                    let array_iter = ArrayIterator::new(current_raw_jsonb)?.unwrap();
                    let mut builder = ArrayBuilder::with_capacity(array_iter.len());
                    for (i, item_result) in &mut array_iter.enumerate() {
                        let item = item_result?;
                        if i != del_index {
                            builder.push_jsonb_item(item);
                        } else if let Some(ref child_jsonb) = child_jsonb {
                            builder.push_owned_jsonb(child_jsonb.clone());
                        }
                    }
                    builder.build()?
                }
                JsonbItemType::Object(_) => {
                    let object_iter = ObjectIterator::new(current_raw_jsonb)?.unwrap();
                    let mut builder = ObjectBuilder::new();
                    for (i, result) in &mut object_iter.enumerate() {
                        let (key, val_item) = result?;
                        if i != del_index {
                            let _ = builder.push_jsonb_item(key, val_item);
                        } else if let Some(ref child_jsonb) = child_jsonb {
                            let _ = builder.push_owned_jsonb(key, child_jsonb.clone());
                        }
                    }
                    builder.build()?
                }
                _ => unreachable!(),
            };
            child_jsonb = Some(current_del_jsonb);
            del_index = next_del_index;
        }
        Ok(child_jsonb.unwrap())
    }

    /// Checks if all specified keys exist in a JSONB.
    ///
    /// This function checks if a JSONB value contains *all* of the keys provided in the `keys` iterator.
    /// If JSONB is an object, check the keys of the object, if it is an array, check the value of type string in the array,
    /// and if it is a scalar, check the value of type string.
    ///
    /// # Arguments
    ///
    /// * `self` - The JSONB value.
    /// * `keys` - An iterator of keys to check for.
    ///
    /// # Returns
    ///
    /// * `Ok(true)` - If all keys exist in the JSONB.
    /// * `Ok(false)` - If any of the keys do not exist.
    /// * `Err(Error)` - If the input JSONB data is invalid.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use jsonb::OwnedJsonb;
    ///
    /// let obj_jsonb = r#"{"a": 1, "b": 2, "c": 3}"#.parse::<OwnedJsonb>().unwrap();
    /// let raw_jsonb = obj_jsonb.as_raw();
    ///
    /// let keys = ["a", "b", "c"];
    /// assert!(raw_jsonb.exists_all_keys(keys.into_iter()).unwrap());
    ///
    /// let keys = ["a", "b", "d"];
    /// assert!(!raw_jsonb.exists_all_keys(keys.into_iter()).unwrap()); // "d" does not exist
    ///
    /// let arr_jsonb = r#"["a","b","c"]"#.parse::<OwnedJsonb>().unwrap();
    /// let raw_jsonb = arr_jsonb.as_raw();
    /// let keys = ["a", "b"];
    /// assert!(raw_jsonb.exists_all_keys(keys.into_iter()).unwrap());
    ///
    /// let str_jsonb = r#""a""#.parse::<OwnedJsonb>().unwrap();
    /// let raw_jsonb = str_jsonb.as_raw();
    /// let keys = ["b"];
    /// assert!(!raw_jsonb.exists_all_keys(keys.into_iter()).unwrap());
    /// ```
    pub fn exists_all_keys<'a, I: Iterator<Item = &'a str>>(&self, keys: I) -> Result<bool> {
        let mut self_keys = BTreeSet::new();
        let jsonb_item_type = self.jsonb_item_type()?;
        match jsonb_item_type {
            JsonbItemType::Object(_) => {
                let mut object_key_iter = ObjectKeyIterator::new(*self)?.unwrap();
                for result in &mut object_key_iter {
                    let item = result?;
                    if let Some(obj_key) = item.as_str() {
                        self_keys.insert(obj_key);
                    }
                }
            }
            JsonbItemType::Array(_) => {
                let mut array_iter = ArrayIterator::new(*self)?.unwrap();
                for result in &mut array_iter {
                    let item = result?;
                    if let Some(arr_key) = item.as_str() {
                        self_keys.insert(arr_key);
                    }
                }
            }
            JsonbItemType::String => {
                if let Some(self_key) = self.as_str()? {
                    for key in keys {
                        if self_key != key {
                            return Ok(false);
                        }
                    }
                }
                return Ok(true);
            }
            _ => {}
        }
        for key in keys {
            if !self_keys.contains(key) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Checks if any of the specified keys exist in a JSONB.
    ///
    /// This function checks if a JSONB value contains *any* of the keys provided in the `keys` iterator.
    /// If JSONB is an object, check the keys of the object, if it is an array, check the value of type string in the array,
    /// and if it is a scalar, check the value of type string.
    ///
    /// # Arguments
    ///
    /// * `self` - The JSONB value.
    /// * `keys` - An iterator of keys to check for.
    ///
    /// # Returns
    ///
    /// * `Ok(true)` - If any of the keys exist in the JSONB.
    /// * `Ok(false)` - If none of the keys exist.
    /// * `Err(Error)` - If the input JSONB data is invalid.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use jsonb::OwnedJsonb;
    ///
    /// let obj_jsonb = r#"{"a": 1, "b": 2, "c": 3}"#.parse::<OwnedJsonb>().unwrap();
    /// let raw_jsonb = obj_jsonb.as_raw();
    ///
    /// let keys = ["a", "d", "e"];
    /// assert!(raw_jsonb.exists_any_keys(keys.into_iter()).unwrap()); // "a" exists
    ///
    /// let keys = ["d", "e", "f"];
    /// assert!(!raw_jsonb.exists_any_keys(keys.into_iter()).unwrap()); // None of the keys exist
    ///
    /// let arr_jsonb = r#"["a","b","c"]"#.parse::<OwnedJsonb>().unwrap();
    /// let raw_jsonb = arr_jsonb.as_raw();
    /// let keys = ["a", "b"];
    /// assert!(raw_jsonb.exists_any_keys(keys.into_iter()).unwrap());
    ///
    /// let str_jsonb = r#""a""#.parse::<OwnedJsonb>().unwrap();
    /// let raw_jsonb = str_jsonb.as_raw();
    /// let keys = ["b"];
    /// assert!(!raw_jsonb.exists_any_keys(keys.into_iter()).unwrap());
    /// ```
    pub fn exists_any_keys<'a, I: Iterator<Item = &'a str>>(&self, keys: I) -> Result<bool> {
        let mut self_keys = BTreeSet::new();
        let jsonb_item_type = self.jsonb_item_type()?;
        match jsonb_item_type {
            JsonbItemType::Object(_) => {
                let mut object_key_iter = ObjectKeyIterator::new(*self)?.unwrap();
                for result in &mut object_key_iter {
                    let item = result?;
                    if let Some(obj_key) = item.as_str() {
                        self_keys.insert(obj_key);
                    }
                }
            }
            JsonbItemType::Array(_) => {
                let mut array_iter = ArrayIterator::new(*self)?.unwrap();
                for result in &mut array_iter {
                    let item = result?;
                    if let Some(arr_key) = item.as_str() {
                        self_keys.insert(arr_key);
                    }
                }
            }
            JsonbItemType::String => {
                if let Some(self_key) = self.as_str()? {
                    for key in keys {
                        if self_key == key {
                            return Ok(true);
                        }
                    }
                }
                return Ok(false);
            }
            _ => {}
        }
        for key in keys {
            if self_keys.contains(key) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Visits every scalar leaf key path in a JSONB value.
    ///
    /// This function recursively traverses objects and, unless `ignore_array` is true,
    /// arrays. It invokes `visitor` once for each leaf instead of allocating and returning
    /// a collection. The path slice is borrowed from traversal state and is only valid for
    /// the duration of the callback.
    ///
    /// # Arguments
    ///
    /// * `ignore_array` - When true, arrays are treated as leaves and their paths are
    ///   visited without descending into their elements.
    /// * `visitor` - Called with the path from the root to each leaf. Returning an error
    ///   stops traversal and propagates that error to the caller.
    ///
    /// Empty objects and arrays are treated as leaves. A scalar at the root has an empty
    /// key path and is skipped.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use jsonb::OwnedJsonb;
    /// use jsonb::keypath::KeyPaths;
    ///
    /// let jsonb = r#"{"user":{"name":"Alice","scores":[85,92]}}"#.parse::<OwnedJsonb>().unwrap();
    /// let mut paths = Vec::new();
    ///
    /// let raw_jsonb = jsonb.as_raw();
    /// raw_jsonb.visit_scalar_key_paths(false, |path| {
    ///         paths.push(KeyPaths {
    ///             paths: path.to_vec(),
    ///         });
    ///         Ok(())
    ///     })
    ///     .unwrap();
    /// assert_eq!(paths.len(), 3);
    /// ```
    pub fn visit_scalar_key_paths<'a, F>(&'a self, ignore_array: bool, mut visitor: F) -> Result<()>
    where
        F: FnMut(&[KeyPath<'a>]) -> Result<()>,
    {
        let item = JsonbItem::from_raw_jsonb(*self)?;
        let mut current_paths = Vec::with_capacity(3);
        Self::visit_scalar_key_paths_recursive(item, ignore_array, &mut current_paths, &mut visitor)
    }

    fn visit_scalar_key_paths_recursive<'a, F>(
        current_item: JsonbItem<'a>,
        ignore_array: bool,
        current_paths: &mut Vec<KeyPath<'a>>,
        visitor: &mut F,
    ) -> Result<()>
    where
        F: FnMut(&[KeyPath<'a>]) -> Result<()>,
    {
        match current_item {
            JsonbItem::Raw(raw) => {
                if let Some(mut object_iter) = ObjectIterator::new(raw)? {
                    if object_iter.len() > 0 {
                        for object_result in &mut object_iter {
                            let (key, value) = object_result?;
                            current_paths.push(KeyPath::Name(Cow::Borrowed(key)));
                            Self::visit_scalar_key_paths_recursive(
                                value,
                                ignore_array,
                                current_paths,
                                visitor,
                            )?;
                            current_paths.pop();
                        }
                        return Ok(());
                    }
                } else if !ignore_array {
                    if let Some(array_iter) = ArrayIterator::new(raw)? {
                        if array_iter.len() > 0 {
                            for (index, array_result) in array_iter.enumerate() {
                                let value = array_result?;
                                current_paths.push(KeyPath::Index(index as i32));
                                Self::visit_scalar_key_paths_recursive(
                                    value,
                                    ignore_array,
                                    current_paths,
                                    visitor,
                                )?;
                                current_paths.pop();
                            }
                            return Ok(());
                        }
                    }
                }
            }
            JsonbItem::Owned(_) => unreachable!(),
            _ => {}
        }

        if !current_paths.is_empty() {
            visitor(current_paths)?;
        }
        Ok(())
    }

    /// Visits every scalar leaf and its key path in a JSONB value.
    ///
    /// This is the callback-based counterpart of [`RawJsonb::extract_scalar_key_values`].
    /// It recursively traverses objects and, unless `ignore_array` is true, arrays, but
    /// avoids allocating a result vector. Both the path slice and value may borrow from the
    /// input JSONB and are only valid according to their callback lifetimes.
    ///
    /// # Arguments
    ///
    /// * `ignore_array` - When true, arrays are treated as leaf `Value::Array` values
    ///   without descending into their elements.
    /// * `visitor` - Called with the path and value of each leaf. Returning an error stops
    ///   traversal and propagates that error to the caller.
    ///
    /// Empty objects and arrays are treated as leaf values. JSON null is visited as
    /// `Value::Null`; it is not treated as a missing path. A scalar at the root has an empty
    /// key path and is skipped.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use jsonb::OwnedJsonb;
    /// use jsonb::Value;
    ///
    /// let jsonb = r#"{"user":{"name":"Alice","scores":[85,92]}}"#.parse::<OwnedJsonb>().unwrap();
    /// let mut values = Vec::new();
    ///
    /// let raw_jsonb = jsonb.as_raw();
    /// raw_jsonb.visit_scalar_key_values(true, |paths, value| {
    ///         values.push((paths.len(), value));
    ///         Ok(())
    ///     })
    ///     .unwrap();
    /// assert_eq!(values.len(), 2);
    /// assert!(
    ///     values
    ///         .iter()
    ///         .any(|(_, value)| matches!(value, Value::Array(_)))
    /// );
    /// ```
    pub fn visit_scalar_key_values<'a, F>(
        &'a self,
        ignore_array: bool,
        mut visitor: F,
    ) -> Result<()>
    where
        F: FnMut(&[KeyPath<'a>], Value<'a>) -> Result<()>,
    {
        let item = JsonbItem::from_raw_jsonb(*self)?;
        let mut current_paths = Vec::with_capacity(3);
        Self::visit_scalar_key_values_recursive(
            item,
            ignore_array,
            &mut current_paths,
            &mut visitor,
        )
    }

    fn visit_scalar_key_values_recursive<'a, F>(
        current_item: JsonbItem<'a>,
        ignore_array: bool,
        current_paths: &mut Vec<KeyPath<'a>>,
        visitor: &mut F,
    ) -> Result<()>
    where
        F: FnMut(&[KeyPath<'a>], Value<'a>) -> Result<()>,
    {
        match current_item {
            JsonbItem::Raw(raw) => {
                if let Some(mut object_iter) = ObjectIterator::new(raw)? {
                    if object_iter.len() > 0 {
                        for object_result in &mut object_iter {
                            let (key, value) = object_result?;
                            current_paths.push(KeyPath::Name(Cow::Borrowed(key)));
                            Self::visit_scalar_key_values_recursive(
                                value,
                                ignore_array,
                                current_paths,
                                visitor,
                            )?;
                            current_paths.pop();
                        }
                        return Ok(());
                    }
                } else if !ignore_array {
                    if let Some(array_iter) = ArrayIterator::new(raw)? {
                        if array_iter.len() > 0 {
                            for (index, array_result) in array_iter.enumerate() {
                                let value = array_result?;
                                current_paths.push(KeyPath::Index(index as i32));
                                Self::visit_scalar_key_values_recursive(
                                    value,
                                    ignore_array,
                                    current_paths,
                                    visitor,
                                )?;
                                current_paths.pop();
                            }
                            return Ok(());
                        }
                    }
                }
                if !current_paths.is_empty() {
                    visitor(current_paths, raw.to_value()?)?;
                }
            }
            JsonbItem::Owned(_) => unreachable!(),
            JsonbItem::Null => {
                if !current_paths.is_empty() {
                    visitor(current_paths, Value::Null)?;
                }
            }
            JsonbItem::Boolean(value) => {
                if !current_paths.is_empty() {
                    visitor(current_paths, Value::Bool(value))?;
                }
            }
            JsonbItem::String(value) => {
                if !current_paths.is_empty() {
                    visitor(current_paths, Value::String(value))?;
                }
            }
            JsonbItem::Number(value) => {
                if !current_paths.is_empty() {
                    visitor(current_paths, Value::Number(value.as_number()?))?;
                }
            }
            JsonbItem::Extension(value) => {
                if !current_paths.is_empty() {
                    let value = match value.as_extension_value()? {
                        ExtensionValue::Binary(value) => Value::Binary(value),
                        ExtensionValue::Date(value) => Value::Date(value),
                        ExtensionValue::Timestamp(value) => Value::Timestamp(value),
                        ExtensionValue::TimestampTz(value) => Value::TimestampTz(value),
                        ExtensionValue::Interval(value) => Value::Interval(value),
                    };
                    visitor(current_paths, value)?;
                }
            }
        }
        Ok(())
    }

    /// Extracts all scalar values from a JSONB value along with their key paths.
    ///
    /// This function recursively traverses the JSONB structure (both objects and arrays)
    /// and collects all leaf node scalar values (null, boolean, number, string, etc.)
    /// along with their corresponding key paths. The key path represents the navigation
    /// path from the root to reach each scalar value.
    ///
    /// # Arguments
    ///
    /// * `ignore_array` - When true, arrays are treated as leaf values and returned as
    ///   `Value::Array` without descending into their elements.
    ///
    /// # Returns
    ///
    /// * `Result<Vec<(KeyPaths<'_>, Value<'_>)>>` - A vector of tuples, each containing:
    ///   - `KeyPaths`: The path to reach the scalar value
    ///   - `Value`: The scalar value itself
    ///
    /// Empty objects or arrays are treated as leaf values and returned as `Value::Object` or
    /// `Value::Array`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use jsonb::OwnedJsonb;
    ///
    /// let json = r#"{"user": {"name": "Alice", "scores": [85, 92, 78]}}"#;
    /// let jsonb = json.parse::<OwnedJsonb>().unwrap();
    /// let raw_jsonb = jsonb.as_raw();
    /// let result = raw_jsonb.extract_scalar_key_values(false);
    /// assert!(result.is_ok());
    /// let result = result.unwrap();
    /// assert_eq!(result.len(), 4);
    /// // Result contains:
    /// // - path: "user", "name" -> value: "Alice"
    /// // - path: "user", "scores", 0 -> value: 85
    /// // - path: "user", "scores", 1 -> value: 92
    /// // - path: "user", "scores", 2 -> value: 78
    /// ```
    ///
    /// ```rust
    /// use jsonb::OwnedJsonb;
    /// use jsonb::Value;
    ///
    /// let json = r#"{"user": {"name": "Alice", "scores": [85, 92, 78]}}"#;
    /// let jsonb = json.parse::<OwnedJsonb>().unwrap();
    /// let raw_jsonb = jsonb.as_raw();
    /// let result = raw_jsonb.extract_scalar_key_values(true).unwrap();
    ///
    /// assert_eq!(result.len(), 2);
    /// assert!(
    ///     result
    ///         .iter()
    ///         .any(|(_, value)| matches!(value, Value::Array(_)))
    /// );
    /// // Result contains:
    /// // - path: "user", "name" -> value: "Alice"
    /// // - path: "user", "scores" -> value: [85, 92, 78]
    /// ```
    pub fn extract_scalar_key_values(
        &self,
        ignore_array: bool,
    ) -> Result<Vec<(KeyPaths<'_>, Value<'_>)>> {
        let mut result = Vec::with_capacity(16);
        self.visit_scalar_key_values(ignore_array, |paths, value| {
            result.push((
                KeyPaths {
                    paths: paths.to_vec(),
                },
                value,
            ));
            Ok(())
        })?;
        Ok(result)
    }
}
