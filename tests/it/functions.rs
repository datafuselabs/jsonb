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
use std::collections::BTreeMap;
use std::collections::BTreeSet;

use ethnum::I256;
use jsonb::core::JsonbItemType;
use jsonb::from_raw_jsonb;
use jsonb::from_slice;
use jsonb::jsonpath::parse_json_path;
use jsonb::keypath::parse_key_paths;
use jsonb::keypath::KeyPath;
use jsonb::keypath::KeyPaths;
use jsonb::parse_value;
use jsonb::Date;
use jsonb::Decimal128;
use jsonb::Decimal256;
use jsonb::Decimal64;
use jsonb::Error;
use jsonb::Interval;
use jsonb::Number;
use jsonb::Object;
use jsonb::OwnedJsonb;
use jsonb::RawJsonb;
use jsonb::Timestamp;
use jsonb::TimestampTz;
use jsonb::Value;
use nom::AsBytes;

#[test]
fn test_build_array() {
    let sources = vec![
        r#"true"#,
        r#"123.45"#,
        r#""abc""#,
        r#"[1,2,3]"#,
        r#"{"k":"v"}"#,
    ];
    let mut expect_array = Vec::with_capacity(sources.len());
    let mut owned_jsonbs = Vec::with_capacity(sources.len());
    for s in sources {
        let value = parse_value(s.as_bytes()).unwrap();
        expect_array.push(value.clone());
        let mut buf: Vec<u8> = Vec::new();
        value.write_to_vec(&mut buf);
        let owned_jsonb = OwnedJsonb::new(buf);
        owned_jsonbs.push(owned_jsonb);
    }

    let expect_value = Value::Array(expect_array);
    let mut expect_buf: Vec<u8> = Vec::new();
    expect_value.write_to_vec(&mut expect_buf);

    let owned_arr = OwnedJsonb::build_array(owned_jsonbs.iter().map(|v| v.as_raw())).unwrap();
    assert_eq!(owned_arr.as_ref(), &expect_buf);

    let value = from_slice(owned_arr.as_ref()).unwrap();
    assert!(value.is_array());
    let array = value.as_array().unwrap();
    assert_eq!(array.len(), 5);
}

#[test]
fn test_build_object() {
    let sources = [
        r#"true"#,
        r#"123.45"#,
        r#""abc""#,
        r#"[1,2,3]"#,
        r#"{"k":"v"}"#,
    ];
    let keys = [
        "k1".to_string(),
        "k2".to_string(),
        "k3".to_string(),
        "k4".to_string(),
        "k5".to_string(),
    ];

    let mut expect_object = Object::new();
    let mut owned_jsonbs = Vec::with_capacity(sources.len());
    for (key, s) in keys.iter().zip(sources.iter()) {
        let value = parse_value(s.as_bytes()).unwrap();
        expect_object.insert(key.clone(), value.clone());

        let mut buf: Vec<u8> = Vec::new();
        value.write_to_vec(&mut buf);
        let owned_jsonb = OwnedJsonb::new(buf);
        owned_jsonbs.push((key.clone(), owned_jsonb));
    }

    let expect_value = Value::Object(expect_object);
    let mut expect_buf: Vec<u8> = Vec::new();
    expect_value.write_to_vec(&mut expect_buf);

    let owned_obj =
        OwnedJsonb::build_object(owned_jsonbs.iter().map(|(k, v)| (k, v.as_raw()))).unwrap();
    assert_eq!(owned_obj.as_ref(), &expect_buf);

    let value = from_slice(owned_obj.as_ref()).unwrap();
    assert!(value.is_object());
    let array = value.as_object().unwrap();
    assert_eq!(array.len(), 5);
}

#[test]
fn test_array_length() {
    let sources = vec![
        (r#"true"#, None),
        (r#"1234"#, None),
        (r#"[]"#, Some(0)),
        (r#"[1,2,3]"#, Some(3)),
        (r#"["a","b","c","d","e","f"]"#, Some(6)),
        (r#"{"k":"v"}"#, None),
    ];

    for (s, expect) in sources {
        let owned_jsonb = s.parse::<OwnedJsonb>().unwrap();
        let raw_jsonb = owned_jsonb.as_raw();
        let res = raw_jsonb.array_length();
        assert_eq!(res, Ok(expect));
    }
}

#[test]
fn test_path_exists() {
    let sources = vec![
        (r#"{"a":1,"b":2}"#, r#"$.a"#, true),
        (r#"{"a":1,"b":2}"#, r#"$.c"#, false),
        (r#"{"a":1,"b":2}"#, r#"$.a ? (@ == 1)"#, true),
        (r#"{"a":1,"b":2}"#, r#"$.a ? (@ > 1)"#, false),
        (r#"{"a":1,"b":[1,2,3]}"#, r#"$.b[0]"#, true),
        (r#"{"a":1,"b":[1,2,3]}"#, r#"$.b[3]"#, false),
        (
            r#"{"a":1,"b":[1,2,3]}"#,
            r#"$.b[1 to last] ? (@ >=2 && @ <=3)"#,
            true,
        ),
        // predicates always return true in path_exists.
        (r#"{"a":1,"b":[1,2,3]}"#, r#"$.b[1 to last] > 10"#, true),
        (r#"{"a":1,"b":[1,2,3]}"#, r#"$.b[1 to last] > 1"#, true),
    ];
    for (json, path, expect) in sources {
        let owned_jsonb = json.parse::<OwnedJsonb>().unwrap();
        let raw_jsonb = owned_jsonb.as_raw();
        let json_path = parse_json_path(path.as_bytes()).unwrap();
        let res = raw_jsonb.path_exists(&json_path);
        assert_eq!(res, Ok(expect));
    }
}

#[test]
fn test_path_exists_expr() {
    let source = r#"{"items": [
        {"id": 0, "name": "Andrew", "car": "Volvo"},
        {"id": 1, "name": "Fred", "car": "BMW"},
        {"id": 2, "name": "James"},
        {"id": 3, "name": "Ken"}
    ]}"#;
    let paths = vec![
        (
            "$.items[*]?(exists($.items))",
            r#"[
                {"id": 0, "name": "Andrew", "car": "Volvo"},
                {"id": 1, "name": "Fred", "car": "BMW"},
                {"id": 2, "name": "James"},
                {"id": 3, "name": "Ken"}
            ]"#,
        ),
        (
            "$.items[*]?(exists(@.car))",
            r#"[
                {"id": 0, "name": "Andrew", "car": "Volvo"},
                {"id": 1, "name": "Fred", "car": "BMW"}
            ]"#,
        ),
        (
            r#"$.items[*]?(exists(@.car?(@ == "Volvo")))"#,
            r#"[
                {"id": 0, "name": "Andrew", "car": "Volvo"}
            ]"#,
        ),
        (
            r#"$.items[*]?(exists(@.car) && @.id >= 1)"#,
            r#"[
                {"id": 1, "name": "Fred", "car": "BMW"}
            ]"#,
        ),
        (
            r#"$ ? (exists(@.items[*]?(exists(@.car))))"#,
            r#"[{"items": [
                {"id": 0, "name": "Andrew", "car": "Volvo"},
                {"id": 1, "name": "Fred", "car": "BMW"},
                {"id": 2, "name": "James"},
                {"id": 3, "name": "Ken"}
            ]}]"#,
        ),
        (
            r#"$ ? (exists(@.items[*]?(exists(@.car) && @.id == 5)))"#,
            r#"[]"#,
        ),
    ];

    let owned_jsonb = source.parse::<OwnedJsonb>().unwrap();
    let raw_jsonb = owned_jsonb.as_raw();
    for (path, expected) in paths {
        let json_path = parse_json_path(path.as_bytes()).unwrap();
        let res = raw_jsonb.select_array_by_path(&json_path);
        assert!(res.is_ok());
        let owned_jsonb = res.unwrap();
        let expected_buf = parse_value(expected.as_bytes()).unwrap().to_vec();

        assert_eq!(owned_jsonb.to_vec(), expected_buf);
    }
}

#[test]
fn test_select_by_path() {
    let source = r#"{"name":"Fred","phones":[{"type":"home","number":3720453},{"type":"work","number":5062051}],"car_no":123,"测试\"\uD83D\uDC8E":"ab","numbers":[2,3,4],"key":null}"#;

    let paths = vec![
        (r#"$.name"#, vec![r#""Fred""#]),
        (
            r#"$.phones"#,
            vec![r#"[{"type":"home","number":3720453},{"type":"work","number":5062051}]"#],
        ),
        (r#"$.phones.*"#, vec![]),
        (
            r#"$.phones[*]"#,
            vec![
                r#"{"type":"home","number":3720453}"#,
                r#"{"type":"work","number":5062051}"#,
            ],
        ),
        (
            r#"$.phones.**"#,
            vec![
                r#"[{"type":"home","number":3720453},{"type":"work","number":5062051}]"#,
                r#"{"type":"home","number":3720453}"#,
                r#"3720453"#,
                r#""home""#,
                r#"{"type":"work","number":5062051}"#,
                r#"5062051"#,
                r#""work""#,
            ],
        ),
        (
            r#"$.phones.**{1 to last}"#,
            vec![
                r#"{"type":"home","number":3720453}"#,
                r#"3720453"#,
                r#""home""#,
                r#"{"type":"work","number":5062051}"#,
                r#"5062051"#,
                r#""work""#,
            ],
        ),
        (r#"$.phones[0].*"#, vec![r#"3720453"#, r#""home""#]),
        (r#"$.phones[0].type"#, vec![r#""home""#]),
        (r#"$.phones[*].type[*]"#, vec![r#""home""#, r#""work""#]),
        (
            r#"$.phones[0 to last].number"#,
            vec![r#"3720453"#, r#"5062051"#],
        ),
        (
            r#"$.phones[0 to last]?(4 == 4)"#,
            vec![
                r#"{"type":"home","number":3720453}"#,
                r#"{"type":"work","number":5062051}"#,
            ],
        ),
        (
            r#"$.phones[0 to last]?(@.type == "home")"#,
            vec![r#"{"type":"home","number":3720453}"#],
        ),
        (
            r#"$.phones[0 to last]?(@.number == 3720453)"#,
            vec![r#"{"type":"home","number":3720453}"#],
        ),
        (
            r#"$.phones[0 to last]?(@.number == 3720453 || @.type == "work")"#,
            vec![
                r#"{"type":"home","number":3720453}"#,
                r#"{"type":"work","number":5062051}"#,
            ],
        ),
        (
            r#"$.phones[0 to last]?(@.number == 3720453 && @.type == "work")"#,
            vec![],
        ),
        (
            r#"$.car_no?($.name == "Fred" && $.car_no != null)"#,
            vec![r#"123"#],
        ),
        (r#"$.car_no"#, vec![r#"123"#]),
        (r#"$.测试\"\uD83D\uDC8E"#, vec![r#""ab""#]),
        // predicates return the result of the filter expression.
        (r#"$.phones[0 to last].number == 3720453"#, vec!["true"]),
        (r#"$.phones[0 to last].type == "workk""#, vec!["false"]),
        (r#"$.name == "Fred" && $.car_no == 123"#, vec!["true"]),
        (
            r#"$.phones[*] ? (@.type starts with "ho")"#,
            vec![r#"{"type":"home","number":3720453}"#],
        ),
        // arithmetic functions
        (r#"$.phones[0].number + 3"#, vec![r#"3720456"#]),
        (r#"$.phones[0].number % 10"#, vec![r#"3"#]),
        (r#"7 - $.phones[1].number"#, vec![r#"-5062044"#]),
        (r#"+$.numbers"#, vec![r#"2"#, r#"3"#, r#"4"#]),
        (r#"-$.numbers"#, vec![r#"-2"#, r#"-3"#, r#"-4"#]),
        (r#"$.numbers[1] / 2"#, vec![r#"1.5"#]),
    ];

    let owned_jsonb = source.parse::<OwnedJsonb>().unwrap();
    let raw_jsonb = owned_jsonb.as_raw();
    for (path, expects) in paths {
        let json_path = parse_json_path(path.as_bytes()).unwrap();
        let res = raw_jsonb.select_by_path(&json_path);
        assert!(res.is_ok());
        let owned_jsonbs = res.unwrap();
        assert_eq!(owned_jsonbs.len(), expects.len());
        for (owned_jsonb, expect) in owned_jsonbs.into_iter().zip(expects.iter()) {
            let expected_buf = parse_value(expect.as_bytes()).unwrap().to_vec();
            assert_eq!(owned_jsonb.as_raw(), RawJsonb::new(&expected_buf));
        }
    }
}

#[test]
fn test_get_by_index() {
    let sources = vec![
        (r#"1234"#, 0, None),
        (r#"[]"#, 0, None),
        (r#"[1,2,3]"#, 1, Some(Value::Number(Number::UInt64(2)))),
        (r#"["a","b","c"]"#, 0, Some(Value::String(Cow::from("a")))),
    ];

    for (s, idx, expect) in sources {
        let owned_jsonb = s.parse::<OwnedJsonb>().unwrap();
        let raw_jsonb = owned_jsonb.as_raw();
        let res = raw_jsonb.get_by_index(idx);
        assert!(res.is_ok());
        let owned_jsonb_opt = res.unwrap();
        match expect {
            Some(expect) => {
                assert!(owned_jsonb_opt.is_some());
                let owned_jsonb = owned_jsonb_opt.unwrap();
                let expected_buf = expect.to_vec();
                assert_eq!(owned_jsonb.to_vec(), expected_buf);
            }
            None => assert_eq!(owned_jsonb_opt, None),
        }
    }
}

#[test]
fn test_get_by_name() {
    let sources = vec![
        (r#"true"#, "a".to_string(), None),
        (r#"[1,2,3]"#, "a".to_string(), None),
        (r#"{"a":"v1","b":[1,2,3]}"#, "k".to_string(), None),
        (
            r#"{"Aa":"v1", "aA":"v2", "aa":"v3"}"#,
            "aa".to_string(),
            Some(Value::String(Cow::from("v3"))),
        ),
        (
            r#"{"Aa":"v1", "aA":"v2", "aa":"v3"}"#,
            "AA".to_string(),
            None,
        ),
    ];

    for (s, name, expect) in sources {
        let owned_jsonb = s.parse::<OwnedJsonb>().unwrap();
        let raw_jsonb = owned_jsonb.as_raw();
        let res = raw_jsonb.get_by_name(&name, false);
        assert!(res.is_ok());
        let owned_jsonb_opt = res.unwrap();
        match expect {
            Some(expect) => {
                assert!(owned_jsonb_opt.is_some());
                let owned_jsonb = owned_jsonb_opt.unwrap();
                let expected_buf = expect.to_vec();
                assert_eq!(owned_jsonb.to_vec(), expected_buf);
            }
            None => assert_eq!(owned_jsonb_opt, None),
        }
    }
}

#[test]
fn test_get_by_name_ignore_case() {
    let sources = vec![
        (r#"true"#, "a".to_string(), None),
        (r#"[1,2,3]"#, "a".to_string(), None),
        (r#"{"a":"v1","b":[1,2,3]}"#, "k".to_string(), None),
        (
            r#"{"Aa":"v1", "aA":"v2", "aa":"v3"}"#,
            "aa".to_string(),
            Some(Value::String(Cow::from("v3"))),
        ),
        (
            r#"{"Aa":"v1", "aA":"v2", "aa":"v3"}"#,
            "AA".to_string(),
            Some(Value::String(Cow::from("v1"))),
        ),
    ];

    for (s, name, expect) in sources {
        let owned_jsonb = s.parse::<OwnedJsonb>().unwrap();
        let raw_jsonb = owned_jsonb.as_raw();
        let res = raw_jsonb.get_by_name(&name, true);
        assert!(res.is_ok());
        let owned_jsonb_opt = res.unwrap();
        match expect {
            Some(expect) => {
                assert!(owned_jsonb_opt.is_some());
                let owned_jsonb = owned_jsonb_opt.unwrap();
                let expected_buf = expect.to_vec();
                assert_eq!(owned_jsonb.to_vec(), expected_buf);
            }
            None => assert_eq!(owned_jsonb_opt, None),
        }
    }
}

#[test]
fn test_object_keys() {
    let sources = vec![
        (r#"[1,2,3]"#, None),
        (
            r#"{"a":"v1","b":[1,2,3]}"#,
            Some(Value::Array(vec![
                Value::String(Cow::from("a")),
                Value::String(Cow::from("b")),
            ])),
        ),
        (
            r#"{"k1":"v1","k2":[1,2,3]}"#,
            Some(Value::Array(vec![
                Value::String(Cow::from("k1")),
                Value::String(Cow::from("k2")),
            ])),
        ),
    ];

    for (s, expect) in sources {
        let owned_jsonb = s.parse::<OwnedJsonb>().unwrap();
        let raw_jsonb = owned_jsonb.as_raw();
        let res = raw_jsonb.object_keys();
        assert!(res.is_ok());
        let owned_jsonb_opt = res.unwrap();
        match expect {
            Some(expect) => {
                assert!(owned_jsonb_opt.is_some());
                let owned_jsonb = owned_jsonb_opt.unwrap();
                let expected_buf = expect.to_vec();
                assert_eq!(owned_jsonb.to_vec(), expected_buf);
            }
            None => assert_eq!(owned_jsonb_opt, None),
        }
    }
}

#[test]
fn test_array_values() {
    let sources = vec![
        (r#"{"a":"v1","b":"v2"}"#, None),
        (r#"[]"#, Some(vec![])),
        (
            r#"[1,"a",[1,2]]"#,
            Some(vec![
                Value::Number(Number::UInt64(1)),
                Value::String(Cow::from("a")),
                Value::Array(vec![
                    Value::Number(Number::UInt64(1)),
                    Value::Number(Number::UInt64(2)),
                ]),
            ]),
        ),
    ];

    for (s, expect) in sources {
        let owned_jsonb = s.parse::<OwnedJsonb>().unwrap();
        let raw_jsonb = owned_jsonb.as_raw();
        let res = raw_jsonb.array_values();
        assert!(res.is_ok());
        let owned_jsonb_opt = res.unwrap();
        match expect {
            Some(expects) => {
                assert!(owned_jsonb_opt.is_some());
                let owned_jsonbs = owned_jsonb_opt.unwrap();
                assert_eq!(owned_jsonbs.len(), expects.len());
                for (owned_jsonb, expect) in owned_jsonbs.into_iter().zip(expects.iter()) {
                    let expected_buf = expect.to_vec();
                    assert_eq!(owned_jsonb.to_vec(), expected_buf);
                }
            }
            None => assert_eq!(owned_jsonb_opt, None),
        }
    }
}

#[test]
fn test_compare() {
    let sources = vec![
        (r#"null"#, r#"null"#, Ordering::Equal),
        (r#"null"#, r#"[1,2,3]"#, Ordering::Greater),
        (r#"null"#, r#"{"k":"v"}"#, Ordering::Greater),
        (r#"null"#, r#"123.45"#, Ordering::Greater),
        (r#"null"#, r#""abcd""#, Ordering::Greater),
        (r#"null"#, r#"true"#, Ordering::Greater),
        (r#"null"#, r#"false"#, Ordering::Greater),
        (r#""abcd""#, r#"null"#, Ordering::Less),
        (r#""abcd""#, r#""def""#, Ordering::Less),
        (r#""abcd""#, r#"123.45"#, Ordering::Greater),
        (r#""abcd""#, r#"true"#, Ordering::Greater),
        (r#""abcd""#, r#"false"#, Ordering::Greater),
        (r#"123"#, r#"12.3"#, Ordering::Greater),
        (r#"123"#, r#"123"#, Ordering::Equal),
        (r#"123"#, r#"456.7"#, Ordering::Less),
        (r#"12.3"#, r#"12"#, Ordering::Greater),
        (r#"-12.3"#, r#"12"#, Ordering::Less),
        (r#"123"#, r#"true"#, Ordering::Greater),
        (r#"123"#, r#"false"#, Ordering::Greater),
        (r#"true"#, r#"true"#, Ordering::Equal),
        (r#"true"#, r#"false"#, Ordering::Greater),
        (r#"false"#, r#"true"#, Ordering::Less),
        (r#"false"#, r#"false"#, Ordering::Equal),
        (r#"[1,2,3]"#, r#"null"#, Ordering::Less),
        (r#"[1,2,3]"#, r#"[1,2,3]"#, Ordering::Equal),
        (r#"[1,2,3]"#, r#"[1,2]"#, Ordering::Greater),
        (r#"[1,2,3]"#, r#"[]"#, Ordering::Greater),
        (r#"[1,2,3]"#, r#"[3]"#, Ordering::Less),
        (r#"[1,2,3]"#, r#"["a"]"#, Ordering::Less),
        (r#"[1,2,3]"#, r#"[true]"#, Ordering::Greater),
        (r#"[1,2,3]"#, r#"[1,2,3,4]"#, Ordering::Less),
        (r#"[1,2,3]"#, r#"{"k":"v"}"#, Ordering::Greater),
        (r#"[1,2,3]"#, r#""abcd""#, Ordering::Greater),
        (r#"[1,2,3]"#, r#"1234"#, Ordering::Greater),
        (r#"[1,2,3]"#, r#"12.34"#, Ordering::Greater),
        (r#"[1,2,3]"#, r#"true"#, Ordering::Greater),
        (r#"[1,2,3]"#, r#"false"#, Ordering::Greater),
        (r#"{"k1":"v1","k2":"v2"}"#, r#"null"#, Ordering::Less),
        (r#"{"k1":"v1","k2":"v2"}"#, r#"[1,2,3]"#, Ordering::Less),
        (
            r#"{"k1":"v1","k2":"v2"}"#,
            r#"{"k1":"v1","k2":"v2"}"#,
            Ordering::Equal,
        ),
        (
            r#"{"k1":"v1","k2":"v2"}"#,
            r#"{"k":"v1","k2":"v2"}"#,
            Ordering::Greater,
        ),
        (
            r#"{"k1":"v1","k2":"v2"}"#,
            r#"{"k1":"a1","k2":"v2"}"#,
            Ordering::Greater,
        ),
        (r#"{"k1":123}"#, r#"{"k1":123,"k2":111}"#, Ordering::Less),
        (r#"{"k1":123,"k2":111}"#, r#"{"k1":123}"#, Ordering::Greater),
        (r#"{"k1":"v1","k2":"v2"}"#, r#"{"a":1}"#, Ordering::Greater),
        (r#"{"k1":"v1","k2":"v2"}"#, r#"{}"#, Ordering::Greater),
        (r#"{"k1":"v1","k2":"v2"}"#, r#""ab""#, Ordering::Greater),
        (r#"{"k1":"v1","k2":"v2"}"#, r#"123"#, Ordering::Greater),
        (r#"{"k1":"v1","k2":"v2"}"#, r#"12.34"#, Ordering::Greater),
        (r#"{"k1":"v1","k2":"v2"}"#, r#"true"#, Ordering::Greater),
        (r#"{"k1":"v1","k2":"v2"}"#, r#"false"#, Ordering::Greater),
    ];

    for (left, right, expected) in sources {
        let left_owned_jsonb = left.parse::<OwnedJsonb>().unwrap();
        let left_raw_jsonb = left_owned_jsonb.as_raw();
        let right_owned_jsonb = right.parse::<OwnedJsonb>().unwrap();
        let right_raw_jsonb = right_owned_jsonb.as_raw();

        let res = left_raw_jsonb.cmp(&right_raw_jsonb);
        assert_eq!(res, expected);
    }
}

#[test]
fn test_as_type() {
    let sources = vec![
        (r#"null"#, Some(()), None, None, None, false, false),
        (r#"true"#, None, Some(true), None, None, false, false),
        (r#"false"#, None, Some(false), None, None, false, false),
        (
            r#"-1234"#,
            None,
            None,
            Some(Number::Int64(-1234)),
            None,
            false,
            false,
        ),
        (
            r#"12.34"#,
            None,
            None,
            Some(Number::Float64(12.34)),
            None,
            false,
            false,
        ),
        (
            r#""abcd""#,
            None,
            None,
            None,
            Some(Cow::from("abcd")),
            false,
            false,
        ),
        (r#"[1,2,3]"#, None, None, None, None, true, false),
        (r#"{"k":"v"}"#, None, None, None, None, false, true),
    ];

    for (s, expect_null, expect_bool, expect_number, expect_str, expect_array, expect_object) in
        sources
    {
        let owned_jsonb = s.parse::<OwnedJsonb>().unwrap();
        let raw_jsonb = owned_jsonb.as_raw();

        let res = raw_jsonb.as_null();
        assert!(res.is_ok());
        let res = res.unwrap();
        match expect_null {
            Some(_) => assert!(res.is_some()),
            None => assert_eq!(res, None),
        }
        let res = raw_jsonb.as_bool();
        assert!(res.is_ok());
        let res = res.unwrap();
        match expect_bool {
            Some(expect) => assert_eq!(res.unwrap(), expect),
            None => assert_eq!(res, None),
        }
        let res = raw_jsonb.as_number();
        assert!(res.is_ok());
        let res = res.unwrap();
        match expect_number.clone() {
            Some(expect) => assert_eq!(res.unwrap(), expect),
            None => assert_eq!(res, None),
        }
        let res = raw_jsonb.as_str();
        assert!(res.is_ok());
        let res = res.unwrap();
        match expect_str.clone() {
            Some(expect) => assert_eq!(res.unwrap(), expect),
            None => assert_eq!(res, None),
        }
        let res = raw_jsonb.is_array();
        assert!(res.is_ok());
        let res = res.unwrap();
        assert_eq!(res, expect_array);
        let res = raw_jsonb.is_object();
        assert!(res.is_ok());
        let res = res.unwrap();
        assert_eq!(res, expect_object);
    }
}

#[test]
fn test_as_integer_type() {
    let sources = vec![
        (r#"123"#, Some(123_i64), Some(123_u64)),
        (r#"123.0"#, Some(123_i64), Some(123_u64)),
        (r#"123.1"#, None, None),
        (r#"1.5e0"#, None, None),
        (r#"-1.0"#, Some(-1_i64), None),
        (r#"-1.1"#, None, None),
    ];

    for (s, expect_i64, expect_u64) in sources {
        let owned_jsonb = s.parse::<OwnedJsonb>().unwrap();
        let raw_jsonb = owned_jsonb.as_raw();

        let res = raw_jsonb.as_i64();
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), expect_i64);

        let res = raw_jsonb.as_u64();
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), expect_u64);
    }
}

#[test]
fn test_to_type() {
    let sources = vec![
        (r#"null"#, None, None, None, None, None),
        (
            r#"true"#,
            Some(true),
            Some(1_i64),
            Some(1_u64),
            Some(1_f64),
            Some("true".to_string()),
        ),
        (
            r#"false"#,
            Some(false),
            Some(0_i64),
            Some(0_u64),
            Some(0_f64),
            Some("false".to_string()),
        ),
        (
            r#"1"#,
            None,
            Some(1_i64),
            Some(1_u64),
            Some(1_f64),
            Some("1".to_string()),
        ),
        (
            r#"-2"#,
            None,
            Some(-2_i64),
            None,
            Some(-2_f64),
            Some("-2".to_string()),
        ),
        (
            r#"1.2"#,
            None,
            Some(1_i64),
            Some(1_u64),
            Some(1.2_f64),
            Some("1.2".to_string()),
        ),
        (
            r#"1.5"#,
            None,
            Some(2_i64),
            Some(2_u64),
            Some(1.5_f64),
            Some("1.5".to_string()),
        ),
        (
            r#"-1.5"#,
            None,
            Some(-2_i64),
            None,
            Some(-1.5_f64),
            Some("-1.5".to_string()),
        ),
        (
            r#"1e-1"#,
            None,
            Some(0_i64),
            Some(0_u64),
            Some(0.1_f64),
            Some("0.1".to_string()),
        ),
        (
            r#"1.5e0"#,
            None,
            Some(2_i64),
            Some(2_u64),
            Some(1.5_f64),
            Some("1.5".to_string()),
        ),
        (
            r#""true""#,
            Some(true),
            None,
            None,
            None,
            Some("true".to_string()),
        ),
        (
            r#""false""#,
            Some(false),
            None,
            None,
            None,
            Some("false".to_string()),
        ),
        (
            r#""abcd""#,
            None,
            None,
            None,
            None,
            Some("abcd".to_string()),
        ),
        (
            r#""123.1""#,
            None,
            Some(123_i64),
            Some(123_u64),
            Some(123.1_f64),
            Some("123.1".to_string()),
        ),
        (
            r#""123.5""#,
            None,
            Some(124_i64),
            Some(124_u64),
            Some(123.5_f64),
            Some("123.5".to_string()),
        ),
        (
            r#""-1.5""#,
            None,
            Some(-2_i64),
            None,
            Some(-1.5_f64),
            Some("-1.5".to_string()),
        ),
        (
            r#""1.5e0""#,
            None,
            Some(2_i64),
            Some(2_u64),
            Some(1.5_f64),
            Some("1.5e0".to_string()),
        ),
        (
            r#""-1.5e0""#,
            None,
            Some(-2_i64),
            None,
            Some(-1.5_f64),
            Some("-1.5e0".to_string()),
        ),
    ];

    for (s, expect_bool, expect_i64, expect_u64, expect_f64, expect_str) in sources {
        let owned_jsonb = s.parse::<OwnedJsonb>().unwrap();
        let raw_jsonb = owned_jsonb.as_raw();

        let res = raw_jsonb.to_bool();
        match expect_bool {
            Some(expect) => {
                assert!(res.is_ok());
                assert_eq!(res.unwrap(), expect);
            }
            None => assert!(res.is_err()),
        }
        let res = raw_jsonb.to_i64();
        match expect_i64 {
            Some(expect) => {
                assert!(res.is_ok());
                assert_eq!(res.unwrap(), expect);
            }
            None => assert!(res.is_err()),
        }
        let res = raw_jsonb.to_u64();
        match expect_u64 {
            Some(expect) => {
                assert!(res.is_ok());
                assert_eq!(res.unwrap(), expect);
            }
            None => assert!(res.is_err()),
        }
        let res = raw_jsonb.to_f64();
        match expect_f64 {
            Some(expect) => {
                assert!(res.is_ok());
                assert_eq!(res.unwrap(), expect);
            }
            None => assert!(res.is_err()),
        }
        let res = raw_jsonb.to_str();
        match expect_str {
            Some(expect) => {
                assert!(res.is_ok());
                assert_eq!(res.unwrap(), expect);
            }
            None => assert!(res.is_err()),
        }
    }
}

#[test]
fn test_to_string() {
    let sources = vec![
        (r#"null"#, r#"null"#),
        (r#"true"#, r#"true"#),
        (r#"false"#, r#"false"#),
        (r#"1234567"#, r#"1234567"#),
        (r#"-1234567"#, r#"-1234567"#),
        (r#"123.4567"#, r#"123.4567"#),
        (r#""abcdef""#, r#""abcdef""#),
        (r#""ab\n\"\uD83D\uDC8E测试""#, r#""ab\n\"💎测试""#),
        (r#""မြန်မာဘာသာ""#, r#""မြန်မာဘာသာ""#),
        (r#""⚠️✅❌""#, r#""⚠️✅❌""#),
        (r#"[1,2,3,4]"#, r#"[1,2,3,4]"#),
        (
            r#"["a","b",true,false,[1,2,3],{"a":"b"}]"#,
            r#"["a","b",true,false,[1,2,3],{"a":"b"}]"#,
        ),
        (
            r#"{"k1":"v1","k2":[1,2,3],"k3":{"a":"b"}}"#,
            r#"{"k1":"v1","k2":[1,2,3],"k3":{"a":"b"}}"#,
        ),
    ];
    for (s, expect) in sources {
        let owned_jsonb = s.parse::<OwnedJsonb>().unwrap();
        let raw_jsonb = owned_jsonb.as_raw();
        let res = raw_jsonb.to_string();
        assert_eq!(res, expect);
    }

    let extension_sources = vec![
        (Value::Binary(&[97, 98, 99]), r#""616263""#),
        (Value::Binary(&[]), r#""""#),
        (Value::Date(Date { value: 90570 }), r#""2217-12-22""#),
        (Value::Date(Date { value: -1 }), r#""1969-12-31""#),
        (
            Value::Timestamp(Timestamp {
                value: 190390000000,
            }),
            r#""1970-01-03 04:53:10.000000""#,
        ),
        (
            Value::Timestamp(Timestamp { value: 0 }),
            r#""1970-01-01 00:00:00.000000""#,
        ),
        (
            Value::TimestampTz(TimestampTz {
                offset: 8 * 3600,
                value: 190390000000,
            }),
            r#""1970-01-03 12:53:10.000000 +0800""#,
        ),
        (
            Value::TimestampTz(TimestampTz {
                offset: 90 * 60,
                value: 0,
            }),
            r#""1970-01-01 01:30:00.000000 +0130""#,
        ),
        (
            Value::Interval(Interval {
                months: 10,
                days: 20,
                micros: 300000000,
            }),
            r#""10 months 20 days 00:05:00""#,
        ),
        (
            Value::Interval(Interval {
                months: -14,
                days: -3,
                micros: -90000000,
            }),
            r#""-1 year -2 months -3 days -00:01:30""#,
        ),
        (
            Value::Interval(Interval {
                months: -25,
                days: -1,
                micros: 0,
            }),
            r#""-2 years -1 month -1 day""#,
        ),
        (
            Value::Interval(Interval {
                months: 0,
                days: -2,
                micros: 5_000_000,
            }),
            r#""-2 days 00:00:05""#,
        ),
        (
            Value::Interval(Interval {
                months: 0,
                days: 0,
                micros: 0,
            }),
            r#""00:00:00""#,
        ),
        (
            Value::Number(Number::Decimal128(Decimal128 {
                scale: 2,
                value: 1234,
            })),
            r#"12.34"#,
        ),
        (
            Value::Number(Number::Decimal64(Decimal64 {
                scale: 2,
                value: -765,
            })),
            r#"-7.65"#,
        ),
        (
            Value::Number(Number::Decimal256(Decimal256 {
                scale: 2,
                value: I256::new(981724),
            })),
            r#"9817.24"#,
        ),
        (
            Value::Array(vec![
                Value::Binary(&[97, 98, 99]),
                Value::Date(Date { value: 90570 }),
                Value::Timestamp(Timestamp {
                    value: 190390000000,
                }),
                Value::TimestampTz(TimestampTz {
                    offset: 8 * 3600,
                    value: 190390000000,
                }),
                Value::Interval(Interval {
                    months: 10,
                    days: 20,
                    micros: 300000000,
                }),
                Value::Number(Number::Decimal128(Decimal128 {
                    scale: 2,
                    value: 1234,
                })),
                Value::Number(Number::Decimal256(Decimal256 {
                    scale: 2,
                    value: I256::new(981724),
                })),
            ]),
            r#"["616263","2217-12-22","1970-01-03 04:53:10.000000","1970-01-03 12:53:10.000000 +0800","10 months 20 days 00:05:00",12.34,9817.24]"#,
        ),
        (
            Value::Object(BTreeMap::from([
                ("k1".to_string(), Value::Binary(&[97, 98, 99])),
                ("k2".to_string(), Value::Date(Date { value: 90570 })),
                (
                    "k3".to_string(),
                    Value::Timestamp(Timestamp {
                        value: 190390000000,
                    }),
                ),
                (
                    "k4".to_string(),
                    Value::TimestampTz(TimestampTz {
                        offset: 8 * 3600,
                        value: 190390000000,
                    }),
                ),
                (
                    "k5".to_string(),
                    Value::Interval(Interval {
                        months: 10,
                        days: 20,
                        micros: 300000000,
                    }),
                ),
                (
                    "k6".to_string(),
                    Value::Number(Number::Decimal128(Decimal128 {
                        scale: 2,
                        value: 1234,
                    })),
                ),
                (
                    "k7".to_string(),
                    Value::Number(Number::Decimal256(Decimal256 {
                        scale: 2,
                        value: I256::new(981724),
                    })),
                ),
            ])),
            r#"{"k1":"616263","k2":"2217-12-22","k3":"1970-01-03 04:53:10.000000","k4":"1970-01-03 12:53:10.000000 +0800","k5":"10 months 20 days 00:05:00","k6":12.34,"k7":9817.24}"#,
        ),
    ];

    for (v, expect) in extension_sources {
        let buf = v.to_vec();
        let raw_jsonb = RawJsonb::new(&buf);

        let res = raw_jsonb.to_string();
        assert_eq!(res, expect);
    }
}

#[test]
fn test_to_pretty_string() {
    let sources = vec![
        (r#"null"#, r#"null"#),
        (r#"true"#, r#"true"#),
        (r#"false"#, r#"false"#),
        (r#"1234567"#, r#"1234567"#),
        (r#"-1234567"#, r#"-1234567"#),
        (r#"123.4567"#, r#"123.4567"#),
        (r#""abcdef""#, r#""abcdef""#),
        (r#""ab\n\"\uD83D\uDC8E测试""#, r#""ab\n\"💎测试""#),
        (r#""မြန်မာဘာသာ""#, r#""မြန်မာဘာသာ""#),
        (r#""⚠️✅❌""#, r#""⚠️✅❌""#),
        (r#"[1,2,3,4]"#, "[\n  1,\n  2,\n  3,\n  4\n]"),
        (
            r#"[1,2,3,4]"#,
            r#"[
  1,
  2,
  3,
  4
]"#,
        ),
        (
            r#"["a","b",true,false,[1,2,3],{"a":"b"}]"#,
            r#"[
  "a",
  "b",
  true,
  false,
  [
    1,
    2,
    3
  ],
  {
    "a": "b"
  }
]"#,
        ),
        (
            r#"{"k1":"v1","k2":[1,2,3],"k3":{"a":"b"}}"#,
            r#"{
  "k1": "v1",
  "k2": [
    1,
    2,
    3
  ],
  "k3": {
    "a": "b"
  }
}"#,
        ),
    ];

    for (s, expect) in sources {
        let owned_jsonb = s.parse::<OwnedJsonb>().unwrap();
        let raw_jsonb = owned_jsonb.as_raw();
        let res = raw_jsonb.to_pretty_string();
        assert_eq!(res, expect);
    }
}

#[test]
fn test_traverse_check_string() {
    let sources = vec![
        (r#"null"#, false),
        (r#"11"#, false),
        (r#""a""#, false),
        (r#""c""#, true),
        (r#"[1,2,3,4,"b"]"#, false),
        (r#"[1,2,[3,[4,"c"]]]"#, true),
        (r#"[true,false,[1,2,3],{"a":"b"}]"#, false),
        (
            r#"{"a":true,"b":{"b1":"v1","b2":11},"c":[true,12,"c1","c2"]}"#,
            true,
        ),
        (
            r#"{"a0":true,"b0":{"b1":"v1","b2":11},"c0":[true,12,"c1","c"]}"#,
            true,
        ),
        (
            r#"{"a0":true,"b0":{"b1":"v1","b2":11},"c0":[true,12,"c1","c2"]}"#,
            false,
        ),
    ];
    for (s, expect) in sources {
        let owned_jsonb = s.parse::<OwnedJsonb>().unwrap();
        let raw_jsonb = owned_jsonb.as_raw();
        let res = raw_jsonb.traverse_check_string(|v| {
            let s = unsafe { std::str::from_utf8_unchecked(v) };
            s == "c"
        });
        assert_eq!(res, Ok(expect));
    }
}

#[test]
fn test_strip_nulls() {
    let sources = vec![
        (r#"null"#, r#"null"#),
        (r#"1"#, r#"1"#),
        (r#"[1,2,3,null]"#, r#"[1,2,3,null]"#),
        (
            r#"{"a":null, "b":{"a":null,"b":1},"c":[1,null,2]}"#,
            r#"{"b":{"b":1},"c":[1,null,2]}"#,
        ),
    ];

    for (s, expect) in sources {
        let owned_jsonb = s.parse::<OwnedJsonb>().unwrap();
        let raw_jsonb = owned_jsonb.as_raw();

        let res = raw_jsonb.strip_nulls();
        let expect_jsonb = expect.parse::<OwnedJsonb>().unwrap();
        assert_eq!(res, Ok(expect_jsonb));
    }
}

#[test]
#[cfg(feature = "arbitrary_precision")]
fn test_decimal_type_of() {
    let sources = vec![
        (r#"-1.2"#, "DECIMAL"),
        (r#"1.9120000000000001"#, "DECIMAL"),
        (
            r#"99999999999999999999999999999999999999999999999999999999.99999999999999999999"#,
            "DECIMAL",
        ),
        (
            r#"-9999999999999999999999999999999999999999999999999999999999999999999999999999"#,
            "DECIMAL",
        ),
    ];

    for (s, expect) in sources {
        let owned_jsonb = s.parse::<OwnedJsonb>().unwrap();
        let raw_jsonb = owned_jsonb.as_raw();

        let res = raw_jsonb.type_of();
        assert_eq!(res, Ok(expect));
    }
}

#[test]
fn test_type_of() {
    let sources = vec![
        (r#"null"#, "NULL_VALUE"),
        (r#"1"#, "INTEGER"),
        (r#"1.912000000000000e+02"#, "DOUBLE"),
        (r#""test""#, "STRING"),
        (r#"[1,2,3,4,5]"#, "ARRAY"),
        (r#"{"a":1,"b":2}"#, "OBJECT"),
    ];

    for (s, expect) in sources {
        let owned_jsonb = s.parse::<OwnedJsonb>().unwrap();
        let raw_jsonb = owned_jsonb.as_raw();

        let res = raw_jsonb.type_of();
        assert_eq!(res, Ok(expect));
    }

    let extension_sources = vec![
        (Value::Binary(&[97, 98, 99]), "BINARY"),
        (Value::Date(Date { value: 90570 }), "DATE"),
        (
            Value::Timestamp(Timestamp {
                value: 190390000000,
            }),
            "TIMESTAMP",
        ),
        (
            Value::TimestampTz(TimestampTz {
                offset: 8 * 3600,
                value: 190390000000,
            }),
            "TIMESTAMP_TZ",
        ),
        (
            Value::Interval(Interval {
                months: 10,
                days: 20,
                micros: 300000000,
            }),
            "INTERVAL",
        ),
        (
            Value::Number(Number::Decimal64(Decimal64 {
                scale: 5,
                value: 111111111111,
            })),
            "DECIMAL",
        ),
        (
            Value::Number(Number::Decimal128(Decimal128 {
                scale: 2,
                value: 99999999999999999999999999999999999999,
            })),
            "DECIMAL",
        ),
        (
            Value::Number(Number::Decimal256(Decimal256 {
                scale: 2,
                value: I256::new(981724),
            })),
            "DECIMAL",
        ),
    ];
    for (v, expect) in extension_sources {
        let buf = v.to_vec();
        let raw_jsonb = RawJsonb::new(&buf);

        let res = raw_jsonb.type_of();
        assert_eq!(res, Ok(expect));
    }
}

#[test]
fn test_object_each() {
    let sources = vec![
        ("true", None),
        (r#"[1,2,3]"#, None),
        (
            r#"{"a":1,"b":false}"#,
            Some(vec![
                ("a", Value::Number(Number::UInt64(1))),
                ("b", Value::Bool(false)),
            ]),
        ),
        (
            r#"{"a":[1,2,3],"b":{"k":1}}"#,
            Some(vec![
                (
                    "a",
                    Value::Array(vec![
                        Value::Number(Number::UInt64(1)),
                        Value::Number(Number::UInt64(2)),
                        Value::Number(Number::UInt64(3)),
                    ]),
                ),
                (
                    "b",
                    init_object(vec![("k", Value::Number(Number::UInt64(1)))]),
                ),
            ]),
        ),
    ];
    for (src, expected) in sources {
        let owned_jsonb = src.parse::<OwnedJsonb>().unwrap();
        let raw_jsonb = owned_jsonb.as_raw();

        let res = raw_jsonb.object_each();
        assert!(res.is_ok());
        let owned_jsonbs_opt = res.unwrap();
        match expected {
            Some(expected) => {
                assert!(owned_jsonbs_opt.is_some());
                let owned_jsonbs = owned_jsonbs_opt.unwrap();
                for (v, e) in owned_jsonbs.into_iter().zip(expected.iter()) {
                    assert_eq!(v.0, e.0.to_string());
                    let expected_buf = e.1.to_vec();
                    assert_eq!(v.1.to_vec(), expected_buf);
                }
            }
            None => assert_eq!(owned_jsonbs_opt, None),
        }
    }
}

#[test]
fn test_get_by_keypath() {
    let sources = vec![
        ("null", " {  } ", Some(Value::Null)),
        ("null", " { a , b } ", None),
        ("true", "{a,b}", None),
        (r#"{"a":{"b":null}}"#, "{a,b}", Some(Value::Null)),
        (r#"[1,"a",null]"#, "{2}", Some(Value::Null)),
        (r#""sdasd""#, "{1}", None),
        ("[10,20,30]", "{1}", Some(Value::Number(Number::UInt64(20)))),
        (
            "[10,20,30]",
            "{-1}",
            Some(Value::Number(Number::UInt64(30))),
        ),
        (
            r#"[10,20,["a","b","c"]]"#,
            "{2,0}",
            Some(Value::String(Cow::from("a"))),
        ),
        (r#"[10,20,["a","b","c"]]"#, "{2,a}", None),
        (
            r#"{"1":{"2":"abc"}}"#,
            "{1,2}",
            Some(Value::String(std::borrow::Cow::Borrowed("abc"))),
        ),
        (
            r#"[10,20,[{"k1":[1,2,3],"k2":{"w":1,"z":2}},"b","c"]]"#,
            "{2,0,k2}",
            Some(init_object(vec![
                ("w", Value::Number(Number::UInt64(1))),
                ("z", Value::Number(Number::UInt64(2))),
            ])),
        ),
    ];
    for (json_str, path_str, expected) in sources {
        let owned_jsonb = json_str.parse::<OwnedJsonb>().unwrap();
        let raw_jsonb = owned_jsonb.as_raw();
        let key_paths = parse_key_paths(path_str.as_bytes()).unwrap();

        let res = raw_jsonb.get_by_keypath(key_paths.paths.iter());
        assert!(res.is_ok());
        let res = res.unwrap();
        match expected {
            Some(e) => {
                assert!(res.is_some());
                let owned_jsonb = res.unwrap();
                let expected_buf = e.to_vec();
                assert_eq!(owned_jsonb.to_vec(), expected_buf);
            }
            None => assert_eq!(res, None),
        }
    }
}

#[test]
fn test_exists_all_keys() {
    let sources = vec![
        (r#"true"#, vec!["10", "20", "40"], false),
        (r#"[]"#, vec!["10", "20", "40"], false),
        (r#"{}"#, vec!["10", "20", "40"], false),
        (r#"["10","20","30"]"#, vec!["10", "20", "40"], false),
        (r#"["10","20","30"]"#, vec!["10", "20", "30"], true),
        (r#"[10,20,30]"#, vec!["10", "20", "30"], false),
        (r#"["10","20","30"]"#, vec!["10", "20", "20"], true),
        (r#"{"a":1,"b":2,"c":3}"#, vec!["c", "b", "a"], true),
        (r#"{"a":1,"b":2,"c":3}"#, vec!["a", "b", "a"], true),
        (r#"{"a":1,"b":2,"c":3}"#, vec!["c", "f", "a"], false),
        (r#""a""#, vec!["c", "f", "a"], false),
        (r#""b""#, vec!["b"], true),
    ];
    for (json, keys, expected) in sources {
        let owned_jsonb = json.parse::<OwnedJsonb>().unwrap();
        let raw_jsonb = owned_jsonb.as_raw();

        let res = raw_jsonb.exists_all_keys(keys.into_iter());
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), expected);
    }
}

#[test]
fn test_exists_any_keys() {
    let sources = vec![
        (r#"true"#, vec!["10", "20", "40"], false),
        (r#"[]"#, vec!["10", "20", "40"], false),
        (r#"{}"#, vec!["10", "20", "40"], false),
        (r#"[10,20,30]"#, vec!["10", "20", "30"], false),
        (r#"["10","20","30"]"#, vec!["10", "20", "40"], true),
        (r#"["10","20","30"]"#, vec!["10", "20", "30"], true),
        (r#"["10","20","30"]"#, vec!["40", "50", "60"], false),
        (r#"{"a":1,"b":2,"c":3}"#, vec!["c", "b", "a"], true),
        (r#"{"a":1,"b":2,"c":3}"#, vec!["a", "b", "a"], true),
        (r#"{"a":1,"b":2,"c":3}"#, vec!["z", "f", "x"], false),
        (r#""a""#, vec!["c", "f", "a"], true),
        (r#""b""#, vec!["b"], true),
    ];
    for (json, keys, expected) in sources {
        let owned_jsonb = json.parse::<OwnedJsonb>().unwrap();
        let raw_jsonb = owned_jsonb.as_raw();

        let res = raw_jsonb.exists_any_keys(keys.into_iter());
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), expected);
    }
}

#[test]
fn test_contains() {
    let sources = vec![
        ("true", "true", true),
        ("false", "true", false),
        ("1", "1", true),
        ("1", "2", false),
        (r#""asd""#, r#""asd""#, true),
        (r#""asd""#, r#""dsa""#, false),
        (r#"[1,2,3,4]"#, "1", true),
        (r#"[1,2,3,4]"#, "5", false),
        (r#"["1","2","3","4"]"#, r#""1""#, true),
        (r#"[1,2,3,4]"#, "[2,1,3]", true),
        (r#"[1,2,3,4]"#, "[2,1,1]", true),
        (r#"[1,2,[1,3]]"#, "[1,3]", false),
        (r#"[1,2,[1,3]]"#, "[[1,3]]", true),
        (r#"[1,2,[1,3]]"#, "[[[1,3]]]", false),
        (r#"[{"a":1}]"#, r#"{"a":1}"#, false),
        (r#"[{"a":1},{"b":2}]"#, r#"[{"a":1}]"#, true),
        (r#"{"a":1,"b":2}"#, r#"{"a":1}"#, true),
        (r#"{"a":1,"b":2}"#, r#"{"a":2}"#, false),
        (r#"{"z":2,"b":{"a":1}}"#, r#"{"a":1}"#, false),
        (r#"{"a":{"c":100,"d":200},"b":2}"#, r#"{"a":{}}"#, true),
    ];
    for (left, right, expected) in sources {
        let left_owned_jsonb = left.parse::<OwnedJsonb>().unwrap();
        let left_raw_jsonb = left_owned_jsonb.as_raw();
        let right_owned_jsonb = right.parse::<OwnedJsonb>().unwrap();
        let right_raw_jsonb = right_owned_jsonb.as_raw();

        let res = left_raw_jsonb.contains(&right_raw_jsonb);
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), expected);
    }
}

#[test]
fn test_path_match() {
    let sources = vec![
        (r#"{"a":1,"b":2}"#, r#"$.a == 1"#, Some(true)),
        (r#"{"a":1,"b":2}"#, r#"$.a > 1"#, Some(false)),
        (r#"{"a":1,"b":2}"#, r#"$.c > 0"#, Some(false)),
        (r#"{"a":1,"b":2}"#, r#"$.b < 2"#, Some(false)),
        (r#"{"a":1,"b":[1,2,3]}"#, r#"$.b[0] == 1"#, Some(true)),
        (r#"{"a":1,"b":[1,2,3]}"#, r#"$.b[0] > 1"#, Some(false)),
        (r#"{"a":1,"b":[1,2,3]}"#, r#"$.b[3] == 0"#, Some(false)),
        (
            r#"{"a":1,"b":[1,2,3]}"#,
            r#"$.b[1 to last] >= 2"#,
            Some(true),
        ),
        (
            r#"{"a":1,"b":[1,2,3]}"#,
            r#"$.b[1 to last] == 2 || $.b[1 to last] == 3"#,
            Some(true),
        ),
        (r#""b""#, r#"$[*] == "b""#, Some(true)),
        (r#""b""#, r#"$[*] == 123"#, None),
    ];
    for (json, predicate, expected) in sources {
        let owned_jsonb = json.parse::<OwnedJsonb>().unwrap();
        let raw_jsonb = owned_jsonb.as_raw();
        let json_path = parse_json_path(predicate.as_bytes()).unwrap();
        let res = raw_jsonb.path_match(&json_path);
        assert_eq!(res, Ok(expected));
    }
}

#[test]
fn test_concat() {
    let sources = vec![
        ("null", "null", "[null,null]"),
        ("true", "null", "[true,null]"),
        ("1", r#""asdasd""#, r#"[1,"asdasd"]"#),
        (r#""asd""#, r#"[1,2,3]"#, r#"["asd",1,2,3]"#),
        (r#"[1,2,3]"#, r#""asd""#, r#"[1,2,3,"asd"]"#),
        (
            r#"[1,{"a":1,"b":2,"c":[1,2,3]},3]"#,
            r#""asd""#,
            r#"[1,{"a":1,"b":2,"c":[1,2,3]},3,"asd"]"#,
        ),
        (
            r#"[1,{"a":1,"b":2,"c":[1,2,3]},3]"#,
            r#"[10,20,30]"#,
            r#"[1,{"a":1,"b":2,"c":[1,2,3]},3,10,20,30]"#,
        ),
        (
            r#"[1,[1,2,3],3]"#,
            r#"[[10,20,30]]"#,
            r#"[1,[1,2,3],3,[10,20,30]]"#,
        ),
        (r#"{"a":1,"b":2}"#, r#"true"#, r#"[{"a":1,"b":2},true]"#),
        (r#"{"a":1,"b":2}"#, r#"123"#, r#"[{"a":1,"b":2},123]"#),
        (r#"{"a":1,"b":2}"#, r#""asd""#, r#"[{"a":1,"b":2},"asd"]"#),
        (r#"[1,2,3]"#, r#"{"a":1,"b":2}"#, r#"[1,2,3,{"a":1,"b":2}]"#),
        (r#"{"a":1,"b":2}"#, r#"[1,2,3]"#, r#"[{"a":1,"b":2},1,2,3]"#),
        (
            r#"{"a":1,"b":2}"#,
            r#"{"c":3,"d":4}"#,
            r#"{"a":1,"b":2,"c":3,"d":4}"#,
        ),
        (
            r#"{"a":1,"b":2,"d":10}"#,
            r#"{"a":3,"b":4}"#,
            r#"{"a":3,"b":4,"d":10}"#,
        ),
    ];
    for (left, right, expected) in sources {
        let left_owned_jsonb = left.parse::<OwnedJsonb>().unwrap();
        let left_raw_jsonb = left_owned_jsonb.as_raw();
        let right_owned_jsonb = right.parse::<OwnedJsonb>().unwrap();
        let right_raw_jsonb = right_owned_jsonb.as_raw();

        let res = left_raw_jsonb.concat(&right_raw_jsonb);
        assert!(res.is_ok());
        let res_owned_jsonb = res.unwrap();

        let expected_owned_jsonb = expected.parse::<OwnedJsonb>().unwrap();
        assert_eq!(res_owned_jsonb.to_vec(), expected_owned_jsonb.to_vec());
    }
}

#[test]
fn test_delete_by_name() {
    let sources = vec![
        ("[1,2,3]", "1", "[1,2,3]"),
        (r#"["1","2","3"]"#, "0", r#"["1","2","3"]"#),
        (r#"["1","2","3"]"#, "1", r#"["2","3"]"#),
        (
            r#"["1","2","3",{"a":1,"b":2}]"#,
            "1",
            r#"["2","3",{"a":1,"b":2}]"#,
        ),
        (r#"{"a":1,"b":2}"#, "c", r#"{"a":1,"b":2}"#),
        (r#"{"a":1,"b":2}"#, "a", r#"{"b":2}"#),
        (r#"{"b":2}"#, "b", "{}"),
    ];
    for (json, name, expected) in sources {
        let owned_jsonb = json.parse::<OwnedJsonb>().unwrap();
        let raw_jsonb = owned_jsonb.as_raw();

        let res = raw_jsonb.delete_by_name(name);
        assert!(res.is_ok());
        let res_jsonb = res.unwrap();
        let expected_jsonb = expected.parse::<OwnedJsonb>().unwrap();
        assert_eq!(res_jsonb, expected_jsonb);
    }
}

#[test]
fn test_delete_by_name_errors() {
    let sources = vec![(r#""asd""#, "asd"), ("true", "true"), ("1", "1")];
    for (json, name) in sources {
        let owned_jsonb = json.parse::<OwnedJsonb>().unwrap();
        let raw_jsonb = owned_jsonb.as_raw();

        let res = raw_jsonb.delete_by_name(name);
        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), Error::InvalidJsonType));
    }
}

#[test]
fn test_delete_by_index() {
    let sources = vec![
        ("[1,2,3]", 0, "[2,3]"),
        ("[1,2,3]", 1, "[1,3]"),
        ("[1,2,3]", 2, "[1,2]"),
        ("[1,2,3]", -1, "[1,2]"),
        ("[1,2,3]", -2, "[1,3]"),
        ("[1,2,3]", -3, "[2,3]"),
        ("[1,2,3]", -4, "[1,2,3]"),
        (r#"[1,2,{"a":[1,2,3],"b":[40,50,60]}]"#, 2, "[1,2]"),
    ];
    for (json, index, expected) in sources {
        let owned_jsonb = json.parse::<OwnedJsonb>().unwrap();
        let raw_jsonb = owned_jsonb.as_raw();

        let res = raw_jsonb.delete_by_index(index);
        assert!(res.is_ok());
        let res_jsonb = res.unwrap();
        let expected_jsonb = expected.parse::<OwnedJsonb>().unwrap();
        assert_eq!(res_jsonb, expected_jsonb);
    }
}

#[test]
fn test_delete_by_index_errors() {
    let sources = vec![
        (r#""asd""#, 1),
        ("true", 0),
        ("1", 10),
        (r#"{"a":1,"b":2}"#, 20),
    ];
    for (json, index) in sources {
        let owned_jsonb = json.parse::<OwnedJsonb>().unwrap();
        let raw_jsonb = owned_jsonb.as_raw();

        let res = raw_jsonb.delete_by_index(index);
        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), Error::InvalidJsonType));
    }
}

#[test]
fn test_delete_by_keypath() {
    let sources = vec![
        (r#"{"a":1,"b":[1,2,3]}"#, "{}", r#"{"a":1,"b":[1,2,3]}"#),
        (r#"{"a":1,"b":[1,2,3]}"#, "{b}", r#"{"a":1}"#),
        (r#"{"a":1,"b":[1,2,3]}"#, "{b,2}", r#"{"a":1,"b":[1,2]}"#),
        (r#"{"a":1,"b":[1,2,3]}"#, "{b,-2}", r#"{"a":1,"b":[1,3]}"#),
        (
            r#"{"a":1,"b":[{"c":1,"d":10},2,3]}"#,
            "{b,0,d}",
            r#"{"a":1,"b":[{"c":1},2,3]}"#,
        ),
        (r#"{"a":1,"b":[1,2,3]}"#, "{b,20}", r#"{"a":1,"b":[1,2,3]}"#),
        (
            r#"{"1":{"2":"abc","3":"def"}}"#,
            "{1,2}",
            r#"{"1":{"3":"def"}}"#,
        ),
        (
            r#"{"a":1,"b":[1,2,3]}"#,
            "{b,20,c,e}",
            r#"{"a":1,"b":[1,2,3]}"#,
        ),
    ];
    for (json, keypath, expected) in sources {
        let owned_jsonb = json.parse::<OwnedJsonb>().unwrap();
        let raw_jsonb = owned_jsonb.as_raw();
        let keypath = parse_key_paths(keypath.as_bytes()).unwrap();

        let res = raw_jsonb.delete_by_keypath(keypath.paths.iter());
        assert!(res.is_ok());
        let res_jsonb = res.unwrap();
        let expected_jsonb = expected.parse::<OwnedJsonb>().unwrap();
        assert_eq!(res_jsonb, expected_jsonb);
    }
}

#[test]
fn test_array_insert() {
    let sources = vec![
        (r#"[0,1,2,3]"#, 2, r#""hello""#, r#"[0,1,"hello",2,3]"#),
        (r#"[0,1,2,3]"#, 10, r#"100"#, r#"[0,1,2,3,100]"#),
        (r#"[0,1,2,3]"#, 0, r#"true"#, r#"[true,0,1,2,3]"#),
        (r#"[0,1,2,3]"#, -1, r#"{"k":"v"}"#, r#"[0,1,2,{"k":"v"},3]"#),
        (r#"1"#, 1, r#"{"k":"v"}"#, r#"[1,{"k":"v"}]"#),
        (r#"{"k":"v"}"#, 2, r#"true"#, r#"[{"k":"v"},true]"#),
    ];
    for (val, pos, new_val, expected) in sources {
        let owned_jsonb = val.parse::<OwnedJsonb>().unwrap();
        let raw_jsonb = owned_jsonb.as_raw();
        let new_owned_jsonb = new_val.parse::<OwnedJsonb>().unwrap();
        let new_raw_jsonb = new_owned_jsonb.as_raw();

        let res = raw_jsonb.array_insert(pos, &new_raw_jsonb);
        assert!(res.is_ok());
        let res_jsonb = res.unwrap();
        let expected_jsonb = expected.parse::<OwnedJsonb>().unwrap();
        assert_eq!(res_jsonb, expected_jsonb);
    }
}

#[test]
fn test_array_distinct() {
    let sources = vec![
        (r#"[0,1,1,2,2,2,3,4]"#, r#"[0,1,2,3,4]"#),
        (r#"["A","A","B","C","A","C"]"#, r#"["A","B","C"]"#),
        (
            r#"["A","A",10,false,null,false,null,10]"#,
            r#"["A",10,false,null]"#,
        ),
        (r#"[[1,2,2],3,4,[1,2,2]]"#, r#"[[1,2,2],3,4]"#),
        (
            r#"[{"k":"v"},"A","A","B",{"k":"v"}]"#,
            r#"[{"k":"v"},"A","B"]"#,
        ),
        (r#"1"#, r#"[1]"#),
        (r#"{"k":"v"}"#, r#"[{"k":"v"}]"#),
    ];
    for (val, expected) in sources {
        let owned_jsonb = val.parse::<OwnedJsonb>().unwrap();
        let raw_jsonb = owned_jsonb.as_raw();

        let res = raw_jsonb.array_distinct();
        assert!(res.is_ok());
        let res_jsonb = res.unwrap();
        let expected_jsonb = expected.parse::<OwnedJsonb>().unwrap();
        assert_eq!(res_jsonb, expected_jsonb);
    }
}

#[test]
fn test_array_intersection() {
    let sources = vec![
        (r#"["A","B","C"]"#, r#"["B","C"]"#, r#"["B","C"]"#),
        (r#"["A","B","B","B","C"]"#, r#"["B","B"]"#, r#"["B","B"]"#),
        (r#"[1,2]"#, r#"[3,4]"#, r#"[]"#),
        (r#"[null,102,null]"#, r#"[null,null,103]"#, r#"[null,null]"#),
        (
            r#"[{"a":1,"b":2},1,2]"#,
            r#"[{"a":1,"b":2},3,4]"#,
            r#"[{"a":1,"b":2}]"#,
        ),
        (r#"[{"a":1,"b":2},1,2]"#, r#"[{"a":2,"c":3},3,4]"#, r#"[]"#),
        (
            r#"[{"a":1,"b":2,"c":3}]"#,
            r#"[{"c":3,"b":2,"a":1},3,4]"#,
            r#"[{"a":1,"b":2,"c":3}]"#,
        ),
        (r#"1"#, r#"1"#, r#"[1]"#),
        (r#"1"#, r#"2"#, r#"[]"#),
        (r#"{"k":"v"}"#, r#"{"k":"v"}"#, r#"[{"k":"v"}]"#),
    ];
    for (left, right, expected) in sources {
        let left_owned_jsonb = left.parse::<OwnedJsonb>().unwrap();
        let left_raw_jsonb = left_owned_jsonb.as_raw();
        let right_owned_jsonb = right.parse::<OwnedJsonb>().unwrap();
        let right_raw_jsonb = right_owned_jsonb.as_raw();

        let res = left_raw_jsonb.array_intersection(&right_raw_jsonb);
        assert!(res.is_ok());
        let res_jsonb = res.unwrap();
        let expected_jsonb = expected.parse::<OwnedJsonb>().unwrap();
        assert_eq!(res_jsonb, expected_jsonb);
    }
}

#[test]
fn test_array_except() {
    let sources = vec![
        (r#"["A","B","C"]"#, r#"["B","C"]"#, r#"["A"]"#),
        (
            r#"["A","B","B","B","C"]"#,
            r#"["B","B"]"#,
            r#"["A","B","C"]"#,
        ),
        (r#"[1,2]"#, r#"[3,4]"#, r#"[1,2]"#),
        (r#"[null,102,null]"#, r#"[null,null,103]"#, r#"[102]"#),
        (
            r#"[{"a":1,"b":2},1,2]"#,
            r#"[{"a":1,"b":2},3,4]"#,
            r#"[1,2]"#,
        ),
        (
            r#"[{"a":1,"b":2},1,2]"#,
            r#"[{"a":2,"c":3},3,4]"#,
            r#"[{"a":1,"b":2},1,2]"#,
        ),
        (
            r#"[{"a":1,"b":2,"c":3}]"#,
            r#"[{"c":3,"b":2,"a":1},3,4]"#,
            r#"[]"#,
        ),
        (r#"1"#, r#"1"#, r#"[]"#),
        (r#"1"#, r#"2"#, r#"[1]"#),
        (r#"{"k":"v"}"#, r#"{"k":"v"}"#, r#"[]"#),
    ];
    for (left, right, expected) in sources {
        let left_owned_jsonb = left.parse::<OwnedJsonb>().unwrap();
        let left_raw_jsonb = left_owned_jsonb.as_raw();
        let right_owned_jsonb = right.parse::<OwnedJsonb>().unwrap();
        let right_raw_jsonb = right_owned_jsonb.as_raw();

        let res = left_raw_jsonb.array_except(&right_raw_jsonb);
        assert!(res.is_ok());
        let res_jsonb = res.unwrap();
        let expected_jsonb = expected.parse::<OwnedJsonb>().unwrap();
        assert_eq!(res_jsonb, expected_jsonb);
    }
}

#[test]
fn test_array_overlap() {
    let sources = vec![
        (r#"["A","B","C"]"#, r#"["B","C"]"#, true),
        (r#"["A","B","B","B","C"]"#, r#"["B","B"]"#, true),
        (r#"[1,2]"#, r#"[3,4]"#, false),
        (r#"[null,102,null]"#, r#"[null,null,103]"#, true),
        (r#"[{"a":1,"b":2},1,2]"#, r#"[{"a":1,"b":2},3,4]"#, true),
        (r#"[{"a":1,"b":2},1,2]"#, r#"[{"a":2,"c":3},3,4]"#, false),
        (
            r#"[{"a":1,"b":2,"c":3}]"#,
            r#"[{"c":3,"b":2,"a":1},3,4]"#,
            true,
        ),
        (r#"1"#, r#"1"#, true),
        (r#"1"#, r#"2"#, false),
        (r#"{"k":"v"}"#, r#"{"k":"v"}"#, true),
    ];
    for (left, right, expected) in sources {
        let left_owned_jsonb = left.parse::<OwnedJsonb>().unwrap();
        let left_raw_jsonb = left_owned_jsonb.as_raw();
        let right_owned_jsonb = right.parse::<OwnedJsonb>().unwrap();
        let right_raw_jsonb = right_owned_jsonb.as_raw();

        let res = left_raw_jsonb.array_overlap(&right_raw_jsonb);
        assert!(res.is_ok());
        let res = res.unwrap();
        assert_eq!(res, expected);
    }
}

#[test]
fn test_object_insert() {
    let sources = vec![
        (
            r#"{"b":11,"d":22,"m":[1,2]}"#,
            "a",
            r#""hello""#,
            false,
            Some(r#"{"a":"hello","b":11,"d":22,"m":[1,2]}"#),
        ),
        (
            r#"{"b":11,"d":22,"m":[1,2]}"#,
            "e",
            r#"{"k":"v"}"#,
            false,
            Some(r#"{"b":11,"d":22,"e":{"k":"v"},"m":[1,2]}"#),
        ),
        (
            r#"{"b":11,"d":22,"m":[1,2]}"#,
            "z",
            r#"["z1","z2"]"#,
            false,
            Some(r#"{"b":11,"d":22,"m":[1,2],"z":["z1","z2"]}"#),
        ),
        (r#"{"b":11,"d":22,"m":[1,2]}"#, "d", r#"100"#, false, None),
        (
            r#"{"b":11,"d":22,"m":[1,2]}"#,
            "d",
            r#"100"#,
            true,
            Some(r#"{"b":11,"d":100,"m":[1,2]}"#),
        ),
        (r#"{"b":11,"d":22,"m":[1,2]}"#, "m", r#"true"#, false, None),
        (
            r#"{"b":11,"d":22,"m":[1,2]}"#,
            "m",
            r#"true"#,
            true,
            Some(r#"{"b":11,"d":22,"m":true}"#),
        ),
        (r#"1"#, "xx", r#"{"k":"v"}"#, true, None),
    ];
    for (val, new_key, new_val, update_flag, expected) in sources {
        let owned_jsonb = val.parse::<OwnedJsonb>().unwrap();
        let raw_jsonb = owned_jsonb.as_raw();
        let new_owned_jsonb = new_val.parse::<OwnedJsonb>().unwrap();
        let new_raw_jsonb = new_owned_jsonb.as_raw();

        let res = raw_jsonb.object_insert(new_key, &new_raw_jsonb, update_flag);
        match expected {
            Some(expected) => {
                assert!(res.is_ok());
                let res_jsonb = res.unwrap();
                let expected_jsonb = expected.parse::<OwnedJsonb>().unwrap();
                assert_eq!(res_jsonb, expected_jsonb);
            }
            None => assert!(res.is_err()),
        }
    }
}

#[test]
fn test_object_pick() {
    let sources = vec![
        (
            r#"{"b":11,"d":22,"m":[1,2]}"#,
            vec!["a", "b", "c"],
            Some(r#"{"b":11}"#),
        ),
        (
            r#"{"b":11,"d":22,"m":[1,2]}"#,
            vec!["a", "x", "y"],
            Some(r#"{}"#),
        ),
        (
            r#"{"k1":"v1","k2":{"x":"y"}}"#,
            vec!["k1"],
            Some(r#"{"k1":"v1"}"#),
        ),
        (r#"1"#, vec!["a", "b"], None),
    ];
    for (val, keys, expected) in sources {
        let owned_jsonb = val.parse::<OwnedJsonb>().unwrap();
        let raw_jsonb = owned_jsonb.as_raw();
        let keys = BTreeSet::from_iter(keys);

        let res = raw_jsonb.object_pick(&keys);
        match expected {
            Some(expected) => {
                assert!(res.is_ok());
                let res_jsonb = res.unwrap();
                let expected_jsonb = expected.parse::<OwnedJsonb>().unwrap();
                assert_eq!(res_jsonb, expected_jsonb);
            }
            None => assert!(res.is_err()),
        }
    }
}

#[test]
fn test_object_delete() {
    let sources = vec![
        (
            r#"{"b":11,"d":22,"m":[1,2]}"#,
            vec!["a", "b", "c"],
            Some(r#"{"d":22,"m":[1,2]}"#),
        ),
        (
            r#"{"b":11,"d":22,"m":[1,2]}"#,
            vec!["a", "x", "y"],
            Some(r#"{"b":11,"d":22,"m":[1,2]}"#),
        ),
        (
            r#"{"k1":"v1","k2":{"x":"y"}}"#,
            vec!["k1"],
            Some(r#"{"k2":{"x":"y"}}"#),
        ),
        (r#"1"#, vec!["a", "b"], None),
    ];
    for (val, keys, expected) in sources {
        let owned_jsonb = val.parse::<OwnedJsonb>().unwrap();
        let raw_jsonb = owned_jsonb.as_raw();
        let keys = BTreeSet::from_iter(keys);

        let res = raw_jsonb.object_delete(&keys);
        match expected {
            Some(expected) => {
                assert!(res.is_ok());
                let res_jsonb = res.unwrap();
                let expected_jsonb = expected.parse::<OwnedJsonb>().unwrap();
                assert_eq!(res_jsonb, expected_jsonb);
            }
            None => assert!(res.is_err()),
        }
    }
}

#[test]
fn test_to_serde_json() {
    let sources = vec![
        r#"true"#,
        r#"1e20"#,
        r#"[100,"abc",{"xx":"✅❌💻"}]"#,
        r#"{"a":1,"b":[1,2,3]}"#,
        r#"{"ab":{"k1":"v1","k2":"v2"},"cd":[true,100.23,"测试"]}"#,
    ];
    for s in sources {
        let owned_jsonb = s.parse::<OwnedJsonb>().unwrap();
        let raw_jsonb = owned_jsonb.as_raw();
        let jsonb_val_str = raw_jsonb.to_string();

        let serde_json_val = from_raw_jsonb::<serde_json::Value>(&raw_jsonb).unwrap();
        let serde_json_val_str = serde_json_val.to_string();
        assert_eq!(jsonb_val_str, serde_json_val_str);

        let serde_json_obj_val =
            from_raw_jsonb::<serde_json::Map<String, serde_json::Value>>(&raw_jsonb);
        if raw_jsonb.is_object().unwrap() {
            assert!(serde_json_obj_val.is_ok());
            let serde_json_obj_val = serde_json_obj_val.unwrap();
            let serde_json_val = serde_json::Value::Object(serde_json_obj_val);
            let serde_json_obj_val_str = serde_json_val.to_string();
            assert_eq!(jsonb_val_str, serde_json_obj_val_str);
        } else {
            assert!(serde_json_obj_val.is_err());
        }
    }
}

#[test]
fn test_visit_scalar_key_paths() {
    let json = r#"{"user":{"name":"Alice","scores":[85,92]},"empty":{}}"#;
    let jsonb = json.parse::<OwnedJsonb>().unwrap();
    let raw_jsonb = jsonb.as_raw();

    let mut paths = Vec::new();
    raw_jsonb
        .visit_scalar_key_paths(false, |path| {
            paths.push(KeyPaths {
                paths: path.to_vec(),
            });
            Ok(())
        })
        .unwrap();
    assert_eq!(
        paths,
        vec![
            KeyPaths {
                paths: vec![KeyPath::Name(Cow::Borrowed("empty"))],
            },
            KeyPaths {
                paths: vec![
                    KeyPath::Name(Cow::Borrowed("user")),
                    KeyPath::Name(Cow::Borrowed("name")),
                ],
            },
            KeyPaths {
                paths: vec![
                    KeyPath::Name(Cow::Borrowed("user")),
                    KeyPath::Name(Cow::Borrowed("scores")),
                    KeyPath::Index(0),
                ],
            },
            KeyPaths {
                paths: vec![
                    KeyPath::Name(Cow::Borrowed("user")),
                    KeyPath::Name(Cow::Borrowed("scores")),
                    KeyPath::Index(1),
                ],
            },
        ]
    );

    paths.clear();
    raw_jsonb
        .visit_scalar_key_paths(true, |path| {
            paths.push(KeyPaths {
                paths: path.to_vec(),
            });
            Ok(())
        })
        .unwrap();
    assert_eq!(
        paths,
        vec![
            KeyPaths {
                paths: vec![KeyPath::Name(Cow::Borrowed("empty"))],
            },
            KeyPaths {
                paths: vec![
                    KeyPath::Name(Cow::Borrowed("user")),
                    KeyPath::Name(Cow::Borrowed("name")),
                ],
            },
            KeyPaths {
                paths: vec![
                    KeyPath::Name(Cow::Borrowed("user")),
                    KeyPath::Name(Cow::Borrowed("scores")),
                ],
            },
        ]
    );
}

#[test]
fn test_visit_scalar_key_values() {
    let json = r#"{"empty":{},"null":null,"user":{"name":"Alice","scores":[85,92]}}"#;
    let jsonb = json.parse::<OwnedJsonb>().unwrap();
    let raw_jsonb = jsonb.as_raw();

    let mut values = Vec::new();
    raw_jsonb
        .visit_scalar_key_values(false, |paths, value| {
            values.push((
                KeyPaths {
                    paths: paths.to_vec(),
                },
                value,
            ));
            Ok(())
        })
        .unwrap();
    assert_eq!(values, raw_jsonb.extract_scalar_key_values(false).unwrap());
    assert_eq!(values.len(), 5);
    assert!(values.iter().any(|(paths, value)| {
        paths.paths == vec![KeyPath::Name(Cow::Borrowed("null"))] && *value == Value::Null
    }));
    assert!(values.iter().any(|(paths, value)| {
        paths.paths
            == vec![
                KeyPath::Name(Cow::Borrowed("user")),
                KeyPath::Name(Cow::Borrowed("scores")),
                KeyPath::Index(1),
            ]
            && *value == Value::Number(Number::UInt64(92))
    }));

    values.clear();
    raw_jsonb
        .visit_scalar_key_values(true, |paths, value| {
            values.push((
                KeyPaths {
                    paths: paths.to_vec(),
                },
                value,
            ));
            Ok(())
        })
        .unwrap();
    assert_eq!(values, raw_jsonb.extract_scalar_key_values(true).unwrap());
    assert_eq!(values.len(), 4);
    assert!(values.iter().any(|(paths, value)| {
        paths.paths
            == vec![
                KeyPath::Name(Cow::Borrowed("user")),
                KeyPath::Name(Cow::Borrowed("scores")),
            ]
            && *value
                == Value::Array(vec![
                    Value::Number(Number::UInt64(85)),
                    Value::Number(Number::UInt64(92)),
                ])
    }));

    let root = "42".parse::<OwnedJsonb>().unwrap();
    let mut visited = false;
    root.as_raw()
        .visit_scalar_key_values(false, |_, _| {
            visited = true;
            Ok(())
        })
        .unwrap();
    assert!(!visited);

    let error = raw_jsonb.visit_scalar_key_values(false, |_, _| Err(Error::InvalidJson));
    assert_eq!(error, Err(Error::InvalidJson));
}

#[test]
fn test_extract_scalar_key_values() {
    // Test case 1: Simple object with scalar values
    let json = r#"{"name": "John", "age": 30, "active": true}"#;
    let jsonb = json.parse::<OwnedJsonb>().unwrap();
    let raw_jsonb = jsonb.as_raw();

    let result = raw_jsonb.extract_scalar_key_values(false).unwrap();
    assert_eq!(result.len(), 3);

    let expected = vec![
        (
            KeyPaths {
                paths: vec![KeyPath::Name(Cow::Borrowed("active"))],
            },
            Value::Bool(true),
        ),
        (
            KeyPaths {
                paths: vec![KeyPath::Name(Cow::Borrowed("age"))],
            },
            Value::Number(Number::UInt64(30)),
        ),
        (
            KeyPaths {
                paths: vec![KeyPath::Name(Cow::Borrowed("name"))],
            },
            Value::String(Cow::Borrowed("John")),
        ),
    ];
    for ((key_paths, value), (expected_key_paths, expected_value)) in
        result.into_iter().zip(expected)
    {
        assert_eq!(key_paths, expected_key_paths);
        assert_eq!(value, expected_value);
    }

    // Test case 2: Nested object with array
    let json = r#"{"user": {"name": "Alice", "scores": [85, 92, 78]}}"#;
    let jsonb = json.parse::<OwnedJsonb>().unwrap();
    let raw_jsonb = jsonb.as_raw();

    let result = raw_jsonb.extract_scalar_key_values(false).unwrap();
    assert_eq!(result.len(), 4);

    let expected = vec![
        (
            KeyPaths {
                paths: vec![
                    KeyPath::Name(Cow::Borrowed("user")),
                    KeyPath::Name(Cow::Borrowed("name")),
                ],
            },
            Value::String(Cow::Borrowed("Alice")),
        ),
        (
            KeyPaths {
                paths: vec![
                    KeyPath::Name(Cow::Borrowed("user")),
                    KeyPath::Name(Cow::Borrowed("scores")),
                    KeyPath::Index(0),
                ],
            },
            Value::Number(Number::UInt64(85)),
        ),
        (
            KeyPaths {
                paths: vec![
                    KeyPath::Name(Cow::Borrowed("user")),
                    KeyPath::Name(Cow::Borrowed("scores")),
                    KeyPath::Index(1),
                ],
            },
            Value::Number(Number::UInt64(92)),
        ),
        (
            KeyPaths {
                paths: vec![
                    KeyPath::Name(Cow::Borrowed("user")),
                    KeyPath::Name(Cow::Borrowed("scores")),
                    KeyPath::Index(2),
                ],
            },
            Value::Number(Number::UInt64(78)),
        ),
    ];
    for ((key_paths, value), (expected_key_paths, expected_value)) in
        result.into_iter().zip(expected)
    {
        assert_eq!(key_paths, expected_key_paths);
        assert_eq!(value, expected_value);
    }

    // Test case 3: Complex nested structure
    let json = r#"{"k1": [{"k2": "v2"}, {"k3": "v3"}]}"#;
    let jsonb = json.parse::<OwnedJsonb>().unwrap();
    let raw_jsonb = jsonb.as_raw();

    let result = raw_jsonb.extract_scalar_key_values(false).unwrap();
    assert_eq!(result.len(), 2);

    let expected = vec![
        (
            KeyPaths {
                paths: vec![
                    KeyPath::Name(Cow::Borrowed("k1")),
                    KeyPath::Index(0),
                    KeyPath::Name(Cow::Borrowed("k2")),
                ],
            },
            Value::String(Cow::Borrowed("v2")),
        ),
        (
            KeyPaths {
                paths: vec![
                    KeyPath::Name(Cow::Borrowed("k1")),
                    KeyPath::Index(1),
                    KeyPath::Name(Cow::Borrowed("k3")),
                ],
            },
            Value::String(Cow::Borrowed("v3")),
        ),
    ];
    for ((key_paths, value), (expected_key_paths, expected_value)) in
        result.into_iter().zip(expected)
    {
        assert_eq!(key_paths, expected_key_paths);
        assert_eq!(value, expected_value);
    }

    // Test case 4: Ignore array values
    let json = r#"{"user": {"name": "Alice", "scores": [85, 92, 78]}}"#;
    let jsonb = json.parse::<OwnedJsonb>().unwrap();
    let raw_jsonb = jsonb.as_raw();

    let result = raw_jsonb.extract_scalar_key_values(true).unwrap();
    assert_eq!(result.len(), 2);

    let expected = vec![
        (
            KeyPaths {
                paths: vec![
                    KeyPath::Name(Cow::Borrowed("user")),
                    KeyPath::Name(Cow::Borrowed("name")),
                ],
            },
            Value::String(Cow::Borrowed("Alice")),
        ),
        (
            KeyPaths {
                paths: vec![
                    KeyPath::Name(Cow::Borrowed("user")),
                    KeyPath::Name(Cow::Borrowed("scores")),
                ],
            },
            Value::Array(vec![
                Value::Number(Number::UInt64(85)),
                Value::Number(Number::UInt64(92)),
                Value::Number(Number::UInt64(78)),
            ]),
        ),
    ];
    for ((key_paths, value), (expected_key_paths, expected_value)) in
        result.into_iter().zip(expected)
    {
        assert_eq!(key_paths, expected_key_paths);
        assert_eq!(value, expected_value);
    }

    // Test case 5: Empty object/array are returned as values
    let json = r#"{"empty_obj": {}, "empty_arr": [], "nested": {"empty": []}}"#;
    let jsonb = json.parse::<OwnedJsonb>().unwrap();
    let raw_jsonb = jsonb.as_raw();

    let result = raw_jsonb.extract_scalar_key_values(false).unwrap();
    assert_eq!(result.len(), 3);

    let expected = vec![
        (
            KeyPaths {
                paths: vec![KeyPath::Name(Cow::Borrowed("empty_arr"))],
            },
            Value::Array(vec![]),
        ),
        (
            KeyPaths {
                paths: vec![KeyPath::Name(Cow::Borrowed("empty_obj"))],
            },
            Value::Object(BTreeMap::new()),
        ),
        (
            KeyPaths {
                paths: vec![
                    KeyPath::Name(Cow::Borrowed("nested")),
                    KeyPath::Name(Cow::Borrowed("empty")),
                ],
            },
            Value::Array(vec![]),
        ),
    ];
    for ((key_paths, value), (expected_key_paths, expected_value)) in
        result.into_iter().zip(expected)
    {
        assert_eq!(key_paths, expected_key_paths);
        assert_eq!(value, expected_value);
    }
}

#[test]
fn test_jsonb_item_type() {
    // Test null value
    let json = "null";
    let jsonb = json.parse::<OwnedJsonb>().unwrap();
    let raw_jsonb = jsonb.as_raw();
    let item_type = raw_jsonb.jsonb_item_type().unwrap();
    assert!(matches!(item_type, JsonbItemType::Null));

    // Test boolean values
    let json = "true";
    let jsonb = json.parse::<OwnedJsonb>().unwrap();
    let raw_jsonb = jsonb.as_raw();
    let item_type = raw_jsonb.jsonb_item_type().unwrap();
    assert!(matches!(item_type, JsonbItemType::Boolean));

    let json = "false";
    let jsonb = json.parse::<OwnedJsonb>().unwrap();
    let raw_jsonb = jsonb.as_raw();
    let item_type = raw_jsonb.jsonb_item_type().unwrap();
    assert!(matches!(item_type, JsonbItemType::Boolean));

    // Test number value
    let json = "123.45";
    let jsonb = json.parse::<OwnedJsonb>().unwrap();
    let raw_jsonb = jsonb.as_raw();
    let item_type = raw_jsonb.jsonb_item_type().unwrap();
    assert!(matches!(item_type, JsonbItemType::Number));

    // Test string value
    let json = r#""hello world""#;
    let jsonb = json.parse::<OwnedJsonb>().unwrap();
    let raw_jsonb = jsonb.as_raw();
    let item_type = raw_jsonb.jsonb_item_type().unwrap();
    assert!(matches!(item_type, JsonbItemType::String));

    // Test array value
    let json = "[1, 2, 3, 4, 5]";
    let jsonb = json.parse::<OwnedJsonb>().unwrap();
    let raw_jsonb = jsonb.as_raw();
    let item_type = raw_jsonb.jsonb_item_type().unwrap();
    assert!(matches!(item_type, JsonbItemType::Array(5)));

    // Test empty array
    let json = "[]";
    let jsonb = json.parse::<OwnedJsonb>().unwrap();
    let raw_jsonb = jsonb.as_raw();
    let item_type = raw_jsonb.jsonb_item_type().unwrap();
    assert!(matches!(item_type, JsonbItemType::Array(0)));

    // Test object value
    let json = r#"{"name": "Alice", "age": 30, "active": true}"#;
    let jsonb = json.parse::<OwnedJsonb>().unwrap();
    let raw_jsonb = jsonb.as_raw();
    let item_type = raw_jsonb.jsonb_item_type().unwrap();
    assert!(matches!(item_type, JsonbItemType::Object(3)));

    // Test empty object
    let json = "{}";
    let jsonb = json.parse::<OwnedJsonb>().unwrap();
    let raw_jsonb = jsonb.as_raw();
    let item_type = raw_jsonb.jsonb_item_type().unwrap();
    assert!(matches!(item_type, JsonbItemType::Object(0)));
}

#[test]
fn test_to_value() {
    // Test null value
    let json = "null";
    let jsonb = json.parse::<OwnedJsonb>().unwrap();
    let raw_jsonb = jsonb.as_raw();
    let value = raw_jsonb.to_value().unwrap();
    assert!(value.is_null());

    // Test boolean values
    let json = "true";
    let jsonb = json.parse::<OwnedJsonb>().unwrap();
    let raw_jsonb = jsonb.as_raw();
    let value = raw_jsonb.to_value().unwrap();
    assert!(value.as_bool().unwrap());

    let json = "false";
    let jsonb = json.parse::<OwnedJsonb>().unwrap();
    let raw_jsonb = jsonb.as_raw();
    let value = raw_jsonb.to_value().unwrap();
    assert!(!value.as_bool().unwrap());

    // Test number values
    let json = "123";
    let jsonb = json.parse::<OwnedJsonb>().unwrap();
    let raw_jsonb = jsonb.as_raw();
    let value = raw_jsonb.to_value().unwrap();
    assert_eq!(value.as_i64().unwrap(), 123);

    let json = "123.45";
    let jsonb = json.parse::<OwnedJsonb>().unwrap();
    let raw_jsonb = jsonb.as_raw();
    let value = raw_jsonb.to_value().unwrap();
    assert_eq!(value.as_f64().unwrap(), 123.45);

    // Test string value
    let json = r#""hello world""#;
    let jsonb = json.parse::<OwnedJsonb>().unwrap();
    let raw_jsonb = jsonb.as_raw();
    let value = raw_jsonb.to_value().unwrap();
    assert_eq!(value.as_str().unwrap(), "hello world");

    // Test array value
    let json = "[1, 2, 3, 4, 5]";
    let jsonb = json.parse::<OwnedJsonb>().unwrap();
    let raw_jsonb = jsonb.as_raw();
    let value = raw_jsonb.to_value().unwrap();
    if let Value::Array(arr) = value {
        assert_eq!(arr.len(), 5);
        for (i, val) in arr.iter().enumerate() {
            assert_eq!(val.as_i64().unwrap(), (i + 1) as i64);
        }
    } else {
        panic!("Expected array value");
    }

    // Test simple object
    let json = r#"{"name": "Alice", "age": 30, "active": true}"#;
    let jsonb = json.parse::<OwnedJsonb>().unwrap();
    let raw_jsonb = jsonb.as_raw();
    let value = raw_jsonb.to_value().unwrap();
    if let Value::Object(obj) = value {
        assert_eq!(obj.len(), 3);
        assert_eq!(obj.get("name").unwrap().as_str().unwrap(), "Alice");
        assert_eq!(obj.get("age").unwrap().as_i64().unwrap(), 30);
        assert!(obj.get("active").unwrap().as_bool().unwrap());
    } else {
        panic!("Expected object value");
    }

    // Test nested object with array
    let json = r#"{"user": {"name": "Bob", "scores": [85, 90, 95]}}"#;
    let jsonb = json.parse::<OwnedJsonb>().unwrap();
    let raw_jsonb = jsonb.as_raw();
    let value = raw_jsonb.to_value().unwrap();
    if let Value::Object(obj) = value {
        assert_eq!(obj.len(), 1);
        if let Value::Object(user) = obj.get("user").unwrap() {
            assert_eq!(user.len(), 2);
            assert_eq!(user.get("name").unwrap().as_str().unwrap(), "Bob");

            if let Value::Array(scores) = user.get("scores").unwrap() {
                assert_eq!(scores.len(), 3);
                assert_eq!(scores[0].as_i64().unwrap(), 85);
                assert_eq!(scores[1].as_i64().unwrap(), 90);
                assert_eq!(scores[2].as_i64().unwrap(), 95);
            } else {
                panic!("Expected array for scores");
            }
        } else {
            panic!("Expected object for user");
        }
    } else {
        panic!("Expected object value");
    }
}

fn init_object<'a>(entries: Vec<(&str, Value<'a>)>) -> Value<'a> {
    let mut map = BTreeMap::new();
    for (key, val) in entries {
        map.insert(key.to_string(), val);
    }
    Value::Object(map)
}
