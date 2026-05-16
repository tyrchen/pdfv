#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]
//! Command-line entrypoint for pdfv.

use clap::Parser;

/// Command-line arguments for the pdfv binary.
#[derive(Debug, Parser)]
#[command(name = "pdfv", version, about = "Validate PDF conformance")]
struct Cli {
    /// Print the linked core library version.
    #[arg(long)]
    core_version: bool,
}

fn main() {
    let _cli = Cli::parse();
}
