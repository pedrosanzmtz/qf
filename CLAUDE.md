# qf: A Rust-based Universal Data Format Query Tool

## Project Vision

Build a high-performance, Rust-based alternative to mikefarah/yq that can query, transform, and convert between multiple data formats (YAML, JSON, XML, TOML, CSV) with better performance and memory efficiency.

**Name**: `qf` stands for "query format" - a short, memorable 2-letter command that's easy to type and available on crates.io.

## Why This Project?

### Problem Statement
- **mikefarah/yq (Go)**: Feature-rich but slower on large files, higher memory usage
- **kislyuk/yq (Python)**: Just a wrapper around jq, limited to YAML/JSON
- **No Rust equivalent**: Gap in the ecosystem for a fast, memory-efficient multi-format query tool

### Use Cases
1. **DevOps**: Quick queries on Kubernetes configs, Docker Compose files, CI/CD configs
2. **Data Engineering**: Transform large configuration files without loading entire file into memory
3. **Telecom/Enterprise**: Process vendor configs (Ericsson, Nokia XML), network configs, large structured data
4. **General Development**: Universal data format converter and query tool

## Developer Context

This project leverages experience from **xmlshift** - a Rust-based XML parser built for processing massive telecom data files (up to 63GB). Key learnings applied:

- Config-driven architecture (similar to xmlshift's YAML config approach)
- Streaming parsers for large files
- Performance-critical path optimization
- CLI design for production use

## Implementation Status

### Phase 1: Basic Format Support - COMPLETE
- [x] YAML reading/writing
- [x] JSON reading/writing
- [x] Format conversion: YAML <-> JSON
- [x] Basic path queries (e.g., `.spec.containers[0].image`)
- [x] Pretty printing output

### Phase 2: Extended Formats - COMPLETE
- [x] XML support (via quick-xml)
- [x] TOML support
- [x] CSV/TSV support
- [x] In-place editing (`-i` flag)

### Phase 3: Advanced Features - COMPLETE
- [x] JQ-compatible query syntax (full lexer -> parser -> AST -> evaluator)
- [x] Streaming mode for large files (JSON, NDJSON, XML, CSV/TSV)
- [x] Merge operations (`--slurp`, `*` operator)
- [x] Template evaluation (string interpolation)
- [x] Colorized output (ANSI, TTY-aware, NO_COLOR support)
- [x] Lossless TOML in-place editing (`-i`) via `toml_edit`
- [x] `jq` CLI flags (`--arg`, `--argjson`, `-e`, `-f`, `-S`, `-C`, `-M`, `--indent`, `--tab`)

See [docs/compatibility.md](docs/compatibility.md) for the full `jq` compatibility matrix, known divergences, format-mapping conventions, and `--stream` behavior.

## Technical Architecture

### Dependencies
```toml
[dependencies]
serde = { version = "1.0", features = ["derive"] }
serde_json = { version = "1.0", features = ["preserve_order"] }
serde_yaml = "0.9"
quick-xml = { version = "0.37", features = ["serialize"] }
toml = "0.8"
toml_edit = "0.22"
csv = "1.3"
clap = { version = "4.4", features = ["derive"] }
anyhow = "1.0"
thiserror = "1.0"
tempfile = "3.14"
regex = "1.10"
base64 = "0.22"

[dev-dependencies]
assert_cmd = "2.0"
predicates = "3.1"
```

### CLI Design
```bash
qf [OPTIONS] [QUERY] [FILES...]

Options:
  -p, --input-format <FORMAT>   Input format [yaml, json, xml, toml, csv, tsv]
  -o, --output-format <FORMAT>  Output format [yaml, json, xml, toml, csv, tsv]
  -i, --in-place                Edit file(s) in place
  -c, --compact                 Compact output (no pretty print)
  -r, --raw                     Output raw strings (no quotes)
  -s, --slurp                   Read all inputs into an array
  -R, --raw-input               Read raw input lines as strings
  -j, --join-output             No newlines between outputs (implies -r)
  -n, --null-input              Use null as input
  -e, --exit-status             Set exit status based on output (1 if false/null, 4 if no output)
  -f, --from-file <PATH>        Read query from file
  -S, --sort-keys               Sort object keys alphabetically in output
      --arg <NAME> <VALUE>      Bind a string variable ($NAME = VALUE)
      --argjson <NAME> <JSON>   Bind a JSON-encoded variable ($NAME = JSON)
      --indent <N>              Use N spaces for indentation (JSON output)
      --tab                     Use tabs for indentation (JSON output)
      --color <MODE>            Colorize output [auto, always, never]
      --no-color                Disable colorized output
  -C                            Always colorize output (alias for --color always)
  -M                            Never colorize output (alias for --color never)
      --stream                  Stream mode (process records/elements one at a time)
      --jsonl                   Read input as NDJSON/JSON Lines

Examples:
  qf '.spec.containers[0]' deployment.yaml
  qf -p xml -o json '.root.element' config.xml
  qf -i '.version = "2.0"' config.yaml
  qf -s '.[0] * .[1]' base.yaml overlay.yaml
  qf '"Hello \(.name)"' data.json
  qf --stream -p csv '.name' large.csv
  qf --jsonl '.status' events.jsonl
```

### Source Tree

```
src/
    main.rs              # CLI entry point, flag handling, output dispatch
    lib.rs               # Public module declarations
    error.rs             # QfError enum (Parse, SyntaxError, TypeError, Runtime, etc.)
    format.rs            # Format enum, detection from extension/content
    parser/
        mod.rs           # Parser dispatch by format
        yaml.rs          # YAML parser (serde_yaml)
        json.rs          # JSON parser (serde_json)
        xml.rs           # XML parser (quick-xml deserialize)
        toml.rs          # TOML parser
        csv.rs           # CSV parser
        tsv.rs           # TSV parser
    query/
        mod.rs           # Query entry point, integration tests
        path.rs          # Legacy path-based query engine
        lexer.rs         # JQ tokenizer
        ast.rs           # Expression tree types
        jq_parser.rs     # Recursive descent parser with Pratt precedence
        eval.rs          # JQ evaluator (generator model, Vec<Value>)
        builtins.rs      # 80+ built-in functions
        env.rs           # Variable/function environment
    output/
        mod.rs           # Output module declarations
        pretty.rs        # Pretty printing + format dispatch
        preserve.rs      # Lossless TOML diff/in-place preservation (toml_edit)
        color.rs         # ANSI colorization for JSON and YAML
    stream/
        mod.rs           # Stream processing dispatch
        json.rs          # JSON streaming + NDJSON
        xml.rs           # XML event-based streaming
        csv.rs           # CSV/TSV row-by-row streaming
```

### Key Design Patterns

1. **Unified Data Model**: `serde_json::Value` as intermediate representation for all formats
2. **Generator Model**: JQ queries return `Vec<Value>` (0, 1, or many outputs per filter)
3. **Streaming**: Event-based parsers for large files (JSON StreamDeserializer, quick-xml Reader, csv row iterator)
4. **ANSI Colorization**: Raw ANSI codes (no color crate dependency), respects NO_COLOR env var and TTY detection
5. **Error Handling**: `anyhow` for CLI, `thiserror` for library errors

### JQ Query Engine

The query engine implements a substantial subset of jq:

- **Paths**: `.foo`, `.a.b.c`, `.[0]`, `.[]`, `.[2:5]`
- **Operators**: `+`, `-`, `*`, `/`, `%`, `==`, `!=`, `<`, `>`, `<=`, `>=`, `and`, `or`, `not`, `//`
- **Construction**: `[expr]`, `{key: expr}`, `"string \(interp)"`
- **Control flow**: `if-then-elif-else-end`, `try-catch`, `as $var |`, `reduce`, `foreach`, `label-break`
- **Functions**: `def name(params): body;`, 80+ built-in functions
- **Assignment**: `=`, `|=`, `+=`, `-=`, `*=`, `/=`, `%=`, `//=`
- **Format strings**: `@base64`, `@base64d`, `@uri`, `@csv`, `@tsv`, `@sh`, `@html`, `@json`, `@text`
- **Missing keys return null** (jq-compatible behavior)
- See [docs/compatibility.md](docs/compatibility.md) for known divergences and format-mapping details.

## Testing Strategy

### Unit & Integration Tests
- Parser tests for each format (YAML, JSON, XML, TOML, CSV, TSV)
- Lexer, parser, evaluator tests for the JQ engine
- Built-in function tests
- Color output tests
- Stream processing tests
- CLI integration tests in `tests/cli_tests.rs` and query tests in `src/query/mod.rs`

### Running Tests
```bash
cargo test             # Run all unit and integration tests
cargo clippy --all-targets -- -D warnings
cargo build --release  # Build optimized binary
```

## Roadmap

- [ ] Arbitrary-precision number support (`arbitrary_precision` / big-int handling)
- [ ] Module system (`import` / `include`) and streaming `input` / `inputs`
- [ ] Comment-preserving in-place editing (`-i`) for YAML
- [ ] Streaming support for YAML and TOML
- [ ] Published benchmarks comparing `qf` against `jq` and `yq`
- [ ] Pre-built release binaries and package manager distribution (Cargo, Homebrew)

## Related Projects

- **xmlshift**: Existing XML parser for telecom data
- **jq**: JSON processor (query syntax inspiration)
- **mikefarah/yq**: Current tool to improve upon
- **yj**: Simple YAML/JSON converter

## Development Workflow

```bash
# Build
cargo build --release

# Run tests
cargo test

# Install locally
cargo install --path .

# Quick test
echo '{"name":"world"}' | qf '"Hello \(.name)"'
```
