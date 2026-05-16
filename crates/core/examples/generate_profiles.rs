#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::format_push_string,
    clippy::map_unwrap_or,
    reason = "profile generation is an offline developer tool invoked by Makefile"
)]
//! Generate the deterministic built-in profile source catalog.

use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::Command,
};

const PROFILE_DIR: &str =
    "vendors/veraPDF-library/core/src/main/resources/org/verapdf/pdfa/validation";

const PROFILE_FILES: &[(&str, &str, &str)] = &[
    ("verapdf-pdfa-1a", "pdfa-1a", "PDFA-1A.xml"),
    ("verapdf-pdfa-1b", "pdfa-1b", "PDFA-1B.xml"),
    ("verapdf-pdfa-2a", "pdfa-2a", "PDFA-2A.xml"),
    ("verapdf-pdfa-2b", "pdfa-2b", "PDFA-2B.xml"),
    ("verapdf-pdfa-2u", "pdfa-2u", "PDFA-2U.xml"),
    ("verapdf-pdfa-3a", "pdfa-3a", "PDFA-3A.xml"),
    ("verapdf-pdfa-3b", "pdfa-3b", "PDFA-3B.xml"),
    ("verapdf-pdfa-3u", "pdfa-3u", "PDFA-3U.xml"),
    ("verapdf-pdfa-4", "pdfa-4", "PDFA-4.xml"),
    ("verapdf-pdfa-4e", "pdfa-4e", "PDFA-4E.xml"),
    ("verapdf-pdfa-4f", "pdfa-4f", "PDFA-4F.xml"),
    ("verapdf-pdfua-1", "pdfua-1", "PDFUA-1.xml"),
    (
        "verapdf-pdfua-2-iso32005",
        "pdfua-2-iso32005",
        "PDFUA-2-ISO32005.xml",
    ),
    (
        "verapdf-wtpdf-1-0-accessibility",
        "wtpdf-1-0-accessibility",
        "WTPDF-1-0-Accessibility.xml",
    ),
    (
        "verapdf-wtpdf-1-0-reuse",
        "wtpdf-1-0-reuse",
        "WTPDF-1-0-Reuse.xml",
    ),
];

fn main() -> io::Result<()> {
    let output = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("crates/core/src/generated_profiles.rs"));
    let root = workspace_root()?;
    let pin = vendor_pin(&root)?;
    let mut generated = String::new();
    generated.push_str("//! Generated built-in profile source catalog.\n");
    generated.push_str("//!\n");
    generated.push_str("//! Regenerate with `make generate-profiles`.\n\n");
    generated.push_str("/// Source pin for the vendored veraPDF library profiles.\n");
    generated.push_str(&format!(
        "pub const VERA_PDF_LIBRARY_PIN: &str = \"{pin}\";\n\n"
    ));
    generated.push_str("/// One generated profile source entry.\n");
    generated.push_str("#[derive(Clone, Copy, Debug)]\n");
    generated.push_str("pub struct GeneratedProfileSource {\n");
    generated.push_str("    /// Stable CLI/catalog id.\n");
    generated.push_str("    pub id: &'static str,\n");
    generated.push_str("    /// Display flavour accepted by the CLI.\n");
    generated.push_str("    pub display_flavour: &'static str,\n");
    generated.push_str("    /// Vendored XML source path.\n");
    generated.push_str("    pub source_file: &'static str,\n");
    generated.push_str("    /// Vendored XML profile body.\n");
    generated.push_str("    pub xml: &'static str,\n");
    generated.push_str("}\n\n");
    generated.push_str("/// Generated built-in profile sources in deterministic catalog order.\n");
    generated.push_str("#[rustfmt::skip]\n");
    generated.push_str("pub const GENERATED_PROFILE_SOURCES: &[GeneratedProfileSource] = &[\n");
    for (id, display_flavour, file_name) in PROFILE_FILES {
        let source_file = format!("{PROFILE_DIR}/{file_name}");
        let path = root.join(&source_file);
        let xml = fs::read_to_string(&path)?;
        ensure_expected_flavour(&xml, display_flavour, file_name)?;
        generated.push_str("    GeneratedProfileSource {\n");
        generated.push_str(&format!("        id: \"{id}\",\n"));
        generated.push_str(&format!(
            "        display_flavour: \"{display_flavour}\",\n"
        ));
        generated.push_str(&format!("        source_file: \"{source_file}\",\n"));
        generated.push_str(&format!(
            "        xml: include_str!(\"../../../{source_file}\"),\n"
        ));
        generated.push_str("    },\n");
    }
    generated.push_str("];\n");
    fs::write(output, generated)
}

fn workspace_root() -> io::Result<PathBuf> {
    let current = env::current_dir()?;
    if current.join("Cargo.toml").exists() && current.join(PROFILE_DIR).exists() {
        Ok(current)
    } else {
        Ok(current
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or(current))
    }
}

fn vendor_pin(root: &Path) -> io::Result<String> {
    let output = Command::new("git")
        .args(["-C", "vendors/veraPDF-library", "rev-parse", "HEAD"])
        .current_dir(root)
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other("failed to read veraPDF-library pin"));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn ensure_expected_flavour(xml: &str, display_flavour: &str, file_name: &str) -> io::Result<()> {
    let expected = match display_flavour {
        "pdfa-1a" => "PDFA_1_A",
        "pdfa-1b" => "PDFA_1_B",
        "pdfa-2a" => "PDFA_2_A",
        "pdfa-2b" => "PDFA_2_B",
        "pdfa-2u" => "PDFA_2_U",
        "pdfa-3a" => "PDFA_3_A",
        "pdfa-3b" => "PDFA_3_B",
        "pdfa-3u" => "PDFA_3_U",
        "pdfa-4" => "PDFA_4",
        "pdfa-4e" => "PDFA_4_E",
        "pdfa-4f" => "PDFA_4_F",
        "pdfua-1" => "PDFUA_1",
        "pdfua-2-iso32005" => "PDFUA_2",
        "wtpdf-1-0-accessibility" => "WTPDF_1_0_ACCESSIBILITY",
        "wtpdf-1-0-reuse" => "WTPDF_1_0_REUSE",
        _ => "",
    };
    if xml.contains(&format!("flavour=\"{expected}\"")) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unexpected flavour in {file_name}"),
        ))
    }
}
