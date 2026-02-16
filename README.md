# qf

A fast, Rust-based universal data format query tool. Think [yq](https://github.com/mikefarah/yq) but faster and with multi-format support.

## Installation

```bash
cargo install --path .
```

## Usage

```bash
qf [OPTIONS] [QUERY] [FILE]
```

### Arguments

| Argument | Description |
|----------|-------------|
| `QUERY`  | Path query expression (default: `.` returns whole document) |
| `FILE`   | Input file (reads from stdin if omitted) |

### Options

| Flag | Description |
|------|-------------|
| `-p, --input-format <FORMAT>` | Force input format (`yaml`, `json`) |
| `-o, --output-format <FORMAT>` | Output format (`yaml`, `json`). Defaults to input format |
| `-c, --compact` | Compact output (no pretty printing) |
| `-r, --raw` | Raw string output (no quotes) |

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
```

### Query Syntax

| Pattern | Description |
|---------|-------------|
| `.` | Identity — return whole document |
| `.key` | Object key lookup |
| `.a.b.c` | Nested key traversal |
| `[0]` | Array index |
| `[]` | Array iterator — map remaining path over all elements |
| `.items[0].name` | Combined path |
| `.items[].name` | Extract field from every array element |

## Supported Formats

| Format | Read | Write |
|--------|------|-------|
| YAML   | Yes  | Yes   |
| JSON   | Yes  | Yes   |
| XML    | Planned | Planned |
| TOML   | Planned | Planned |
| CSV    | Planned | Planned |

## Building

```bash
cargo build --release
```

## Testing

```bash
cargo test
```

## License

MIT
