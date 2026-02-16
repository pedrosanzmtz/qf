use std::io::Read;
use std::io::Write;
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

    /// Force input format [yaml, json, xml, toml, csv, tsv]
    #[arg(short = 'p', long = "input-format")]
    input_format: Option<String>,

    /// Output format [yaml, json, xml, toml, csv, tsv] (default: same as input)
    #[arg(short, long = "output-format")]
    output_format: Option<String>,

    /// Edit file in place
    #[arg(short, long = "in-place")]
    in_place: bool,

    /// Compact output (no pretty printing)
    #[arg(short, long)]
    compact: bool,

    /// Raw string output (no quotes for string values)
    #[arg(short, long)]
    raw: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Validate: -i requires a file argument
    if cli.in_place && cli.file.is_none() {
        anyhow::bail!("--in-place requires a file argument");
    }

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
    let mut formatted = output::format_value(&result, out_fmt, cli.compact, cli.raw)?;

    // Ensure trailing newline
    if !formatted.ends_with('\n') {
        formatted.push('\n');
    }

    if cli.in_place {
        // Atomic write: write to temp file then rename
        let path = cli.file.as_ref().unwrap();
        let parent = path.parent().unwrap_or(std::path::Path::new("."));
        let mut tmp = tempfile::NamedTempFile::new_in(parent)
            .context("creating temporary file")?;
        tmp.write_all(formatted.as_bytes())
            .context("writing temporary file")?;
        tmp.persist(path)
            .context("replacing file with updated content")?;
    } else {
        print!("{formatted}");
    }

    Ok(())
}

/// Try to detect format from content when no file extension is available.
fn detect_format(input: &str) -> Result<Format, QfError> {
    let trimmed = input.trim_start();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        Ok(Format::Json)
    } else if trimmed.starts_with('<') {
        Ok(Format::Xml)
    } else {
        // Default to YAML — it's a superset of many plain text formats
        Ok(Format::Yaml)
    }
}
