//! Writes generated-profile coverage and unsupported-rule parity metrics.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "Makefile parity targets are synchronous one-shot local artifact writers"
)]

use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
};

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
        return Err(ConfigError::InvalidValue {
            field: "parityBaseline",
            reason: BoundedText::new(
                format!("baseline artifact {} is missing", baseline_path.display()),
                512,
            )?,
        }
        .into());
    }
    let baseline = read_json_file(&baseline_path)?;
    let decreases = coverage_decreases(&baseline, current);
    if decreases.is_empty() || has_review_note(&decreases) {
        return Ok(());
    }
    Err(ConfigError::InvalidValue {
        field: "parityCoverage",
        reason: BoundedText::new(
            format!(
                "coverage decreased without review note; set {REVIEW_NOTE_ENV} or add a \
                 docs/reviews/specs-93 coverage decrease note: {}",
                decreases
                    .iter()
                    .map(CoverageDecrease::description)
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
            1024,
        )?,
    }
    .into())
}

fn coverage_decreases(
    baseline: &ProfileCoverageParityReport,
    current: &ProfileCoverageParityReport,
) -> Vec<CoverageDecrease> {
    let current_by_flavour = current
        .profiles
        .iter()
        .map(|profile| (profile.flavour.as_str(), profile))
        .collect::<BTreeMap<_, _>>();
    baseline
        .profiles
        .iter()
        .filter_map(|before| {
            let Some(after) = current_by_flavour.get(before.flavour.as_str()) else {
                return Some(CoverageDecrease {
                    flavour: before.flavour.as_str().to_owned(),
                    fields: vec![CoverageDecreaseField::RemovedProfile],
                });
            };
            let mut fields = Vec::new();
            if after.total_rules < before.total_rules {
                fields.push(CoverageDecreaseField::Counter {
                    name: "totalRules",
                    before: before.total_rules,
                    after: after.total_rules,
                });
            }
            if after.lowered_rules < before.lowered_rules {
                fields.push(CoverageDecreaseField::Counter {
                    name: "loweredRules",
                    before: before.lowered_rules,
                    after: after.lowered_rules,
                });
            }
            if after.bound_rules < before.bound_rules {
                fields.push(CoverageDecreaseField::Counter {
                    name: "boundRules",
                    before: before.bound_rules,
                    after: after.bound_rules,
                });
            }
            (!fields.is_empty()).then(|| CoverageDecrease {
                flavour: before.flavour.as_str().to_owned(),
                fields,
            })
        })
        .collect()
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CoverageDecrease {
    flavour: String,
    fields: Vec<CoverageDecreaseField>,
}

impl CoverageDecrease {
    fn description(&self) -> String {
        format!(
            "{} {}",
            self.flavour,
            self.fields
                .iter()
                .map(CoverageDecreaseField::description)
                .collect::<Vec<_>>()
                .join(",")
        )
    }

    fn is_covered_by(&self, note: &str) -> bool {
        note.contains(&self.flavour) && self.fields.iter().all(|field| field.is_covered_by(note))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CoverageDecreaseField {
    Counter {
        name: &'static str,
        before: u64,
        after: u64,
    },
    RemovedProfile,
}

impl CoverageDecreaseField {
    fn description(&self) -> String {
        match self {
            Self::Counter {
                name,
                before,
                after,
            } => format!("{name} {before} -> {after}"),
            Self::RemovedProfile => String::from("profile removed"),
        }
    }

    fn is_covered_by(&self, note: &str) -> bool {
        match self {
            Self::Counter {
                name,
                before,
                after,
            } => {
                note.contains(name)
                    && note.contains(&before.to_string())
                    && note.contains(&after.to_string())
            }
            Self::RemovedProfile => note.contains("profile removed"),
        }
    }
}

fn has_review_note(decreases: &[CoverageDecrease]) -> bool {
    review_note_paths()
        .into_iter()
        .any(|path| review_note_covers_decreases(&path, decreases))
}

fn review_note_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(path) = env::var_os(REVIEW_NOTE_ENV) {
        paths.push(PathBuf::from(path));
    }
    paths.push(PathBuf::from("specs/93-improvements-review.md"));
    paths.extend(review_dir_paths(Path::new("docs/reviews")));
    paths
}

fn review_dir_paths(path: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(path) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().and_then(|extension| extension.to_str()) == Some("md")
                && path.file_name().and_then(|name| name.to_str())
                    != Some("parity-snapshot-instructions.md")
        })
        .collect()
}

fn review_note_covers_decreases(path: &Path, decreases: &[CoverageDecrease]) -> bool {
    fs::read_to_string(path).is_ok_and(|content| {
        content.contains("coverage decrease")
            && decreases
                .iter()
                .all(|decrease| decrease.is_covered_by(&content))
    })
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

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, path::Path};

    use pdfv_core::{ModelSchemaProfileReport, ProfileCoverageParityReport};
    use serde_json::json;

    use super::{
        CoverageDecreaseField, coverage_decreases, review_note_covers_decreases, review_note_paths,
    };

    #[test]
    fn test_should_detect_total_lowered_bound_and_removed_profile_decreases()
    -> pdfv_core::Result<()> {
        let baseline = report(vec![
            profile("pdfa-1b", 10, 8, 6)?,
            profile("pdfua-1", 12, 10, 7)?,
        ])?;
        let current = report(vec![profile("pdfa-1b", 9, 7, 5)?])?;

        let decreases = coverage_decreases(&baseline, &current);

        assert_eq!(decreases.len(), 2);
        assert_eq!(decreases[0].flavour, "pdfa-1b");
        assert!(
            decreases[0]
                .fields
                .contains(&CoverageDecreaseField::Counter {
                    name: "totalRules",
                    before: 10,
                    after: 9,
                })
        );
        assert!(
            decreases[0]
                .fields
                .contains(&CoverageDecreaseField::Counter {
                    name: "loweredRules",
                    before: 8,
                    after: 7,
                })
        );
        assert!(
            decreases[0]
                .fields
                .contains(&CoverageDecreaseField::Counter {
                    name: "boundRules",
                    before: 6,
                    after: 5,
                })
        );
        assert_eq!(decreases[1].flavour, "pdfua-1");
        assert_eq!(
            decreases[1].fields,
            vec![CoverageDecreaseField::RemovedProfile]
        );
        Ok(())
    }

    #[test]
    fn test_should_require_review_note_to_name_decreased_profile_field_and_counts()
    -> pdfv_core::Result<()> {
        let baseline = report(vec![profile("pdfa-1b", 10, 8, 6)?])?;
        let current = report(vec![profile("pdfa-1b", 10, 7, 6)?])?;
        let decreases = coverage_decreases(&baseline, &current);
        let temp = tempfile::NamedTempFile::new()
            .map_err(|source| pdfv_core::PdfvError::Io { path: None, source })?;

        std::fs::write(
            temp.path(),
            "coverage decrease\nprofile pdfa-1b loweredRules 8 -> 7 reviewed",
        )
        .map_err(|source| pdfv_core::PdfvError::Io { path: None, source })?;
        assert!(review_note_covers_decreases(temp.path(), &decreases));

        std::fs::write(temp.path(), "coverage decrease\nreviewed generally")
            .map_err(|source| pdfv_core::PdfvError::Io { path: None, source })?;
        assert!(!review_note_covers_decreases(temp.path(), &decreases));
        Ok(())
    }

    #[test]
    fn test_should_not_treat_snapshot_instructions_as_review_note() {
        assert!(
            !review_note_paths()
                .iter()
                .any(|path| path == Path::new("docs/reviews/parity-snapshot-instructions.md"))
        );
    }

    fn report(
        profiles: Vec<ModelSchemaProfileReport>,
    ) -> pdfv_core::Result<ProfileCoverageParityReport> {
        serde_json::from_value(json!({
            "vendorPins": {},
            "profiles": profiles,
        }))
        .map_err(pdfv_core::ReportError::from)
        .map_err(Into::into)
    }

    fn profile(
        flavour: &str,
        total_rules: u64,
        lowered_rules: u64,
        bound_rules: u64,
    ) -> pdfv_core::Result<ModelSchemaProfileReport> {
        serde_json::from_value(json!({
            "flavour": flavour,
            "source": "test.xml",
            "totalRules": total_rules,
            "loweredRules": lowered_rules,
            "boundRules": bound_rules,
            "unsupportedByReason": BTreeMap::<String, u64>::new(),
        }))
        .map_err(pdfv_core::ReportError::from)
        .map_err(Into::into)
    }
}
