#![allow(missing_docs, reason = "criterion bench functions are self-describing")]

use std::io::Cursor;

use criterion::{Criterion, criterion_group, criterion_main};
use pdfv_core::{FeatureSelection, InputName, ResourceLimits, ValidationOptions, Validator};

fn content_stream_pdf(operator_count: usize) -> Vec<u8> {
    let mut stream = Vec::with_capacity(operator_count.saturating_mul(4));
    for _ in 0..operator_count {
        stream.extend_from_slice(b"q Q\n");
    }
    let mut pdf = br"%PDF-1.7
1 0 obj
<< /Type /Catalog /Pages 2 0 R >>
endobj
2 0 obj
<< /Type /Pages /Kids [3 0 R] /Count 1 >>
endobj
3 0 obj
<< /Type /Page /Parent 2 0 R /Resources << >> /Contents 4 0 R >>
endobj
4 0 obj
<< /Length "
        .to_vec();
    pdf.extend(stream.len().to_string().as_bytes());
    pdf.extend(
        br" >>
stream
",
    );
    pdf.extend(stream);
    pdf.extend(
        br"endstream
endobj
trailer
<< /Root 1 0 R >>
%%EOF
",
    );
    pdf
}

fn bench_content_stream_10k_simple_operators(c: &mut Criterion) {
    let fixture = content_stream_pdf(10_000);
    let mut limits = ResourceLimits::default();
    limits.max_content_stream_ops = 20_000;
    let options = ValidationOptions::builder()
        .resource_limits(limits)
        .feature_selection(FeatureSelection::All)
        .build();
    let validator = match Validator::new(options) {
        Ok(validator) => validator,
        Err(error) => {
            eprintln!("failed to construct content stream bench validator: {error}");
            std::process::exit(1);
        }
    };

    c.bench_function("m6_content_stream_10k_simple_operators", |bench| {
        bench.iter(|| {
            if validator
                .validate_reader(Cursor::new(fixture.as_slice()), InputName::memory())
                .is_err()
            {
                std::process::abort();
            }
        });
    });
}

criterion_group!(benches, bench_content_stream_10k_simple_operators);
criterion_main!(benches);
