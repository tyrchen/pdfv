#![no_main]

use std::io::Cursor;

use libfuzzer_sys::fuzz_target;
use pdfv_core::{FeatureSelection, InputName, ResourceLimits, ValidationOptions, Validator};

fuzz_target!(|data: &[u8]| {
    let mut pdf = br"%PDF-1.7
1 0 obj
<< /Type /Catalog /Pages 2 0 R /StructTreeRoot 4 0 R /MarkInfo << /Marked true >> >>
endobj
2 0 obj
<< /Type /Pages /Kids [3 0 R] /Count 1 >>
endobj
3 0 obj
<< /Type /Page /Parent 2 0 R /StructParents 0 /Contents 5 0 R >>
endobj
4 0 obj
<< /Type /StructTreeRoot /K "
        .to_vec();
    pdf.extend(data);
    pdf.extend(
        br" /ParentTree << /Nums [0 []] >> >>
endobj
5 0 obj
<< /Length 0 >>
stream
endstream
endobj
trailer
<< /Root 1 0 R >>
%%EOF
",
    );
    let mut limits = ResourceLimits::default();
    limits.max_structure_nodes = 1024;
    limits.max_structure_depth = 32;
    limits.max_parent_tree_entries = 1024;
    limits.max_objects = 4096;
    let options = ValidationOptions::builder()
        .resource_limits(limits)
        .feature_selection(FeatureSelection::All)
        .build();
    if let Ok(validator) = Validator::new(options) {
        let _ = validator.validate_reader(Cursor::new(pdf), InputName::memory());
    }
});
