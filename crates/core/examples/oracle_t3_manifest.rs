//! Generates a private T3 real-world oracle manifest from a corpus directory.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "Makefile oracle targets are synchronous one-shot local artifact writers"
)]

use std::{
    env,
    fmt::Write,
    fs,
    num::NonZeroUsize,
    path::{Path, PathBuf},
};

use pdfv_core::{BoundedText, ConfigError, CorpusPath, PdfvError, Result};

const CORPUS_ROOT_ENV: &str = "PDFV_ORACLE_T3_CORPUS_ROOT";
const OUTPUT_ENV: &str = "PDFV_ORACLE_T3_MANIFEST";
const MAX_ROWS_ENV: &str = "PDFV_ORACLE_T3_MAX_ROWS";
const DEFAULT_OUTPUT: &str = "target/parity/oracle-t3-real-world.yml";
const DEFAULT_MAX_ROWS: usize = 100_000;

fn main() -> Result<()> {
    let corpus_root = required_path_env(CORPUS_ROOT_ENV)?;
    let canonical_root = canonical_dir(&corpus_root)?;
    let output_path =
        env::var_os(OUTPUT_ENV).map_or_else(|| PathBuf::from(DEFAULT_OUTPUT), PathBuf::from);
    let max_rows = env_max_rows()?;

    let mut pdfs = Vec::new();
    collect_pdfs(&canonical_root, &canonical_root, max_rows, &mut pdfs)?;
    if pdfs.is_empty() {
        return Err(invalid_value(
            CORPUS_ROOT_ENV,
            "T3 corpus contains no PDF files",
        ));
    }
    pdfs.sort();

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).map_err(|source| PdfvError::Io {
            path: Some(parent.to_path_buf()),
            source,
        })?;
    }
    fs::write(&output_path, render_manifest(&pdfs)?).map_err(|source| PdfvError::Io {
        path: Some(output_path),
        source,
    })?;
    Ok(())
}

fn required_path_env(name: &'static str) -> Result<PathBuf> {
    env::var_os(name)
        .map(PathBuf::from)
        .ok_or_else(|| invalid_value(name, format!("{name} must be set")))
}

fn canonical_dir(path: &Path) -> Result<PathBuf> {
    let canonical = path.canonicalize().map_err(|source| PdfvError::Io {
        path: Some(path.to_path_buf()),
        source,
    })?;
    if !canonical.is_dir() {
        return Err(invalid_value(
            CORPUS_ROOT_ENV,
            "T3 corpus root must be a directory",
        ));
    }
    Ok(canonical)
}

fn env_max_rows() -> Result<NonZeroUsize> {
    match env::var(MAX_ROWS_ENV) {
        Ok(value) => parse_non_zero_usize(MAX_ROWS_ENV, &value),
        Err(env::VarError::NotPresent) => {
            Ok(NonZeroUsize::new(DEFAULT_MAX_ROWS).unwrap_or(NonZeroUsize::MIN))
        }
        Err(source) => Err(invalid_value(
            MAX_ROWS_ENV,
            format!("invalid environment value: {source}"),
        )),
    }
}

fn parse_non_zero_usize(field: &'static str, value: &str) -> Result<NonZeroUsize> {
    let parsed = value
        .parse::<usize>()
        .map_err(|source| invalid_value(field, format!("invalid integer value: {source}")))?;
    NonZeroUsize::new(parsed).ok_or_else(|| invalid_value(field, "value must be at least 1"))
}

fn collect_pdfs(
    root: &Path,
    current: &Path,
    max_rows: NonZeroUsize,
    pdfs: &mut Vec<ManifestPdf>,
) -> Result<()> {
    let mut entries = fs::read_dir(current)
        .map_err(|source| PdfvError::Io {
            path: Some(current.to_path_buf()),
            source,
        })?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|source| PdfvError::Io {
            path: Some(current.to_path_buf()),
            source,
        })?;
    entries.sort_by_key(std::fs::DirEntry::path);

    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|source| PdfvError::Io {
            path: Some(path.clone()),
            source,
        })?;
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            collect_pdfs(root, &path, max_rows, pdfs)?;
            continue;
        }
        if !file_type.is_file() || !has_pdf_extension(&path) {
            continue;
        }
        if pdfs.len() >= max_rows.get() {
            return Err(invalid_value(MAX_ROWS_ENV, "T3 corpus exceeds row limit"));
        }
        let canonical = path.canonicalize().map_err(|source| PdfvError::Io {
            path: Some(path.clone()),
            source,
        })?;
        if !canonical.starts_with(root) {
            return Err(invalid_value(
                CORPUS_ROOT_ENV,
                "PDF path escaped the canonical corpus root",
            ));
        }
        let relative = relative_path(root, &canonical)?;
        let size_bytes = metadata.len();
        pdfs.push(ManifestPdf {
            relative,
            size_bucket: size_bucket(size_bytes),
        });
    }
    Ok(())
}

fn has_pdf_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
}

fn relative_path(root: &Path, path: &Path) -> Result<String> {
    let relative = path.strip_prefix(root).map_err(|source| {
        invalid_value(
            CORPUS_ROOT_ENV,
            format!("could not compute relative corpus path: {source}"),
        )
    })?;
    let text = relative
        .components()
        .map(|component| {
            component
                .as_os_str()
                .to_str()
                .ok_or_else(|| invalid_value(CORPUS_ROOT_ENV, "corpus path is not valid UTF-8"))
        })
        .collect::<Result<Vec<_>>>()?
        .join("/");
    CorpusPath::new(text.clone())?;
    Ok(text)
}

fn size_bucket(size_bytes: u64) -> &'static str {
    match size_bytes {
        0..=10_239 => "tiny",
        10_240..=1_048_575 => "small",
        1_048_576..=26_214_399 => "medium",
        26_214_400..=262_143_999 => "large",
        _ => "huge",
    }
}

fn render_manifest(pdfs: &[ManifestPdf]) -> Result<String> {
    let mut output =
        String::from("schemaVersion: phase22-oracle-manifest-v1\ncorpusRoot: .\nrows:\n");
    for (index, pdf) in pdfs.iter().enumerate() {
        let row_number = index.saturating_add(1);
        write!(
            output,
            "  - id: t3-{row_number:06}\n    path: {}\n    tier: t3RealWorld\n    \
             profilePolicy:\n      kind: autoDefaultPdfa1b\n    expectedFeatures:\n      - \
             parser\n      - profile\n    sizeBucket: {}\n    encryption: unknown\n    tagging: \
             unknown\n    license: private\n",
            yaml_string(&pdf.relative)?,
            pdf.size_bucket,
        )
        .map_err(|source| {
            invalid_value("manifest", format!("could not render manifest: {source}"))
        })?;
    }
    Ok(output)
}

fn yaml_string(value: &str) -> Result<String> {
    let bounded = BoundedText::new(value, 1024)?;
    let mut output = String::with_capacity(bounded.as_str().len().saturating_add(2));
    output.push('"');
    for character in bounded.as_str().chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character.is_control() => {
                return Err(invalid_value(
                    CORPUS_ROOT_ENV,
                    "corpus path contains a control character",
                ));
            }
            character => output.push(character),
        }
    }
    output.push('"');
    Ok(output)
}

fn invalid_value(field: &'static str, reason: impl Into<String>) -> PdfvError {
    match BoundedText::new(reason, 1024) {
        Ok(reason) => ConfigError::InvalidValue { field, reason }.into(),
        Err(error) => error.into(),
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ManifestPdf {
    relative: String,
    size_bucket: &'static str,
}

#[cfg(test)]
mod tests {
    use std::{fs, num::NonZeroUsize};

    use super::{
        ManifestPdf, collect_pdfs, has_pdf_extension, parse_non_zero_usize, render_manifest,
        size_bucket, yaml_string,
    };

    #[test]
    fn test_should_bucket_t3_pdf_sizes() {
        assert_eq!(size_bucket(0), "tiny");
        assert_eq!(size_bucket(10_240), "small");
        assert_eq!(size_bucket(1_048_576), "medium");
        assert_eq!(size_bucket(26_214_400), "large");
        assert_eq!(size_bucket(262_144_000), "huge");
    }

    #[test]
    fn test_should_detect_pdf_extension_case_insensitively() {
        assert!(has_pdf_extension(std::path::Path::new("a.PDF")));
        assert!(!has_pdf_extension(std::path::Path::new("a.txt")));
    }

    #[test]
    fn test_should_reject_zero_max_rows() {
        let result = parse_non_zero_usize("PDFV_ORACLE_T3_MAX_ROWS", "0");

        assert!(result.is_err());
    }

    #[test]
    fn test_should_escape_yaml_string() -> pdfv_core::Result<()> {
        let escaped = yaml_string("dir/a \"quoted\" file.pdf")?;

        assert_eq!(escaped, "\"dir/a \\\"quoted\\\" file.pdf\"");
        Ok(())
    }

    #[test]
    fn test_should_render_private_t3_manifest() -> pdfv_core::Result<()> {
        let manifest = render_manifest(&[ManifestPdf {
            relative: String::from("a.pdf"),
            size_bucket: "tiny",
        }])?;

        assert!(manifest.contains("tier: t3RealWorld"));
        assert!(manifest.contains("license: private"));
        assert!(manifest.contains("kind: autoDefaultPdfa1b"));
        Ok(())
    }

    #[test]
    fn test_should_collect_pdfs_and_skip_symlinks() -> pdfv_core::Result<()> {
        let temp = tempfile::tempdir()
            .map_err(|source| pdfv_core::PdfvError::Io { path: None, source })?;
        let root = temp
            .path()
            .canonicalize()
            .map_err(|source| pdfv_core::PdfvError::Io {
                path: Some(temp.path().to_path_buf()),
                source,
            })?;
        fs::write(root.join("a.pdf"), b"%PDF").map_err(|source| pdfv_core::PdfvError::Io {
            path: Some(root.join("a.pdf")),
            source,
        })?;
        fs::write(root.join("b.txt"), b"text").map_err(|source| pdfv_core::PdfvError::Io {
            path: Some(root.join("b.txt")),
            source,
        })?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("a.pdf"), root.join("linked.pdf")).map_err(
            |source| pdfv_core::PdfvError::Io {
                path: Some(root.join("linked.pdf")),
                source,
            },
        )?;

        let mut pdfs = Vec::new();
        collect_pdfs(&root, &root, NonZeroUsize::MIN, &mut pdfs)?;

        assert_eq!(pdfs.len(), 1);
        assert_eq!(pdfs[0].relative, "a.pdf");
        Ok(())
    }
}
