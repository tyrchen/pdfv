#![allow(missing_docs, reason = "criterion bench functions are self-describing")]

use std::io::Cursor;

use criterion::{Criterion, criterion_group, criterion_main};
use pdfv_core::{InputName, ResourceLimits, ValidationOptions, Validator};

fn m6_model_pdf() -> &'static [u8] {
    br"%PDF-1.7
1 0 obj
<< /Type /Catalog /Pages 2 0 R /AcroForm 7 0 R /Names 8 0 R /Outlines 9 0 R >>
endobj
2 0 obj
<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 100 100] /Resources << /Font << /F1 4 0 R >> >> >>
endobj
3 0 obj
<< /Type /Page /Parent 2 0 R /Annots [5 0 R] /Contents 6 0 R >>
endobj
4 0 obj
<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>
endobj
5 0 obj
<< /Type /Annot /Subtype /Widget /FT /Btn /A 10 0 R /AA << /D 11 0 R >> >>
endobj
6 0 obj
<< /Length 3 >>
stream
q Q
endstream
endobj
7 0 obj
<< /Fields [5 0 R] >>
endobj
8 0 obj
<< /Dests << /Names [(home) 12 0 R] >> /EmbeddedFiles << /Names [(f) 13 0 R] >> >>
endobj
9 0 obj
<< /Type /Outlines /First 14 0 R /Last 14 0 R /Count 1 >>
endobj
10 0 obj
<< /Type /Action /S /URI /URI (https://example.invalid) /Next 11 0 R >>
endobj
11 0 obj
<< /Type /Action /S /GoTo /D 12 0 R >>
endobj
12 0 obj
<< /D [3 0 R /Fit] >>
endobj
13 0 obj
<< /Type /Filespec /F (attachment.txt) >>
endobj
14 0 obj
<< /Title (one) /Dest 12 0 R >>
endobj
trailer
<< /Root 1 0 R >>
%%EOF
"
}

fn bench_model_traversal(c: &mut Criterion) {
    let mut limits = ResourceLimits::default();
    limits.max_objects = 512;
    let options = ValidationOptions::builder().resource_limits(limits).build();
    let Ok(validator) = Validator::new(options) else {
        return;
    };
    c.bench_function("m6_model_traversal", |bench| {
        bench.iter(|| {
            if validator
                .validate_reader(Cursor::new(m6_model_pdf()), InputName::memory())
                .is_err()
            {
                std::process::abort();
            }
        });
    });
}

criterion_group!(benches, bench_model_traversal);
criterion_main!(benches);
