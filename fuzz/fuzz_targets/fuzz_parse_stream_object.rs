#![no_main]

use std::io::Cursor;

use libfuzzer_sys::fuzz_target;
use pdfv_core::Parser;

fuzz_target!(|data: &[u8]| {
    let mut pdf = br"%PDF-1.7
1 0 obj
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
%%EOF
",
    );
    let _ = Parser::default().parse(Cursor::new(pdf));
});
