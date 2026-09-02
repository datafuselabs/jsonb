// Copyright 2024 Datafuse Labs.
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

use std::{fs, io::Read};

use criterion::{criterion_group, criterion_main, Criterion};
use jsonb::{from_slice, Value};

fn read(file: &str) -> Vec<u8> {
    let mut f = fs::File::open(file).unwrap();
    let mut data = vec![];
    f.read_to_end(&mut data).unwrap();
    data
}

fn strip_nulls_deser(data: &[u8]) {
    let mut buf = Vec::new();
    let mut json = from_slice(data).unwrap();
    strip_value_nulls(&mut json);
    json.write_to_vec(&mut buf);
    assert!(!buf.is_empty());
}

fn strip_value_nulls(val: &mut Value<'_>) {
    match val {
        Value::Array(arr) => {
            for v in arr {
                strip_value_nulls(v);
            }
        }
        Value::Object(ref mut obj) => {
            for v in obj.values_mut() {
                strip_value_nulls(v);
            }
            obj.retain(|_, v| !matches!(v, Value::Null));
        }
        _ => {}
    }
}

fn strip_nulls_fast(data: &[u8]) {
    let raw_jsonb = jsonb::RawJsonb::new(data);
    let result_jsonb = raw_jsonb.strip_nulls().unwrap();
    assert!(!result_jsonb.is_empty());
}

fn add_benchmark(c: &mut Criterion) {
    let paths = fs::read_dir("./data/").unwrap();
    for path in paths {
        let file = format!("{}", path.unwrap().path().display());
        let bytes = read(&file);
        let json = from_slice(&bytes).unwrap().to_vec();

        c.bench_function(&format!("strip_nulls_deser[{file}]"), |b| {
            b.iter(|| strip_nulls_deser(&json));
        });

        c.bench_function(&format!("strip_nulls_fast[{file}]"), |b| {
            b.iter(|| strip_nulls_fast(&json));
        });
    }
}

criterion_group!(benches, add_benchmark);
criterion_main!(benches);
