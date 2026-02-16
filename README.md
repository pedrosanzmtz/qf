# qf

A fast, Rust-based universal data format query tool with JQ-compatible syntax. Think [jq](https://github.com/jqlang/jq) + [yq](https://github.com/mikefarah/yq) but faster, unified, and multi-format.

## Installation

```bash
cargo install --path .
```

## Usage

```bash
qf [OPTIONS] [QUERY] [FILES...]
```

### Arguments

| Argument | Description |
|----------|-------------|
| `QUERY`  | JQ-compatible query expression (default: `.` returns whole document) |
| `FILES`  | Input file(s) (reads from stdin if omitted) |

### Options

| Flag | Description |
|------|-------------|
| `-p, --input-format <FORMAT>` | Force input format (`yaml`, `json`, `xml`, `toml`, `csv`, `tsv`) |
| `-o, --output-format <FORMAT>` | Output format (`yaml`, `json`, `xml`, `toml`, `csv`, `tsv`). Defaults to input format |
| `-i, --in-place` | Edit file in place |
| `-c, --compact` | Compact output (no pretty printing) |
| `-r, --raw` | Raw string output (no quotes) |
| `-s, --slurp` | Read all inputs into a JSON array |
| `-R, --raw-input` | Read raw input lines as strings |
| `-j, --join-output` | No newlines between outputs |
| `-n, --null-input` | Use null as input |
| `--color <MODE>` | Colorize output (`auto`, `always`, `never`) |
| `--no-color` | Disable colorized output |
| `--stream` | Stream mode: process records one at a time (for large files) |
| `--jsonl` | Read input as NDJSON/JSON Lines |

### Examples

```bash
# Query a YAML file
qf '.metadata.name' deployment.yaml

# Convert YAML to JSON
qf -o json '.' config.yaml

# Convert JSON to YAML
qf -o yaml '.' data.json

# Nested query with array index
qf '.spec.containers[0].image' deployment.yaml

# Iterate over array elements
qf '.spec.containers[].name' deployment.yaml

# Read from stdin
echo '{"name": "test"}' | qf '.name'

# Compact JSON output
qf -c '.' data.json

# Raw string output (no quotes)
qf -r '.metadata.name' deployment.yaml

# String interpolation / templates
echo '{"name":"world"}' | qf '"Hello \(.name)"'

# Select and filter
echo '[1,2,3,4,5]' | qf '[.[] | select(. > 3)]'

# Object construction
qf '{name: .metadata.name, image: .spec.containers[0].image}' deployment.yaml

# Map and transform
echo '[1,2,3]' | qf 'map(. * 2)'

# Sort, group, unique
echo '[3,1,2,1,3]' | qf 'sort | unique'

# Reduce
echo '[1,2,3,4,5]' | qf 'reduce .[] as $x (0; . + $x)'

# Merge two files
qf -s '.[0] * .[1]' base.yaml overlay.yaml

# In-place editing
qf -i '.version = "2.0"' config.yaml

# XML to JSON conversion
qf -p xml -o json '.' config.xml

# Stream large CSV files
qf --stream -p csv '.name' large.csv

# Process NDJSON/JSON Lines
qf --jsonl '.status' events.jsonl

# Regex matching
echo '{"email":"user@example.com"}' | qf '.email | test("@example")'

# Base64 encode
echo '"hello"' | qf '@base64'

# Keys and values
echo '{"a":1,"b":2}' | qf 'keys'

# To/from entries
echo '{"a":1,"b":2}' | qf 'to_entries | map(.value += 10) | from_entries'

# User-defined functions
echo '5' | qf 'def factorial: if . <= 1 then 1 else . * ((. - 1) | factorial) end; factorial'
```

### Query Syntax

qf implements a substantial subset of the [jq language](https://jqlang.github.io/jq/manual/):

#### Paths and Navigation

| Pattern | Description |
|---------|-------------|
| `.` | Identity -- return whole document |
| `.key` | Object key lookup |
| `.a.b.c` | Nested key traversal |
| `.[0]` | Array index |
| `.[-1]` | Negative index (from end) |
| `.[2:5]` | Array slice |
| `.[]` | Array/object iterator |
| `.key?` | Optional (suppress errors) |

#### Operators

| Operator | Description |
|----------|-------------|
| `\|` | Pipe (chain filters) |
| `,` | Multiple outputs |
| `+`, `-`, `*`, `/`, `%` | Arithmetic (also string concat, object merge) |
| `==`, `!=`, `<`, `>`, `<=`, `>=` | Comparison |
| `and`, `or`, `not` | Logical |
| `//` | Alternative (default for null) |

#### Construction

| Syntax | Description |
|--------|-------------|
| `[expr]` | Array construction |
| `{key: expr, ...}` | Object construction |
| `"Hello \(.name)"` | String interpolation |

#### Control Flow

| Syntax | Description |
|--------|-------------|
| `if C then T elif C2 then T2 else F end` | Conditionals |
| `try E catch F` | Error handling |
| `.expr as $var \| body` | Variable binding |
| `reduce E as $var (init; update)` | Reduce |
| `foreach E as $var (init; update; extract)` | Foreach |
| `def name(params): body;` | User-defined functions |
| `label $name \| break $name` | Label/break |

#### Assignment

| Operator | Description |
|----------|-------------|
| `=` | Set value |
| `\|=` | Update in place |
| `+=`, `-=`, `*=`, `/=`, `%=` | Arithmetic update |
| `//=` | Alternative assign |

#### Built-in Functions (80+)

**Type/Info**: `length`, `utf8bytelength`, `keys`, `values`, `has`, `in`, `type`, `builtins`, `infinite`, `nan`, `isinfinite`, `isnan`, `isnormal`

**Selection**: `select`, `empty`, `error`, `debug`

**Map/Transform**: `map`, `map_values`, `to_entries`, `from_entries`, `with_entries`, `transpose`, `add`, `any`, `all`, `flatten`, `range`

**Sorting**: `sort`, `sort_by`, `group_by`, `unique`, `unique_by`, `reverse`, `min`, `max`, `min_by`, `max_by`

**Searching**: `contains`, `inside`, `indices`, `index`, `rindex`

**Strings**: `tostring`, `tonumber`, `ascii_downcase`, `ascii_upcase`, `ltrimstr`, `rtrimstr`, `trim`, `split`, `join`, `startswith`, `endswith`, `ascii`, `explode`, `implode`

**Regex**: `test`, `match`, `capture`, `scan`, `sub`, `gsub`

**Iteration**: `first`, `last`, `nth`, `limit`, `recurse`, `until`, `while`, `repeat`

**Math**: `floor`, `ceil`, `round`, `fabs`, `sqrt`, `log`, `exp`, `pow`, `sin`, `cos`, `tan`, `asin`, `acos`, `atan`

**JSON**: `tojson`, `fromjson`

**Paths**: `path`, `paths`, `leaf_paths`, `getpath`, `setpath`, `delpaths`, `del`

**Format strings**: `@base64`, `@base64d`, `@uri`, `@csv`, `@tsv`, `@html`, `@json`, `@text`

**Other**: `env`, `not`, `input`, `inputs`

## Supported Formats

| Format | Read | Write | Stream |
|--------|------|-------|--------|
| YAML   | Yes  | Yes   | --     |
| JSON   | Yes  | Yes   | Yes    |
| NDJSON | Yes  | --    | Yes    |
| XML    | Yes  | Yes   | Yes    |
| TOML   | Yes  | Yes   | --     |
| CSV    | Yes  | Yes   | Yes    |
| TSV    | Yes  | Yes   | Yes    |

## Building

```bash
cargo build --release
```

## Testing

```bash
cargo test    # 165 tests
```

## License

MIT
