use std::io::IsTerminal;
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
use qf::stream;

#[derive(Clone, Debug, PartialEq, Eq)]
enum ColorMode {
    Auto,
    Always,
    Never,
}

impl std::str::FromStr for ColorMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "auto" => Ok(ColorMode::Auto),
            "always" => Ok(ColorMode::Always),
            "never" => Ok(ColorMode::Never),
            other => Err(format!("invalid color mode: {other} (expected auto, always, never)")),
        }
    }
}

#[derive(Parser)]
#[command(name = "qf", version, about = "A fast, universal data format query tool")]
struct Cli {
    /// Path query expression (default: "." returns whole document)
    #[arg(default_value = ".")]
    query: String,

    /// Input file(s) (reads from stdin if omitted)
    files: Vec<PathBuf>,

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

    /// Colorize output [auto, always, never]
    #[arg(long, default_value = "auto")]
    color: ColorMode,

    /// Disable colorized output
    #[arg(long)]
    no_color: bool,

    /// Slurp: read all inputs into an array
    #[arg(short = 's', long)]
    slurp: bool,

    /// Read raw input lines as strings
    #[arg(long = "raw-input", short = 'R')]
    raw_input: bool,

    /// Join output (no newlines between outputs)
    #[arg(short = 'j', long = "join-output")]
    join_output: bool,

    /// Use null as input instead of reading from stdin/files
    #[arg(short = 'n', long = "null-input")]
    null_input: bool,

    /// Stream mode: process input records one at a time (for large files)
    #[arg(long)]
    stream: bool,

    /// Read input as newline-delimited JSON (NDJSON/JSON Lines)
    #[arg(long)]
    jsonl: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // For backward compat: treat first file arg as the single file
    let file = cli.files.first();

    // Validate: -i requires a file argument
    if cli.in_place && cli.files.is_empty() {
        anyhow::bail!("--in-place requires a file argument");
    }

    // Determine if we should colorize
    let colorize = should_colorize(&cli);

    // Read input
    let input = if cli.null_input {
        String::new()
    } else if cli.slurp && cli.files.len() > 1 {
        // Multi-file slurp: handled specially below
        String::new()
    } else {
        match file {
            Some(path) => {
                std::fs::read_to_string(path)
                    .with_context(|| format!("reading {}", path.display()))?
            }
            None => {
                let mut buf = String::new();
                std::io::stdin()
                    .read_to_string(&mut buf)
                    .context("reading stdin")?;
                buf
            }
        }
    };

    // Determine input format
    let in_fmt = match &cli.input_format {
        Some(f) => Format::from_str_name(f)?,
        None => match file {
            Some(path) => Format::from_extension(path)?,
            None => {
                if cli.null_input {
                    Format::Json
                } else {
                    detect_format(&input)?
                }
            }
        },
    };

    // Determine output format
    // In streaming/jsonl mode, default to JSON output since individual records
    // often can't serialize back to CSV/XML/etc.
    let out_fmt = match &cli.output_format {
        Some(f) => Format::from_str_name(f)?,
        None => {
            if cli.stream || cli.jsonl {
                Format::Json
            } else {
                in_fmt
            }
        }
    };

    // Handle null-input mode
    if cli.null_input {
        let value = serde_json::Value::Null;
        let results = query::query(&value, &cli.query)?;
        output_results(&results, out_fmt, &cli, colorize)?;
        return Ok(());
    }

    // Handle slurp mode with multiple files
    if cli.slurp && cli.files.len() > 1 {
        let mut all_values = Vec::new();
        for path in &cli.files {
            let content = std::fs::read_to_string(path)
                .with_context(|| format!("reading {}", path.display()))?;
            let fmt = match &cli.input_format {
                Some(f) => Format::from_str_name(f)?,
                None => Format::from_extension(path)?,
            };
            let val = parser::parse(&content, fmt)?;
            all_values.push(val);
        }
        let slurped = serde_json::Value::Array(all_values);
        let results = query::query(&slurped, &cli.query)?;
        output_results(&results, out_fmt, &cli, colorize)?;
        return Ok(());
    }

    // Handle raw-input mode
    if cli.raw_input {
        let lines: Vec<serde_json::Value> = input
            .lines()
            .map(|l| serde_json::Value::String(l.to_string()))
            .collect();
        let value = if cli.slurp {
            serde_json::Value::Array(lines)
        } else {
            // Process each line separately
            for line_val in &lines {
                let results = query::query(line_val, &cli.query)?;
                output_results(&results, out_fmt, &cli, colorize)?;
            }
            return Ok(());
        };
        let results = query::query(&value, &cli.query)?;
        output_results(&results, out_fmt, &cli, colorize)?;
        return Ok(());
    }

    // Handle JSONL (newline-delimited JSON) mode
    if cli.jsonl {
        stream::stream_ndjson(&input, &cli.query, |result| {
            let formatted = output::pretty::format_value_colored(
                &result, out_fmt, cli.compact, cli.raw, colorize,
            )
            .map_err(|e| QfError::Runtime(e.to_string()))?;
            print!("{formatted}");
            if !formatted.ends_with('\n') && !cli.join_output {
                println!();
            }
            Ok(())
        })?;
        return Ok(());
    }

    // Handle streaming mode
    if cli.stream {
        stream::stream_process(&input, in_fmt, &cli.query, |result| {
            let formatted = output::pretty::format_value_colored(
                &result, out_fmt, cli.compact, cli.raw, colorize,
            )
            .map_err(|e| QfError::Runtime(e.to_string()))?;
            print!("{formatted}");
            if !formatted.ends_with('\n') && !cli.join_output {
                println!();
            }
            Ok(())
        })?;
        return Ok(());
    }

    // Parse
    let value = parser::parse(&input, in_fmt)?;

    // Handle slurp with single file (wrap in array)
    let value = if cli.slurp && !cli.files.is_empty() {
        serde_json::Value::Array(vec![value])
    } else {
        value
    };

    // Query
    let results = query::query(&value, &cli.query)?;

    // Output
    if cli.in_place {
        let formatted = format_results(&results, out_fmt, &cli, false)?;
        let path = cli.files.first().unwrap();
        let parent = path.parent().unwrap_or(std::path::Path::new("."));
        let mut tmp = tempfile::NamedTempFile::new_in(parent)
            .context("creating temporary file")?;
        tmp.write_all(formatted.as_bytes())
            .context("writing temporary file")?;
        tmp.persist(path)
            .context("replacing file with updated content")?;
    } else {
        output_results(&results, out_fmt, &cli, colorize)?;
    }

    Ok(())
}

fn should_colorize(cli: &Cli) -> bool {
    if cli.no_color {
        return false;
    }
    if std::env::var("NO_COLOR").is_ok() {
        return false;
    }
    match cli.color {
        ColorMode::Always => true,
        ColorMode::Never => false,
        ColorMode::Auto => std::io::stdout().is_terminal(),
    }
}

fn format_results(
    results: &[serde_json::Value],
    out_fmt: Format,
    cli: &Cli,
    colorize: bool,
) -> Result<String, anyhow::Error> {
    let mut buf = String::new();
    let is_yaml = out_fmt == Format::Yaml;

    for (i, result) in results.iter().enumerate() {
        if is_yaml && i > 0 {
            buf.push_str("---\n");
        }
        let formatted = output::pretty::format_value_colored(
            result, out_fmt, cli.compact, cli.raw, colorize,
        )?;
        buf.push_str(&formatted);
        if !formatted.ends_with('\n') {
            if !cli.join_output {
                buf.push('\n');
            }
        }
    }

    Ok(buf)
}

fn output_results(
    results: &[serde_json::Value],
    out_fmt: Format,
    cli: &Cli,
    colorize: bool,
) -> Result<()> {
    let formatted = format_results(results, out_fmt, cli, colorize)?;
    print!("{formatted}");
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
