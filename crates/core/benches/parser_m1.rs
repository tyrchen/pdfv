#![allow(
    missing_docs,
    reason = "criterion bench entrypoints are not public API"
)]

use std::io::Cursor;

use criterion::{Criterion, criterion_group, criterion_main};
use pdfv_core::{InputName, Parser, ValidationOptions, Validator};

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

fn bench_decode_ascii85_stream(c: &mut Criterion) {
    let fixture = filtered_pdf("ASCII85Decode", b"9jqo~>");
    c.bench_function("decode_ascii85_stream", |b| {
        b.iter(|| Parser::default().parse(Cursor::new(fixture.as_slice())));
    });
}

fn bench_decode_runlength_stream(c: &mut Criterion) {
    let fixture = filtered_pdf("RunLengthDecode", &[2, b'a', b'b', b'c', 254, b'x', 128]);
    c.bench_function("decode_runlength_stream", |b| {
        b.iter(|| Parser::default().parse(Cursor::new(fixture.as_slice())));
    });
}

fn bench_validate_broad_model_graph(c: &mut Criterion) {
    let fixture = broad_model_pdf();
    let Ok(validator) = Validator::new(ValidationOptions::default()) else {
        return;
    };
    c.bench_function("validate_broad_model_graph", |b| {
        b.iter(|| validator.validate_reader(Cursor::new(fixture.as_slice()), InputName::memory()));
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

fn filtered_pdf(filter: &str, encoded: &[u8]) -> Vec<u8> {
    let mut pdf = format!(
        "%PDF-1.7\n1 0 obj\n<< /Type /ObjStm /N 0 /First 0 /Filter /{filter} /Length {} \
         >>\nstream\n",
        encoded.len()
    )
    .into_bytes();
    pdf.extend(encoded);
    pdf.extend(b"\nendstream\nendobj\n%%EOF\n");
    pdf
}

fn broad_model_pdf() -> Vec<u8> {
    br"%PDF-1.7
1 0 obj
<< /Type /Catalog /Pages 2 0 R /AcroForm 7 0 R /StructTreeRoot 8 0 R /OCProperties 9 0 R /Names 10 0 R /Outlines 11 0 R /Perms 12 0 R >>
endobj
2 0 obj
<< /Type /Pages /Kids [3 0 R] /Count 1 >>
endobj
3 0 obj
<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 4 0 R >> /XObject << /Im1 5 0 R >> /ColorSpace << /CS1 13 0 R >> /ExtGState << /GS1 14 0 R >> >> /Annots [6 0 R] /Contents 15 0 R >>
endobj
4 0 obj
<< /Type /Font /Subtype /Type0 /BaseFont /Faux /ToUnicode 16 0 R >>
endobj
5 0 obj
<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length 0 >>
stream
endstream
endobj
6 0 obj
<< /Type /Annot /Subtype /Widget /FT /Sig /A 17 0 R >>
endobj
7 0 obj
<< /Fields [6 0 R] /SigFlags 3 >>
endobj
8 0 obj
<< /Type /StructTreeRoot /K 18 0 R /RoleMap << /H1 /H >> >>
endobj
9 0 obj
<< /OCGs [] /D << >> >>
endobj
10 0 obj
<< /Dests << /Names [] >> >>
endobj
11 0 obj
<< /Type /Outlines /Count 0 >>
endobj
12 0 obj
<< /DocMDP 19 0 R >>
endobj
13 0 obj
<< /N 3 /Alternate /DeviceRGB >>
endobj
14 0 obj
<< /Type /ExtGState /BM /Normal /CA 1 >>
endobj
15 0 obj
<< /Length 3 >>
stream
q Q
endstream
endobj
16 0 obj
<< /Type /CMap /CMapName /Identity-H >>
endobj
17 0 obj
<< /Type /Action /S /URI /URI (https://example.invalid) >>
endobj
18 0 obj
<< /Type /StructElem /S /Document /K [] >>
endobj
19 0 obj
<< /Type /Sig /Filter /Adobe.PPKLite /ByteRange [0 0 0 0] >>
endobj
trailer
<< /Root 1 0 R >>
%%EOF
"
    .to_vec()
}

criterion_group!(
    parser_m1,
    bench_parse_minimal,
    bench_parse_10_mib_simple,
    bench_parse_1_mib_malformed,
    bench_decode_ascii85_stream,
    bench_decode_runlength_stream,
    bench_validate_broad_model_graph
);
criterion_main!(parser_m1);
