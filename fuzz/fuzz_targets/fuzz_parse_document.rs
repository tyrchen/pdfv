#![no_main]

use std::io::Cursor;

use libfuzzer_sys::fuzz_target;
use pdfv_core::Parser;

fuzz_target!(|data: &[u8]| {
    let _ = Parser::default().parse(Cursor::new(data));
});
