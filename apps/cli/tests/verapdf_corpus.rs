#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "the opt-in corpus harness creates a local parse-error fixture synchronously before \
              invoking the CLI"
)]

use std::{
    env,
    error::Error,
    ffi::OsString,
    fs::File,
    io,
    path::{Path, PathBuf},
    process::Output,
};

use assert_cmd::Command;
use predicates::{Predicate, str::contains};
use tempfile::tempdir;

const CORPUS_ENV: &str = "PDFV_VERAPDF_CORPUS_DIR";
const EXIT_PARSE_FAILED: i32 = 2;
const EXIT_INCOMPLETE: i32 = 4;
const EXIT_USAGE: i32 = 64;
const PDFA_1B_XREF_DIR: &str = "PDF_A-1b/6.1 File structure/6.1.4 Cross reference table";
const PDFA_1B_INFO_DIR: &str = "PDF_A-1b/6.1 File structure/6.1.5 Document information dictionary";
const XREF_PASS: &str = "veraPDF test suite 6-1-4-t03-pass-b.pdf";
const XREF_FAIL: &str = "veraPDF test suite 6-1-4-t02-fail-a.pdf";
const INFO_PASS_A: &str = "veraPDF test suite 6-1-5-t02-pass-a.pdf";
const INFO_PASS_B: &str = "veraPDF test suite 6-1-5-t02-pass-b.pdf";
const INFO_PASS_C: &str = "veraPDF test suite 6-1-5-t02-pass-c.pdf";
const INFO_FAIL_A: &str = "veraPDF test suite 6-1-5-t01-fail-a.pdf";
const INFO_FAIL_B: &str = "veraPDF test suite 6-1-5-t01-fail-b.pdf";
const INFO_FAIL_C: &str = "veraPDF test suite 6-1-5-t01-fail-c.pdf";

#[derive(Debug)]
struct CorpusCase {
    name: &'static str,
    args: Vec<OsString>,
    expected_exit: i32,
    expected_stdout: &'static [&'static str],
}

#[ignore = "requires PDFV_VERAPDF_CORPUS_DIR and full veraPDF corpus fixtures"]
#[test]
fn test_should_match_verapdf_corpus_validation_exit_status_slice() -> Result<(), Box<dyn Error>> {
    let corpus = corpus_dir()?;
    let cases = [
        CorpusCase {
            name: "single pass",
            args: validate_args([corpus_file(&corpus, PDFA_1B_XREF_DIR, XREF_PASS)]),
            expected_exit: EXIT_INCOMPLETE,
            expected_stdout: &[r#""status":"incomplete""#, r#""failedRules":0"#],
        },
        CorpusCase {
            name: "single fail",
            args: validate_args([corpus_file(&corpus, PDFA_1B_XREF_DIR, XREF_FAIL)]),
            expected_exit: EXIT_INCOMPLETE,
            expected_stdout: &[r#""status":"incomplete""#, r#""failedRules":0"#],
        },
        CorpusCase {
            name: "batch pass",
            args: validate_args([
                corpus_file(&corpus, PDFA_1B_INFO_DIR, INFO_PASS_A),
                corpus_file(&corpus, PDFA_1B_INFO_DIR, INFO_PASS_B),
                corpus_file(&corpus, PDFA_1B_INFO_DIR, INFO_PASS_C),
            ]),
            expected_exit: EXIT_INCOMPLETE,
            expected_stdout: &[r#""totalFiles":3"#, r#""incomplete":3"#],
        },
        CorpusCase {
            name: "batch fail",
            args: validate_args([
                corpus_file(&corpus, PDFA_1B_INFO_DIR, INFO_FAIL_A),
                corpus_file(&corpus, PDFA_1B_INFO_DIR, INFO_FAIL_B),
                corpus_file(&corpus, PDFA_1B_INFO_DIR, INFO_FAIL_C),
            ]),
            expected_exit: EXIT_INCOMPLETE,
            expected_stdout: &[r#""totalFiles":3"#, r#""incomplete":3"#],
        },
        CorpusCase {
            name: "directory mixed",
            args: validate_args_with_options(["--recursive"], [corpus.join(PDFA_1B_INFO_DIR)]),
            expected_exit: EXIT_INCOMPLETE,
            expected_stdout: &[r#""incomplete":"#],
        },
    ];

    for case in cases {
        let output = run_pdfv(case.args)?;
        assert_output(case.name, &output, case.expected_exit, case.expected_stdout)?;
    }

    Ok(())
}

#[ignore = "requires PDFV_VERAPDF_CORPUS_DIR and full veraPDF corpus fixtures"]
#[test]
fn test_should_match_verapdf_corpus_error_exit_status_slice() -> Result<(), Box<dyn Error>> {
    let corpus = corpus_dir()?;
    let pass_file = corpus_file(&corpus, PDFA_1B_XREF_DIR, XREF_PASS);

    let usage_output = run_pdfv([
        OsString::from("validate"),
        OsString::from("--flavour"),
        OsString::from("jbnd"),
        pass_file.into_os_string(),
    ])?;
    assert_output("bad params", &usage_output, EXIT_USAGE, &[])?;

    let temp = tempdir()?;
    let empty_pdf = temp.path().join("test.pdf");
    File::create(&empty_pdf)?;
    let parse_output = run_pdfv(validate_args([empty_pdf]))?;
    assert_output(
        "parse error",
        &parse_output,
        EXIT_PARSE_FAILED,
        &[r#""status":"parseFailed""#],
    )?;

    Ok(())
}

fn corpus_dir() -> Result<PathBuf, Box<dyn Error>> {
    let path = env::var_os(CORPUS_ENV).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("{CORPUS_ENV} must point to a veraPDF-corpus checkout"),
        )
    })?;
    let path = PathBuf::from(path);
    if !path.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} is not a directory", path.display()),
        )
        .into());
    }
    Ok(path)
}

fn corpus_file(corpus: &Path, directory: &str, file_name: &str) -> PathBuf {
    corpus.join(directory).join(file_name)
}

fn validate_args(paths: impl IntoIterator<Item = PathBuf>) -> Vec<OsString> {
    validate_args_with_options(std::iter::empty::<&str>(), paths)
}

fn validate_args_with_options(
    options: impl IntoIterator<Item = &'static str>,
    paths: impl IntoIterator<Item = PathBuf>,
) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("validate"),
        OsString::from("--format"),
        OsString::from("json"),
        OsString::from("--flavour"),
        OsString::from("pdfa-1b"),
    ];
    args.extend(options.into_iter().map(OsString::from));
    args.extend(paths.into_iter().map(PathBuf::into_os_string));
    args
}

fn run_pdfv(args: impl IntoIterator<Item = OsString>) -> Result<Output, Box<dyn Error>> {
    Command::cargo_bin("pdfv")?
        .args(args)
        .output()
        .map_err(Into::into)
}

fn assert_output(
    case_name: &str,
    output: &Output,
    expected_exit: i32,
    expected_stdout: &[&str],
) -> Result<(), Box<dyn Error>> {
    assert_eq!(
        output.status.code(),
        Some(expected_exit),
        "{case_name} stderr:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8(output.stdout.clone())?;
    for expected in expected_stdout {
        assert!(
            contains(*expected).eval(&stdout),
            "{case_name} stdout missing {expected}:\n{stdout}",
        );
    }
    Ok(())
}
