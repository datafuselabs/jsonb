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

use std::io::Write;

use goldenfile::Mint;
use jsonb::jsonpath::parse_json_path;

#[test]
fn test_json_path() {
    let mut mint = Mint::new("tests/it/testdata");
    let mut file = mint.new_goldenfile("json_path.txt").unwrap();
    let cases = &[
        r#"$"#,
        r#"$.*"#,
        r#"$[*]"#,
        r#"$.store.book[*].*"#,
        r#"$.store.book[0].price"#,
        r#"$.store.book[last].isbn"#,
        r"$.store.book[last].test_key\uD83D\uDC8E测试",
        r#"$.store.book[0,1, last - 2].price"#,
        r#"$.store.book[0,1 to last-1]"#,
        r#"$."store"."book""#,
        r#"$."st\"ore"."book\uD83D\uDC8E""#,
        r#"$[*].book.price ? (@ == 10)"#,
        r#"$.store.book?(@.price > 10).title"#,
        r#"$.store.book?(@.price < $.expensive).price"#,
        r#"$.store.book?(@.price < 10 && @.category == "fiction")"#,
        r#"$.store.book?(@.price > 10 || @.category == "reference")"#,
        r#"$.store.book?(@.price > 20 && (@.category == "reference" || @.category == "fiction"))"#,
        // compatible with Snowflake style path
        r#"[1][2]"#,
        r#"["k1"]["k2"]"#,
        r#"k1.k2:k3"#,
        r#"k1["k2"][1]"#,
        // predicates
        r#"$ > 1"#,
        r#"$.* == 0"#,
        r#"$[*] > 1"#,
        r#"$.a > $.b"#,
        r#"$.price > 10 || $.category == "reference""#,
        // exists expression
        r#"$.store.book?(exists(@.price?(@ > 20)))"#,
        r#"$.store?(exists(@.book?(exists(@.category?(@ == "fiction")))))"#,
        r#"$.store.book?(starts with "Nigel")"#,
    ];

    for case in cases {
        let json_path = parse_json_path(case.as_bytes()).unwrap();

        writeln!(file, "---------- Input ----------").unwrap();
        writeln!(file, "{}", case).unwrap();
        writeln!(file, "---------- Output ---------").unwrap();
        writeln!(file, "{}", json_path).unwrap();
        writeln!(file, "---------- AST ------------").unwrap();
        writeln!(file, "{:#?}", json_path).unwrap();
        writeln!(file, "\n").unwrap();
    }
}

#[test]
fn test_json_path_error() {
    let cases = &[
        r#"$.["#,
        r#"$X"#,
        r#"$."#,
        r#"$.prop."#,
        r#"$.prop+."#,
        r#"$.."#,
        r#"$.prop.."#,
        r#"$.foo bar"#,
        r#"$[0, 1, 2 4]"#,
        r#"$['1','2',]"#,
        r#"$['1', ,'3']"#,
        r#"$['aaa'}'bbb']"#,
        r#"@ > 10"#,
    ];

    for case in cases {
        let res = parse_json_path(case.as_bytes());
        assert!(res.is_err());
    }
}
