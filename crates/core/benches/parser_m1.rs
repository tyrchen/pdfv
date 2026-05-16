#![allow(
    missing_docs,
    reason = "criterion bench entrypoints are not public API"
)]

use std::io::Cursor;

use criterion::{Criterion, criterion_group, criterion_main};
use pdfv_core::Parser;

const MINIMAL_VALID: &[u8] = include_bytes!("../../../tests/fixtures/minimal-valid.pdf");
const TEN_MIB: usize = 10 * 1024 * 1024;
const ONE_MIB: usize = 1024 * 1024;

fn bench_parse_minimal(c: &mut Criterion) {
    c.bench_function("parse_minimal_valid", |b| {
        b.iter(|| Parser::default().parse(Cursor::new(MINIMAL_VALID)));
    });
}

fn bench_parse_10_mib_simple(c: &mut Criterion) {
    let fixture = simple_pdf_with_payload(TEN_MIB);
    c.bench_function("parse_10_mib_simple_pdf", |b| {
        b.iter(|| Parser::default().parse(Cursor::new(fixture.as_slice())));
    });
}

fn bench_parse_1_mib_malformed(c: &mut Criterion) {
    let fixture = vec![b'x'; ONE_MIB];
    c.bench_function("parse_1_mib_malformed", |b| {
        b.iter(|| Parser::default().parse(Cursor::new(fixture.as_slice())));
    });
}

fn simple_pdf_with_payload(payload_len: usize) -> Vec<u8> {
    let mut pdf = br"%PDF-1.7
1 0 obj
<< /Type /Catalog >>
endobj
2 0 obj
<< /Length "
        .to_vec();
    pdf.extend(payload_len.to_string().as_bytes());
    pdf.extend(
        br" >>
stream
",
    );
    pdf.extend(std::iter::repeat_n(b'a', payload_len));
    pdf.extend(
        br"
endstream
endobj
trailer
<< /Root 1 0 R >>
%%EOF
",
    );
    pdf
}

criterion_group!(
    parser_m1,
    bench_parse_minimal,
    bench_parse_10_mib_simple,
    bench_parse_1_mib_malformed
);
criterion_main!(parser_m1);
