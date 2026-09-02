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

use jsonb::{
    parse_owned_jsonb, parse_owned_jsonb_standard_mode, parse_owned_jsonb_standard_mode_with_buf,
    parse_owned_jsonb_with_buf, parse_value, parse_value_standard_mode, OwnedJsonb, Value,
};

fn decode_owned(jsonb: &OwnedJsonb) -> Value<'_> {
    jsonb.as_raw().to_value().unwrap()
}

fn assert_extended_owned_roundtrip(input: &str) {
    let expected = parse_value(input.as_bytes()).unwrap();
    let owned = parse_owned_jsonb(input.as_bytes()).unwrap();
    assert_eq!(decode_owned(&owned), expected);

    let mut buf = Vec::with_capacity(7);
    parse_owned_jsonb_with_buf(input.as_bytes(), &mut buf).unwrap();
    let with_buf = OwnedJsonb::new(buf);
    assert_eq!(decode_owned(&with_buf), expected);
    assert_eq!(with_buf.as_ref(), owned.as_ref());
}

fn assert_standard_owned_roundtrip(input: &str) {
    let expected = parse_value_standard_mode(input.as_bytes()).unwrap();
    let owned = parse_owned_jsonb_standard_mode(input.as_bytes()).unwrap();
    assert_eq!(decode_owned(&owned), expected);

    let mut buf = Vec::with_capacity(7);
    parse_owned_jsonb_standard_mode_with_buf(input.as_bytes(), &mut buf).unwrap();
    let with_buf = OwnedJsonb::new(buf);
    assert_eq!(decode_owned(&with_buf), expected);
    assert_eq!(with_buf.as_ref(), owned.as_ref());
}

#[test]
fn test_parse_owned_jsonb_extended_roundtrip_cases() {
    let cases = [
        "",
        "  ",
        "null",
        "TRUE",
        "-Infinity",
        "NaN",
        "0x7f",
        "0x1A.B",
        ".25",
        "+42",
        "00123",
        r#""plain ascii string""#,
        r#""escaped\n\t\"\\string\u0041\uD83D\uDE04""#,
        r#"['single quoted', 'quoted value', {z: 1, a: [1,,3,]}]"#,
        r#"{z: 1, a: 2, nested: {beta: true, alpha: null}}"#,
        r#"{
            unquoted: 'value',
            "escaped-key\n": "escaped-value\u0041",
            numbers: [1, 2.5, 123456789012345678901234567890.1234],
            empty_items: [1,,3,]
        }"#,
    ];

    for input in cases {
        assert_extended_owned_roundtrip(input);
    }
}

#[test]
fn test_parse_owned_jsonb_standard_roundtrip_cases() {
    let cases = [
        "null",
        "true",
        "false",
        "0",
        "-42",
        "18446744073709551615",
        "123456789012345678901234567890.1234",
        r#""plain ascii string""#,
        r#""escaped\n\t\"\\string\u0041\uD83D\uDE04""#,
        r#"[1,2,3,{"z":1,"a":[true,false,null]}]"#,
        r#"{"z":1,"a":2,"nested":{"beta":true,"alpha":null}}"#,
        r#"{
            "long_string": "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz",
            "escaped": "line\nbreak\tand unicode \u0041\uD83D\uDE04",
            "numbers": [1, 2.5, 123456789012345678901234567890.1234]
        }"#,
    ];

    for input in cases {
        assert_standard_owned_roundtrip(input);
    }
}

#[test]
fn test_parse_owned_jsonb_duplicate_keys_match_value_parser() {
    let cases = [
        r#"{"a":1,"a":2}"#,
        r#"{a:1,a:2}"#,
        r#"{"nested":{"x":1,"x":2}}"#,
    ];

    for input in cases {
        assert!(parse_value(input.as_bytes()).is_err());
        assert!(parse_owned_jsonb(input.as_bytes()).is_err());
    }
}
