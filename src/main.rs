use std::io::Read;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;

use qf::error::QfError;
use qf::format::Format;
use qf::output;
use qf::parser;
use qf::query;

#[derive(Parser)]
#[command(name = "qf", version, about = "A fast, universal data format query tool")]
struct Cli {
    /// Path query expression (default: "." returns whole document)
    #[arg(default_value = ".")]
    query: String,

    /// Input file (reads from stdin if omitted)
    file: Option<PathBuf>,

    /// Force input format [yaml, json]
    #[arg(short = 'p', long = "input-format")]
    input_format: Option<String>,

    /// Output format [yaml, json] (default: same as input)
    #[arg(short, long = "output-format")]
    output_format: Option<String>,

    /// Compact output (no pretty printing)
    #[arg(short, long)]
    compact: bool,

    /// Raw string output (no quotes for string values)
    #[arg(short, long)]
    raw: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Read input
    let input = match &cli.file {
        Some(path) => {
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?
        }
        None => {
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .context("reading stdin")?;
            buf
        }
    };

    // Determine input format
    let in_fmt = match &cli.input_format {
        Some(f) => Format::from_str_name(f)?,
        None => match &cli.file {
            Some(path) => Format::from_extension(path)?,
            None => detect_format(&input)?,
        },
    };

    // Determine output format
    let out_fmt = match &cli.output_format {
        Some(f) => Format::from_str_name(f)?,
        None => in_fmt,
    };

    // Parse
    let value = parser::parse(&input, in_fmt)?;

    // Query
    let result = query::query(&value, &cli.query)?;

    // Output
    let formatted = output::format_value(&result, out_fmt, cli.compact, cli.raw)?;
    print!("{formatted}");

    // Ensure trailing newline for non-raw JSON output
    if !formatted.ends_with('\n') {
        println!();
    }

    Ok(())
}

/// Try to detect format from content when no file extension is available.
fn detect_format(input: &str) -> Result<Format, QfError> {
    let trimmed = input.trim_start();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        Ok(Format::Json)
    } else {
        // Default to YAML — it's a superset of many plain text formats
        Ok(Format::Yaml)
    }
}
