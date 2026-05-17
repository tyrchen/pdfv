//! Writes generated-profile coverage and unsupported-rule parity metrics.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "Makefile parity targets are synchronous one-shot local artifact writers"
)]

use std::{collections::BTreeMap, env, fs, path::Path};

use pdfv_core::{
    BoundedText, ConfigError, PdfvError, ProfileCoverageParityReport, ReportError, Result,
    profile_coverage_parity_report, unsupported_rules_parity_report,
};

const BASELINE_ENV: &str = "PDFV_PARITY_BASELINE_DIR";
const REVIEW_NOTE_ENV: &str = "PDFV_PARITY_REVIEW_NOTE";
const OUTPUT_DIR: &str = "target/parity";

fn main() -> Result<()> {
    let coverage = profile_coverage_parity_report()?;
    let unsupported_rules = unsupported_rules_parity_report()?;
    ensure_no_unreviewed_decrease(&coverage)?;

    let output_dir = Path::new(OUTPUT_DIR);
    create_output_dir(output_dir)?;
    write_json_file(&output_dir.join("profile-coverage.json"), &coverage)?;
    write_json_file(
        &output_dir.join("unsupported-rules.json"),
        &unsupported_rules,
    )?;
    Ok(())
}

fn ensure_no_unreviewed_decrease(current: &ProfileCoverageParityReport) -> Result<()> {
    let Some(baseline_dir) = env::var_os(BASELINE_ENV) else {
        return Ok(());
    };
    let baseline_path = Path::new(&baseline_dir).join("profile-coverage.json");
    if !baseline_path.exists() {
        return Ok(());
    }
    let baseline = read_json_file(&baseline_path)?;
    let decreases = coverage_decreases(&baseline, current);
    if decreases.is_empty() || has_review_note() {
        return Ok(());
    }
    Err(ConfigError::InvalidValue {
        field: "parityCoverage",
        reason: BoundedText::new(
            format!(
                "coverage decreased without review note; set {REVIEW_NOTE_ENV} or add a \
                 docs/reviews/specs-93 coverage decrease note: {}",
                decreases.join("; ")
            ),
            1024,
        )?,
    }
    .into())
}

fn coverage_decreases(
    baseline: &ProfileCoverageParityReport,
    current: &ProfileCoverageParityReport,
) -> Vec<String> {
    let current_by_flavour = current
        .profiles
        .iter()
        .map(|profile| (profile.flavour.as_str(), profile))
        .collect::<BTreeMap<_, _>>();
    baseline
        .profiles
        .iter()
        .filter_map(|before| {
            let after = current_by_flavour.get(before.flavour.as_str())?;
            let mut fields = Vec::new();
            if after.lowered_rules < before.lowered_rules {
                fields.push(format!(
                    "loweredRules {} -> {}",
                    before.lowered_rules, after.lowered_rules
                ));
            }
            if after.bound_rules < before.bound_rules {
                fields.push(format!(
                    "boundRules {} -> {}",
                    before.bound_rules, after.bound_rules
                ));
            }
            (!fields.is_empty())
                .then(|| format!("{} {}", before.flavour.as_str(), fields.join(",")))
        })
        .collect()
}

fn has_review_note() -> bool {
    env::var_os(REVIEW_NOTE_ENV).is_some_and(|path| Path::new(&path).is_file())
        || file_contains(
            Path::new("specs/93-improvements-review.md"),
            "coverage decrease",
        )
        || review_dir_contains_coverage_decrease(Path::new("docs/reviews"))
}

fn review_dir_contains_coverage_decrease(path: &Path) -> bool {
    let Ok(entries) = fs::read_dir(path) else {
        return false;
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("md"))
        .any(|path| file_contains(&path, "coverage decrease"))
}

fn file_contains(path: &Path, needle: &str) -> bool {
    fs::read_to_string(path).is_ok_and(|content| content.contains(needle))
}

fn read_json_file(path: &Path) -> Result<ProfileCoverageParityReport> {
    let file = fs::File::open(path).map_err(|source| PdfvError::Io {
        path: Some(path.to_path_buf()),
        source,
    })?;
    serde_json::from_reader(file)
        .map_err(ReportError::from)
        .map_err(Into::into)
}

fn create_output_dir(output_dir: &Path) -> Result<()> {
    fs::create_dir_all(output_dir).map_err(|source| PdfvError::Io {
        path: Some(output_dir.to_path_buf()),
        source,
    })
}

fn write_json_file<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    let file = fs::File::create(path).map_err(|source| PdfvError::Io {
        path: Some(path.to_path_buf()),
        source,
    })?;
    serde_json::to_writer_pretty(file, value).map_err(ReportError::from)?;
    Ok(())
}
