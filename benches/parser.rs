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

use std::fs;
use std::io::Read;

use criterion::{
    black_box, criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput,
};

fn parse_jsonb(data: &[u8]) {
    let v: jsonb::OwnedJsonb = jsonb::parse_owned_jsonb(data).unwrap();
    black_box(v);
}

fn parse_jsonb_with_buf(data: &[u8], buf: &mut Vec<u8>) {
    buf.clear();
    jsonb::parse_owned_jsonb_with_buf(data, buf).unwrap();
    black_box(&buf);
}

fn parse_jsonb_standard(data: &[u8]) {
    let v: jsonb::OwnedJsonb = jsonb::parse_owned_jsonb_standard_mode(data).unwrap();
    black_box(v);
}

fn parse_jsonb_direct(data: &[u8]) {
    let v: jsonb::OwnedJsonb = jsonb::parse_owned_jsonb_direct(data).unwrap();
    black_box(v);
}

fn parse_jsonb_standard_direct(data: &[u8]) {
    let v: jsonb::OwnedJsonb = jsonb::parse_owned_jsonb_standard_mode_direct(data).unwrap();
    black_box(v);
}

fn parse_jsonb_standard_with_buf(data: &[u8], buf: &mut Vec<u8>) {
    buf.clear();
    jsonb::parse_owned_jsonb_standard_mode_with_buf(data, buf).unwrap();
    black_box(&buf);
}

fn parse_serde_json(data: &[u8]) {
    let v: serde_json::Value = serde_json::from_slice(data).unwrap();
    black_box(v);
}

fn parse_json_deserializer(data: &[u8]) {
    let v: json_deserializer::Value = json_deserializer::parse(data).unwrap();
    black_box(v);
}

fn parse_simd_json(data: &mut [u8]) {
    let v = simd_json::to_borrowed_value(data).unwrap();
    black_box(v);
}

fn read(file: &str) -> Vec<u8> {
    let mut f = fs::File::open(file).unwrap();
    let mut data = vec![];
    f.read_to_end(&mut data).unwrap();
    data
}

fn compact_object() -> Vec<u8> {
    let mut s = String::from("{");
    for i in 0..96 {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!(
            r#""key_{i}":{{"id":{i},"name":"item-{i}","active":{},"values":[1,2,3,4]}}"#,
            i % 2 == 0
        ));
    }
    s.push('}');
    s.into_bytes()
}

fn pretty_object() -> Vec<u8> {
    let mut s = String::from("{\n");
    for i in 0..96 {
        if i > 0 {
            s.push_str(",\n");
        }
        s.push_str(&format!(
            "  \"key_{i}\": {{\n    \"id\": {i},\n    \"name\": \"item-{i}\",\n    \"active\": {},\n    \"values\": [1, 2, 3, 4]\n  }}",
            i % 2 == 0
        ));
    }
    s.push_str("\n}");
    s.into_bytes()
}

fn large_array() -> Vec<u8> {
    let mut s = String::from("[");
    for i in 0..4096 {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&(i * 17).to_string());
    }
    s.push(']');
    s.into_bytes()
}

fn large_string() -> Vec<u8> {
    let payload = "abcdefghijklmnopqrstuvwxyz0123456789".repeat(1024);
    format!(r#"{{"payload":"{payload}"}}"#).into_bytes()
}

fn escaped_string() -> Vec<u8> {
    let mut payload = String::new();
    for i in 0..1024 {
        payload.push_str(r#"line\n\t\"quoted\"\\slash\u0041"#);
        payload.push_str(&i.to_string());
    }
    format!(r#"{{"payload":"{payload}"}}"#).into_bytes()
}

fn decimal_heavy() -> Vec<u8> {
    let mut s = String::from("[");
    for i in 0..512 {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!("{i}.12345678901234567890123456789012345678"));
    }
    s.push(']');
    s.into_bytes()
}

fn extended_json5_like() -> Vec<u8> {
    let mut s = String::from("{");
    for i in 0..128 {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!(
            "key_{i}: {{hex: 0x{:x}, single: 'value-{i}', plus: +{i}, sparse: [1,,3,]}}",
            i * 31
        ));
    }
    s.push('}');
    s.into_bytes()
}

fn synthetic_standard_cases() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("compact_object", compact_object()),
        ("pretty_object", pretty_object()),
        ("large_array", large_array()),
        ("large_string", large_string()),
        ("escaped_string", escaped_string()),
        ("decimal_heavy", decimal_heavy()),
        ("json5_like", extended_json5_like()),
    ]
}

fn synthetic_extended_cases() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("compact_object", compact_object()),
        ("pretty_object", pretty_object()),
        ("large_array", large_array()),
        ("large_string", large_string()),
        ("escaped_string", escaped_string()),
        ("decimal_heavy", decimal_heavy()),
        ("json5_like", extended_json5_like()),
    ]
}

fn bench_file_inputs(c: &mut Criterion) {
    let paths = fs::read_dir("./data/").unwrap();
    for path in paths {
        let file = format!("{}", path.unwrap().path().display());
        let bytes = read(&file);
        let mut group = c.benchmark_group(format!("parser/file/{file}"));
        group.throughput(Throughput::Bytes(bytes.len() as u64));

        group.bench_function("jsonb_owned", |b| b.iter(|| parse_jsonb(&bytes)));

        group.bench_function("jsonb_owned_with_buf", |b| {
            let mut buf = Vec::with_capacity(bytes.len());
            b.iter(|| parse_jsonb_with_buf(&bytes, &mut buf))
        });

        group.bench_function("jsonb_standard_owned", |b| {
            b.iter(|| parse_jsonb_standard(&bytes))
        });

        group.bench_function("jsonb_direct_owned", |b| {
            b.iter(|| parse_jsonb_direct(&bytes))
        });

        group.bench_function("jsonb_standard_direct_owned", |b| {
            b.iter(|| parse_jsonb_standard_direct(&bytes))
        });

        group.bench_function("jsonb_standard_owned_with_buf", |b| {
            let mut buf = Vec::with_capacity(bytes.len());
            b.iter(|| parse_jsonb_standard_with_buf(&bytes, &mut buf))
        });

        group.bench_function("serde_json", |b| b.iter(|| parse_serde_json(&bytes)));

        group.bench_function("json_deserializer", |b| {
            b.iter(|| parse_json_deserializer(&bytes))
        });

        let bytes = bytes.clone();
        group.bench_function("simd_json", move |b| {
            b.iter_batched(
                || bytes.clone(),
                |mut data| parse_simd_json(&mut data),
                BatchSize::SmallInput,
            )
        });
        group.finish();
    }
}

fn bench_synthetic_standard_inputs(c: &mut Criterion) {
    let mut group = c.benchmark_group("parser/synthetic_standard");
    for (name, bytes) in synthetic_standard_cases() {
        group.throughput(Throughput::Bytes(bytes.len() as u64));

        group.bench_with_input(BenchmarkId::new("jsonb_owned", name), &bytes, |b, data| {
            b.iter(|| parse_jsonb(data))
        });

        group.bench_with_input(
            BenchmarkId::new("jsonb_owned_with_buf", name),
            &bytes,
            |b, data| {
                let mut buf = Vec::with_capacity(data.len());
                b.iter(|| parse_jsonb_with_buf(data, &mut buf))
            },
        );

        group.bench_with_input(
            BenchmarkId::new("jsonb_standard_owned", name),
            &bytes,
            |b, data| b.iter(|| parse_jsonb_standard(data)),
        );

        group.bench_with_input(
            BenchmarkId::new("jsonb_direct_owned", name),
            &bytes,
            |b, data| b.iter(|| parse_jsonb_direct(data)),
        );

        group.bench_with_input(
            BenchmarkId::new("jsonb_standard_direct_owned", name),
            &bytes,
            |b, data| b.iter(|| parse_jsonb_standard_direct(data)),
        );

        group.bench_with_input(
            BenchmarkId::new("jsonb_standard_owned_with_buf", name),
            &bytes,
            |b, data| {
                let mut buf = Vec::with_capacity(data.len());
                b.iter(|| parse_jsonb_standard_with_buf(data, &mut buf))
            },
        );

        group.bench_with_input(BenchmarkId::new("serde_json", name), &bytes, |b, data| {
            b.iter(|| parse_serde_json(data))
        });

        group.bench_with_input(
            BenchmarkId::new("json_deserializer", name),
            &bytes,
            |b, data| b.iter(|| parse_json_deserializer(data)),
        );

        group.bench_with_input(BenchmarkId::new("simd_json", name), &bytes, |b, data| {
            b.iter_batched(
                || data.clone(),
                |mut data| parse_simd_json(&mut data),
                BatchSize::SmallInput,
            )
        });
    }
    group.finish();
}

fn bench_synthetic_extended_inputs(c: &mut Criterion) {
    let mut group = c.benchmark_group("parser/synthetic_extended");
    for (name, bytes) in synthetic_extended_cases() {
        group.throughput(Throughput::Bytes(bytes.len() as u64));

        group.bench_with_input(BenchmarkId::new("jsonb_owned", name), &bytes, |b, data| {
            b.iter(|| parse_jsonb(data))
        });

        group.bench_with_input(
            BenchmarkId::new("jsonb_direct_owned", name),
            &bytes,
            |b, data| b.iter(|| parse_jsonb_direct(data)),
        );

        group.bench_with_input(
            BenchmarkId::new("jsonb_owned_with_buf", name),
            &bytes,
            |b, data| {
                let mut buf = Vec::with_capacity(data.len());
                b.iter(|| parse_jsonb_with_buf(data, &mut buf))
            },
        );
    }
    group.finish();
}

fn add_benchmark(c: &mut Criterion) {
    bench_file_inputs(c);
    bench_synthetic_standard_inputs(c);
    bench_synthetic_extended_inputs(c);
}

criterion_group!(benches, add_benchmark);
criterion_main!(benches);
