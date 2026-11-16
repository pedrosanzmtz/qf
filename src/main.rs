use std::io::{self, BufWriter, ErrorKind, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};

use clap::Parser;
use serde::Serialize;
use serde_json::Value;

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
            other => Err(format!(
                "invalid color mode: {other} (expected auto, always, never)"
            )),
        }
    }
}

#[derive(Parser)]
#[command(
    name = "qf",
    version,
    about = "A fast, universal data format query tool"
)]
struct Cli {
    /// Path query expression (default: "." returns whole document)
    query: Option<String>,

    /// Input file(s) (reads from stdin if omitted)
    files: Vec<PathBuf>,

    /// Force input format [yaml, json, xml, toml, csv, tsv]
    #[arg(short = 'p', long = "input-format")]
    input_format: Option<String>,

    /// Output format [yaml, json, xml, toml, csv, tsv] (default: same as input)
    #[arg(short, long = "output-format")]
    output_format: Option<String>,

    /// Edit file(s) in place
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

    /// Always colorize output (alias for --color always)
    #[arg(short = 'C')]
    color_always: bool,

    /// Never colorize output (alias for --color never)
    #[arg(short = 'M')]
    color_never: bool,

    /// Slurp: read all inputs into an array
    #[arg(short = 's', long)]
    slurp: bool,

    /// Read raw input lines as strings
    #[arg(long = "raw-input", short = 'R')]
    raw_input: bool,

    /// Join output (no newlines between outputs; implies -r)
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

    /// Set exit status based on output (1 if false/null, 4 if no output)
    #[arg(short = 'e', long = "exit-status")]
    exit_status: bool,

    /// Read query from file
    #[arg(short = 'f', long = "from-file", value_name = "PATH")]
    from_file: Option<PathBuf>,

    /// Sort object keys alphabetically in output
    #[arg(short = 'S', long = "sort-keys")]
    sort_keys: bool,

    /// Bind a string variable ($NAME = VALUE)
    #[arg(
        long = "arg",
        num_args = 2,
        value_names = ["NAME", "VALUE"],
        action = clap::ArgAction::Append
    )]
    args: Vec<String>,

    /// Bind a JSON-encoded variable ($NAME = JSON)
    #[arg(
        long = "argjson",
        num_args = 2,
        value_names = ["NAME", "JSON"],
        action = clap::ArgAction::Append
    )]
    argjsons: Vec<String>,

    /// Use N spaces for indentation (JSON output)
    #[arg(long, value_name = "N")]
    indent: Option<usize>,

    /// Use tabs for indentation (JSON output)
    #[arg(long)]
    tab: bool,
}

#[derive(Debug)]
struct CliError {
    code: i32,
    message: String,
}

impl CliError {
    fn usage(message: impl Into<String>) -> Self {
        Self {
            code: 2,
            message: message.into(),
        }
    }

    fn io(message: impl Into<String>) -> Self {
        Self {
            code: 2,
            message: message.into(),
        }
    }

    fn runtime(message: impl Into<String>) -> Self {
        Self {
            code: 5,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl From<QfError> for CliError {
    fn from(err: QfError) -> Self {
        let code = match err {
            QfError::SyntaxError { .. } => 3,
            QfError::Io(_)
            | QfError::UnsupportedFormat(_)
            | QfError::NoExtension
            | QfError::UnknownExtension(_) => 2,
            QfError::Parse(_)
            | QfError::Serialize(_)
            | QfError::TypeError(_)
            | QfError::UndefinedVariable(_)
            | QfError::UndefinedFunction(_, _)
            | QfError::Runtime(_)
            | QfError::UserError(_) => 5,
        };
        Self {
            code,
            message: err.to_string(),
        }
    }
}

#[derive(Clone)]
struct FormatOpts {
    compact: bool,
    raw: bool,
    colorize: bool,
    join_output: bool,
    sort_keys: bool,
    indent_str: Option<String>,
}

impl FormatOpts {
    fn from_cli(cli: &Cli, colorize: bool) -> Self {
        let indent_str = if cli.tab {
            Some("\t".to_string())
        } else {
            cli.indent.map(|n| " ".repeat(n))
        };
        Self {
            compact: cli.compact,
            // jq behavior: -j implies -r
            raw: cli.raw || cli.join_output,
            colorize,
            join_output: cli.join_output,
            sort_keys: cli.sort_keys,
            indent_str,
        }
    }
}

struct OutputState<W: Write> {
    writer: W,
    out_fmt: Format,
    opts: FormatOpts,
    output_count: usize,
    last_value: Option<Value>,
    broken_pipe: bool,
}

impl<W: Write> OutputState<W> {
    fn new(writer: W, out_fmt: Format, opts: FormatOpts) -> Self {
        Self {
            writer,
            out_fmt,
            opts,
            output_count: 0,
            last_value: None,
            broken_pipe: false,
        }
    }

    fn write_value(&mut self, value: Value) -> Result<(), CliError> {
        if self.broken_pipe {
            return Ok(());
        }
        let val = if self.opts.sort_keys {
            sort_value_keys(value)
        } else {
            value
        };
        self.last_value = Some(val.clone());

        if self.out_fmt == Format::Yaml && self.output_count > 0 {
            self.write_bytes(b"---\n")?;
            if self.broken_pipe {
                return Ok(());
            }
        }
        self.output_count += 1;

        let formatted = format_single_value(&val, self.out_fmt, &self.opts)?;
        self.write_bytes(formatted.as_bytes())?;
        if !self.broken_pipe && !formatted.ends_with('\n') && !self.opts.join_output {
            self.write_bytes(b"\n")?;
        }
        Ok(())
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), CliError> {
        match self.writer.write_all(bytes) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == ErrorKind::BrokenPipe => {
                self.broken_pipe = true;
                Ok(())
            }
            Err(e) => Err(CliError::io(e.to_string())),
        }
    }

    fn flush(&mut self) -> Result<(), CliError> {
        if self.broken_pipe {
            return Ok(());
        }
        match self.writer.flush() {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == ErrorKind::BrokenPipe => {
                self.broken_pipe = true;
                Ok(())
            }
            Err(e) => Err(CliError::io(e.to_string())),
        }
    }
}

fn main() {
    let code = match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("qf: error: {err}");
            err.code
        }
    };
    std::process::exit(code);
}

fn run() -> Result<i32, CliError> {
    let mut cli = Cli::parse();

    // When -f is used, there is no positional query; any positional arg is a file.
    let raw_query = if let Some(ref query_path) = cli.from_file {
        if let Some(first_file) = cli.query.take() {
            cli.files.insert(0, PathBuf::from(first_file));
        }
        std::fs::read_to_string(query_path).map_err(|e| {
            CliError::io(format!("reading query file {}: {e}", query_path.display()))
        })?
    } else {
        cli.query.take().unwrap_or_else(|| ".".to_string())
    };

    if cli.in_place && cli.files.is_empty() {
        return Err(CliError::usage("--in-place requires a file argument"));
    }

    let query_str = build_query_with_args(&raw_query, &cli.args, &cli.argjsons)?;
    let colorize = should_colorize(&cli);
    let opts = FormatOpts::from_cli(&cli, colorize);

    if cli.in_place {
        return run_in_place(&cli, &query_str, &opts);
    }

    let stdout = io::stdout().lock();
    let writer = BufWriter::new(stdout);

    let (output_count, last_value, broken_pipe) = if cli.null_input {
        let out_fmt = resolve_output_format(cli.output_format.as_deref(), Format::Json)?;
        let mut out = OutputState::new(writer, out_fmt, opts);
        let results = query::query(&Value::Null, &query_str)?;
        for res in results {
            out.write_value(res)?;
            if out.broken_pipe {
                break;
            }
        }
        out.flush()?;
        (out.output_count, out.last_value, out.broken_pipe)
    } else if cli.raw_input {
        let out_fmt = resolve_output_format(cli.output_format.as_deref(), Format::Json)?;
        let mut out = OutputState::new(writer, out_fmt, opts);
        run_raw_input(&cli.files, cli.slurp, &query_str, &mut out)?;
        out.flush()?;
        (out.output_count, out.last_value, out.broken_pipe)
    } else if cli.files.is_empty() {
        run_stdin(&cli, &query_str, opts, writer)?
    } else {
        run_files(&cli, &query_str, opts, writer)?
    };

    if broken_pipe {
        return Ok(0);
    }

    if cli.exit_status {
        if output_count == 0 {
            return Ok(4);
        }
        if matches!(last_value, Some(Value::Null) | Some(Value::Bool(false))) {
            return Ok(1);
        }
    }

    Ok(0)
}

fn run_in_place(cli: &Cli, query_str: &str, base_opts: &FormatOpts) -> Result<i32, CliError> {
    let mut output_count = 0usize;
    let mut last_value: Option<Value> = None;

    for path in &cli.files {
        let target_path = std::fs::canonicalize(path)
            .map_err(|e| CliError::io(format!("resolving {}: {e}", path.display())))?;
        let perms = std::fs::metadata(&target_path)
            .map_err(|e| CliError::io(format!("reading metadata {}: {e}", target_path.display())))?
            .permissions();
        let content = std::fs::read_to_string(&target_path)
            .map_err(|e| CliError::io(format!("reading {}: {e}", target_path.display())))?;

        let in_fmt = resolve_input_format(cli.input_format.as_deref(), Some(path), &content)?;
        if let Some(ref out_f) = cli.output_format {
            let requested = Format::from_str_name(out_f)?;
            if requested != in_fmt {
                return Err(CliError::usage(format!(
                    "cannot use --in-place with a different output format ({requested} != {in_fmt})"
                )));
            }
        }

        let values = parse_values(&content, in_fmt)?;
        let mut results = Vec::new();
        if cli.slurp {
            results.extend(query::query(&Value::Array(values), query_str)?);
        } else {
            for val in &values {
                results.extend(query::query(val, query_str)?);
            }
        }

        if results.len() != 1 {
            return Err(CliError::runtime(format!(
                "in-place edit requires query to produce exactly 1 result, got {}",
                results.len()
            )));
        }

        let mut result = results.into_iter().next().unwrap();
        if base_opts.sort_keys {
            result = sort_value_keys(result);
        }
        output_count += 1;
        last_value = Some(result.clone());

        let mut file_opts = base_opts.clone();
        file_opts.colorize = false;
        let mut formatted = format_single_value(&result, in_fmt, &file_opts)?;
        if !formatted.ends_with('\n') && !file_opts.join_output {
            formatted.push('\n');
        }

        let parent = target_path.parent().unwrap_or(Path::new("."));
        let mut tmp = tempfile::NamedTempFile::new_in(parent)
            .map_err(|e| CliError::io(format!("creating temporary file: {e}")))?;
        tmp.write_all(formatted.as_bytes())
            .map_err(|e| CliError::io(format!("writing temporary file: {e}")))?;
        tmp.as_file()
            .set_permissions(perms)
            .map_err(|e| CliError::io(format!("setting permissions on temporary file: {e}")))?;
        tmp.persist(&target_path)
            .map_err(|e| CliError::io(format!("replacing {}: {e}", target_path.display())))?;
    }

    if cli.exit_status {
        if output_count == 0 {
            return Ok(4);
        }
        if matches!(last_value, Some(Value::Null) | Some(Value::Bool(false))) {
            return Ok(1);
        }
    }

    Ok(0)
}

fn run_raw_input<W: Write>(
    files: &[PathBuf],
    slurp: bool,
    query_str: &str,
    out: &mut OutputState<W>,
) -> Result<(), CliError> {
    let mut lines = Vec::new();
    if files.is_empty() {
        let input = read_stdin()?;
        for line in input.lines() {
            lines.push(Value::String(line.to_string()));
        }
    } else {
        for path in files {
            let input = std::fs::read_to_string(path)
                .map_err(|e| CliError::io(format!("reading {}: {e}", path.display())))?;
            for line in input.lines() {
                lines.push(Value::String(line.to_string()));
            }
        }
    }

    if slurp {
        let results = query::query(&Value::Array(lines), query_str)?;
        for res in results {
            out.write_value(res)?;
            if out.broken_pipe {
                break;
            }
        }
    } else {
        for line_val in &lines {
            let results = query::query(line_val, query_str)?;
            for res in results {
                out.write_value(res)?;
                if out.broken_pipe {
                    return Ok(());
                }
            }
        }
    }

    Ok(())
}

fn run_stdin<W: Write>(
    cli: &Cli,
    query_str: &str,
    opts: FormatOpts,
    writer: W,
) -> Result<(usize, Option<Value>, bool), CliError> {
    let input = read_stdin()?;
    let in_fmt = resolve_input_format(cli.input_format.as_deref(), None, &input)?;
    let default_out = if cli.stream || cli.jsonl {
        Format::Json
    } else {
        in_fmt
    };
    let out_fmt = resolve_output_format(cli.output_format.as_deref(), default_out)?;
    let mut out = OutputState::new(writer, out_fmt, opts);

    if cli.jsonl && !cli.slurp {
        stream_ndjson_to_out(&input, query_str, &mut out)?;
    } else if cli.stream && !cli.slurp {
        stream_process_to_out(&input, in_fmt, query_str, &mut out)?;
    } else {
        let values = parse_values(&input, in_fmt)?;
        if cli.slurp {
            let results = query::query(&Value::Array(values), query_str)?;
            for res in results {
                out.write_value(res)?;
                if out.broken_pipe {
                    break;
                }
            }
        } else {
            for val in &values {
                let results = query::query(val, query_str)?;
                for res in results {
                    out.write_value(res)?;
                    if out.broken_pipe {
                        break;
                    }
                }
                if out.broken_pipe {
                    break;
                }
            }
        }
    }

    out.flush()?;
    Ok((out.output_count, out.last_value, out.broken_pipe))
}

fn run_files<W: Write>(
    cli: &Cli,
    query_str: &str,
    opts: FormatOpts,
    writer: W,
) -> Result<(usize, Option<Value>, bool), CliError> {
    if cli.slurp {
        let mut all_values = Vec::new();
        let mut first_fmt: Option<Format> = None;

        for path in &cli.files {
            let content = std::fs::read_to_string(path)
                .map_err(|e| CliError::io(format!("reading {}: {e}", path.display())))?;
            let fmt = resolve_input_format(cli.input_format.as_deref(), Some(path), &content)?;
            if first_fmt.is_none() {
                first_fmt = Some(fmt);
            }
            all_values.extend(parse_values(&content, fmt)?);
        }

        let default_out = first_fmt.unwrap_or(Format::Json);
        let out_fmt = resolve_output_format(cli.output_format.as_deref(), default_out)?;
        let mut out = OutputState::new(writer, out_fmt, opts);

        let results = query::query(&Value::Array(all_values), query_str)?;
        for res in results {
            out.write_value(res)?;
            if out.broken_pipe {
                break;
            }
        }
        out.flush()?;
        return Ok((out.output_count, out.last_value, out.broken_pipe));
    }

    // Non-slurp: process each file in order, determining output format from the first file.
    let mut out_opt: Option<OutputState<W>> = None;
    let mut writer_opt = Some(writer);

    for path in &cli.files {
        let content = std::fs::read_to_string(path)
            .map_err(|e| CliError::io(format!("reading {}: {e}", path.display())))?;
        let in_fmt = resolve_input_format(cli.input_format.as_deref(), Some(path), &content)?;
        let is_ndjson =
            cli.jsonl || (cli.input_format.is_none() && Format::is_ndjson_extension(path));

        if out_opt.is_none() {
            let default_out = if cli.stream || is_ndjson {
                Format::Json
            } else {
                in_fmt
            };
            let out_fmt = resolve_output_format(cli.output_format.as_deref(), default_out)?;
            out_opt = Some(OutputState::new(
                writer_opt.take().unwrap(),
                out_fmt,
                opts.clone(),
            ));
        }

        let out = out_opt.as_mut().unwrap();
        if is_ndjson {
            stream_ndjson_to_out(&content, query_str, out)?;
        } else if cli.stream {
            stream_process_to_out(&content, in_fmt, query_str, out)?;
        } else {
            let values = parse_values(&content, in_fmt)?;
            for val in &values {
                let results = query::query(val, query_str)?;
                for res in results {
                    out.write_value(res)?;
                    if out.broken_pipe {
                        break;
                    }
                }
                if out.broken_pipe {
                    break;
                }
            }
        }

        if out.broken_pipe {
            break;
        }
    }

    let mut out = out_opt.unwrap();
    out.flush()?;
    Ok((out.output_count, out.last_value, out.broken_pipe))
}

fn stream_ndjson_to_out<W: Write>(
    input: &str,
    query_str: &str,
    out: &mut OutputState<W>,
) -> Result<(), CliError> {
    let res = stream::stream_ndjson(input, query_str, |result| {
        if let Err(err) = out.write_value(result) {
            return Err(QfError::Runtime(err.message));
        }
        Ok(())
    });
    match res {
        Ok(()) => Ok(()),
        Err(e) if out.broken_pipe => {
            let _ = e;
            Ok(())
        }
        Err(e) => Err(CliError::from(e)),
    }
}

fn stream_process_to_out<W: Write>(
    input: &str,
    in_fmt: Format,
    query_str: &str,
    out: &mut OutputState<W>,
) -> Result<(), CliError> {
    let res = stream::stream_process(input, in_fmt, query_str, |result| {
        if let Err(err) = out.write_value(result) {
            return Err(QfError::Runtime(err.message));
        }
        Ok(())
    });
    match res {
        Ok(()) => Ok(()),
        Err(e) if out.broken_pipe => {
            let _ = e;
            Ok(())
        }
        Err(e) => Err(CliError::from(e)),
    }
}

fn read_stdin() -> Result<String, CliError> {
    let mut buf = String::new();
    io::stdin()
        .read_to_string(&mut buf)
        .map_err(|e| CliError::io(format!("reading stdin: {e}")))?;
    Ok(buf)
}

fn resolve_input_format(
    cli_fmt: Option<&str>,
    path: Option<&Path>,
    content: &str,
) -> Result<Format, CliError> {
    match cli_fmt {
        Some(f) => Ok(Format::from_str_name(f)?),
        None => match path {
            Some(p) => match Format::from_extension(p) {
                Ok(fmt) => Ok(fmt),
                Err(QfError::NoExtension | QfError::UnknownExtension(_)) => {
                    Ok(Format::detect_from_content(content))
                }
                Err(e) => Err(CliError::from(e)),
            },
            None => Ok(Format::detect_from_content(content)),
        },
    }
}

fn resolve_output_format(cli_fmt: Option<&str>, default: Format) -> Result<Format, CliError> {
    match cli_fmt {
        Some(f) => Ok(Format::from_str_name(f)?),
        None => Ok(default),
    }
}

fn parse_values(content: &str, fmt: Format) -> Result<Vec<Value>, CliError> {
    if fmt == Format::Json {
        let mut values = Vec::new();
        for item in serde_json::Deserializer::from_str(content).into_iter::<Value>() {
            match item {
                Ok(v) => values.push(v),
                Err(e) => return Err(CliError::from(QfError::Parse(e.to_string()))),
            }
        }
        Ok(values)
    } else {
        Ok(vec![parser::parse(content, fmt)?])
    }
}

fn build_query_with_args(
    query: &str,
    args: &[String],
    argjsons: &[String],
) -> Result<String, CliError> {
    let mut prefix = String::new();

    for [name, val] in args.as_chunks::<2>().0 {
        validate_var_name(name)?;
        let json_lit = serde_json::to_string(&Value::String(val.clone()))
            .map_err(|e| CliError::usage(format!("encoding --arg {name}: {e}")))?;
        prefix.push_str(&format!("{json_lit} as ${name} | "));
    }

    for [name, raw_json] in argjsons.as_chunks::<2>().0 {
        validate_var_name(name)?;
        let parsed: Value = serde_json::from_str(raw_json)
            .map_err(|e| CliError::usage(format!("invalid JSON for --argjson {name}: {e}")))?;
        let json_lit = serde_json::to_string(&parsed)
            .map_err(|e| CliError::usage(format!("encoding --argjson {name}: {e}")))?;
        prefix.push_str(&format!("{json_lit} as ${name} | "));
    }

    if prefix.is_empty() {
        Ok(query.to_string())
    } else {
        Ok(format!("{prefix}{query}"))
    }
}

fn validate_var_name(name: &str) -> Result<(), CliError> {
    let mut chars = name.chars();
    let valid = match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {
            chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        _ => false,
    };
    if !valid {
        return Err(CliError::usage(format!("invalid variable name: {name}")));
    }
    Ok(())
}

fn should_colorize(cli: &Cli) -> bool {
    if cli.no_color || cli.color_never || cli.color == ColorMode::Never {
        return false;
    }
    if cli.color_always || cli.color == ColorMode::Always {
        return true;
    }
    if std::env::var("NO_COLOR").is_ok() {
        return false;
    }
    io::stdout().is_terminal()
}

fn sort_value_keys(value: Value) -> Value {
    match value {
        Value::Array(arr) => Value::Array(arr.into_iter().map(sort_value_keys).collect()),
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map
                .into_iter()
                .map(|(k, v)| (k, sort_value_keys(v)))
                .collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            let mut sorted = serde_json::Map::new();
            for (k, v) in entries {
                sorted.insert(k, v);
            }
            Value::Object(sorted)
        }
        other => other,
    }
}

fn format_single_value(
    value: &Value,
    out_fmt: Format,
    opts: &FormatOpts,
) -> Result<String, CliError> {
    if opts.raw {
        if let Value::String(s) = value {
            return Ok(s.clone());
        }
    }

    if out_fmt == Format::Json && !opts.compact {
        if let Some(ref indent) = opts.indent_str {
            let mut buf = Vec::new();
            let formatter = serde_json::ser::PrettyFormatter::with_indent(indent.as_bytes());
            let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);
            value
                .serialize(&mut ser)
                .map_err(|e| CliError::runtime(e.to_string()))?;
            return String::from_utf8(buf).map_err(|e| CliError::runtime(e.to_string()));
        }
    }

    output::pretty::format_value_colored(value, out_fmt, opts.compact, opts.raw, opts.colorize)
        .map_err(CliError::from)
}
