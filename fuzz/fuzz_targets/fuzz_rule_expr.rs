#![no_main]

use libfuzzer_sys::fuzz_target;
use pdfv_core::RuleExpr;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = serde_json::from_str::<RuleExpr>(text);
    }
});
