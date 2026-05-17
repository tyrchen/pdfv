#![no_main]

use std::io::Cursor;

use libfuzzer_sys::fuzz_target;
use pdfv_core::{FeatureSelection, InputName, ResourceLimits, ValidationOptions, Validator};

fuzz_target!(|data: &[u8]| {
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
    pdf.extend(data.len().to_string().as_bytes());
    pdf.extend(
        br" >>
stream
",
    );
    pdf.extend(data);
    pdf.extend(
        br"
endstream
endobj
trailer
<< /Root 1 0 R >>
%%EOF
",
    );
    let mut limits = ResourceLimits::default();
    limits.max_content_stream_ops = 1024;
    limits.max_content_stream_operand_count = 64;
    limits.max_content_stream_operand_bytes = 4096;
    limits.max_inline_image_bytes = 4096;
    let options = ValidationOptions::builder()
        .resource_limits(limits)
        .feature_selection(FeatureSelection::All)
        .build();
    if let Ok(validator) = Validator::new(options) {
        let _ = validator.validate_reader(Cursor::new(pdf), InputName::memory());
    }
});
