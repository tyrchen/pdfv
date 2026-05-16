//! Generated built-in profile source catalog.
//!
//! Regenerate with `make generate-profiles`.

/// Source pin for the vendored veraPDF library profiles.
pub const VERA_PDF_LIBRARY_PIN: &str = "acfcc419a5df444e3e8b2a18266d01e249299957";

/// One generated profile source entry.
#[derive(Clone, Copy, Debug)]
pub struct GeneratedProfileSource {
    /// Stable CLI/catalog id.
    pub id: &'static str,
    /// Display flavour accepted by the CLI.
    pub display_flavour: &'static str,
    /// Vendored XML source path.
    pub source_file: &'static str,
    /// Vendored XML profile body.
    pub xml: &'static str,
}

/// Generated built-in profile sources in deterministic catalog order.
#[rustfmt::skip]
pub const GENERATED_PROFILE_SOURCES: &[GeneratedProfileSource] = &[
    GeneratedProfileSource {
        id: "verapdf-pdfa-1a",
        display_flavour: "pdfa-1a",
        source_file: "vendors/veraPDF-library/core/src/main/resources/org/verapdf/pdfa/validation/PDFA-1A.xml",
        xml: include_str!("../../../vendors/veraPDF-library/core/src/main/resources/org/verapdf/pdfa/validation/PDFA-1A.xml"),
    },
    GeneratedProfileSource {
        id: "verapdf-pdfa-1b",
        display_flavour: "pdfa-1b",
        source_file: "vendors/veraPDF-library/core/src/main/resources/org/verapdf/pdfa/validation/PDFA-1B.xml",
        xml: include_str!("../../../vendors/veraPDF-library/core/src/main/resources/org/verapdf/pdfa/validation/PDFA-1B.xml"),
    },
    GeneratedProfileSource {
        id: "verapdf-pdfa-2a",
        display_flavour: "pdfa-2a",
        source_file: "vendors/veraPDF-library/core/src/main/resources/org/verapdf/pdfa/validation/PDFA-2A.xml",
        xml: include_str!("../../../vendors/veraPDF-library/core/src/main/resources/org/verapdf/pdfa/validation/PDFA-2A.xml"),
    },
    GeneratedProfileSource {
        id: "verapdf-pdfa-2b",
        display_flavour: "pdfa-2b",
        source_file: "vendors/veraPDF-library/core/src/main/resources/org/verapdf/pdfa/validation/PDFA-2B.xml",
        xml: include_str!("../../../vendors/veraPDF-library/core/src/main/resources/org/verapdf/pdfa/validation/PDFA-2B.xml"),
    },
    GeneratedProfileSource {
        id: "verapdf-pdfa-2u",
        display_flavour: "pdfa-2u",
        source_file: "vendors/veraPDF-library/core/src/main/resources/org/verapdf/pdfa/validation/PDFA-2U.xml",
        xml: include_str!("../../../vendors/veraPDF-library/core/src/main/resources/org/verapdf/pdfa/validation/PDFA-2U.xml"),
    },
    GeneratedProfileSource {
        id: "verapdf-pdfa-3a",
        display_flavour: "pdfa-3a",
        source_file: "vendors/veraPDF-library/core/src/main/resources/org/verapdf/pdfa/validation/PDFA-3A.xml",
        xml: include_str!("../../../vendors/veraPDF-library/core/src/main/resources/org/verapdf/pdfa/validation/PDFA-3A.xml"),
    },
    GeneratedProfileSource {
        id: "verapdf-pdfa-3b",
        display_flavour: "pdfa-3b",
        source_file: "vendors/veraPDF-library/core/src/main/resources/org/verapdf/pdfa/validation/PDFA-3B.xml",
        xml: include_str!("../../../vendors/veraPDF-library/core/src/main/resources/org/verapdf/pdfa/validation/PDFA-3B.xml"),
    },
    GeneratedProfileSource {
        id: "verapdf-pdfa-3u",
        display_flavour: "pdfa-3u",
        source_file: "vendors/veraPDF-library/core/src/main/resources/org/verapdf/pdfa/validation/PDFA-3U.xml",
        xml: include_str!("../../../vendors/veraPDF-library/core/src/main/resources/org/verapdf/pdfa/validation/PDFA-3U.xml"),
    },
    GeneratedProfileSource {
        id: "verapdf-pdfa-4",
        display_flavour: "pdfa-4",
        source_file: "vendors/veraPDF-library/core/src/main/resources/org/verapdf/pdfa/validation/PDFA-4.xml",
        xml: include_str!("../../../vendors/veraPDF-library/core/src/main/resources/org/verapdf/pdfa/validation/PDFA-4.xml"),
    },
    GeneratedProfileSource {
        id: "verapdf-pdfa-4e",
        display_flavour: "pdfa-4e",
        source_file: "vendors/veraPDF-library/core/src/main/resources/org/verapdf/pdfa/validation/PDFA-4E.xml",
        xml: include_str!("../../../vendors/veraPDF-library/core/src/main/resources/org/verapdf/pdfa/validation/PDFA-4E.xml"),
    },
    GeneratedProfileSource {
        id: "verapdf-pdfa-4f",
        display_flavour: "pdfa-4f",
        source_file: "vendors/veraPDF-library/core/src/main/resources/org/verapdf/pdfa/validation/PDFA-4F.xml",
        xml: include_str!("../../../vendors/veraPDF-library/core/src/main/resources/org/verapdf/pdfa/validation/PDFA-4F.xml"),
    },
    GeneratedProfileSource {
        id: "verapdf-pdfua-1",
        display_flavour: "pdfua-1",
        source_file: "vendors/veraPDF-library/core/src/main/resources/org/verapdf/pdfa/validation/PDFUA-1.xml",
        xml: include_str!("../../../vendors/veraPDF-library/core/src/main/resources/org/verapdf/pdfa/validation/PDFUA-1.xml"),
    },
    GeneratedProfileSource {
        id: "verapdf-pdfua-2-iso32005",
        display_flavour: "pdfua-2-iso32005",
        source_file: "vendors/veraPDF-library/core/src/main/resources/org/verapdf/pdfa/validation/PDFUA-2-ISO32005.xml",
        xml: include_str!("../../../vendors/veraPDF-library/core/src/main/resources/org/verapdf/pdfa/validation/PDFUA-2-ISO32005.xml"),
    },
    GeneratedProfileSource {
        id: "verapdf-wtpdf-1-0-accessibility",
        display_flavour: "wtpdf-1-0-accessibility",
        source_file: "vendors/veraPDF-library/core/src/main/resources/org/verapdf/pdfa/validation/WTPDF-1-0-Accessibility.xml",
        xml: include_str!("../../../vendors/veraPDF-library/core/src/main/resources/org/verapdf/pdfa/validation/WTPDF-1-0-Accessibility.xml"),
    },
    GeneratedProfileSource {
        id: "verapdf-wtpdf-1-0-reuse",
        display_flavour: "wtpdf-1-0-reuse",
        source_file: "vendors/veraPDF-library/core/src/main/resources/org/verapdf/pdfa/validation/WTPDF-1-0-Reuse.xml",
        xml: include_str!("../../../vendors/veraPDF-library/core/src/main/resources/org/verapdf/pdfa/validation/WTPDF-1-0-Reuse.xml"),
    },
];
