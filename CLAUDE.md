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

This project leverages experience from **xmlshift** - a Rust-based XML parser built for processing massive telecom data files (up to 63GB). Key learnings to apply:

- Config-driven architecture (similar to xmlshift's YAML config approach)
- Streaming parsers for large files
- Performance-critical path optimization
- CLI design for production use

## Core Features (MVP)

### Phase 1: Basic Format Support
- [x] YAML reading
- [x] JSON reading  
- [x] Format conversion: YAML ↔ JSON
- [x] Basic path queries (e.g., `.spec.containers[0].image`)
- [x] Pretty printing output

### Phase 2: Extended Formats
- [ ] XML support (leverage quick-xml experience from xmlshift)
- [ ] TOML support
- [ ] CSV/TSV support
- [ ] In-place editing (`-i` flag)

### Phase 3: Advanced Features
- [ ] JQ-compatible query syntax
- [ ] Streaming mode for large files
- [ ] Merge operations
- [ ] Template evaluation
- [ ] Colorized output

## Technical Architecture

### Dependencies
```toml
[dependencies]
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
serde_yaml = "0.9"
quick-xml = { version = "0.31", features = ["serialize"] }
toml = "0.8"
clap = { version = "4.4", features = ["derive"] }
anyhow = "1.0"
thiserror = "1.0"
```

### CLI Design
```bash
qf [OPTIONS] <QUERY> <FILE>

Options:
  -p, --input-format <FORMAT>   Input format [auto, yaml, json, xml, toml]
  -o, --output-format <FORMAT>  Output format [yaml, json, xml, toml]
  -i, --in-place                Edit file in place
  -c, --compact                 Compact output (no pretty print)
  -r, --raw                     Output raw strings (no quotes)
  
Examples:
  qf '.spec.containers[0]' deployment.yaml
  qf -p xml -o json '.root.element' config.xml
  qf -i '.version = "2.0"' config.yaml
```

### Core Modules

```
src/
├── main.rs           # CLI entry point
├── parser/
│   ├── mod.rs       # Parser trait and common logic
│   ├── yaml.rs      # YAML parser
│   ├── json.rs      # JSON parser
│   ├── xml.rs       # XML parser (leverage xmlshift patterns)
│   └── toml.rs      # TOML parser
├── query/
│   ├── mod.rs       # Query engine
│   ├── path.rs      # Path-based queries (.foo.bar[0])
│   └── expr.rs      # Expression evaluation (future)
├── transform/
│   ├── mod.rs       # Transform operations
│   └── convert.rs   # Format conversion
└── output/
    ├── mod.rs       # Output formatting
    └── pretty.rs    # Pretty printing
```

### Key Design Patterns

1. **Unified Data Model**: Use serde's `Value` type as intermediate representation
2. **Streaming Where Possible**: For large files, use streaming parsers
3. **Zero-Copy Parsing**: Minimize allocations using references and borrows
4. **Error Handling**: Use `anyhow` for CLI, `thiserror` for library errors

## Implementation Strategy

### Step 1: Basic YAML/JSON (Week 1)
```rust
// Start with simple path queries
qf '.metadata.name' deployment.yaml
qf -o json input.yaml > output.json
```

### Step 2: Add XML Support (Week 2)
```rust
// Leverage xmlshift XML parsing experience
qf -p xml '.configuration.setting[0]' config.xml
qf -p xml -o yaml vendor-config.xml
```

### Step 3: Query Engine (Week 3)
```rust
// Implement path traversal
.foo.bar          // Navigate objects
.items[0]         // Array indexing  
.items[]          // Array iteration
.items[].name     // Map over arrays
```

### Step 4: Polish & Performance (Week 4)
- Benchmarks against mikefarah/yq
- Memory profiling for large files
- Error message improvements
- Documentation

## Performance Goals

- **Large files** (>100MB): 5-10x faster than mikefarah/yq
- **Memory usage**: 50% less than mikefarah/yq through streaming
- **Startup time**: <50ms for simple queries
- **Binary size**: <10MB static binary

## Testing Strategy

### Unit Tests
- Parser tests for each format
- Query path evaluation
- Format conversion accuracy

### Integration Tests
```bash
test_data/
├── sample.yaml
├── sample.json
├── sample.xml
├── large-file.xml  # 1GB+ test file
└── edge-cases/
    ├── empty.yaml
    ├── nested-deep.json
    └── malformed.xml
```

### Benchmarks
```rust
// Compare against yq, jq, others
#[bench]
fn bench_large_yaml_query() { ... }

#[bench]
fn bench_yaml_to_json_conversion() { ... }
```

## Distribution

1. **GitHub Releases**: Pre-built binaries for Linux, macOS, Windows
2. **Cargo**: `cargo install qf`
3. **Homebrew**: Formula for macOS users
4. **Docker**: Containerized version for CI/CD

## Success Metrics

- [ ] Can replace mikefarah/yq in daily workflow
- [ ] Handles 10GB+ XML files without memory issues
- [ ] 1000+ GitHub stars (community validation)
- [ ] Used in production for telecom config processing

## Related Projects

- **xmlshift**: Your existing XML parser for telecom data
- **jq**: JSON processor (inspiration for query syntax)
- **mikefarah/yq**: Current tool to improve upon
- **yj**: Simple YAML/JSON converter

## Questions for Claude Code

1. Should we use `serde_json::Value` or create our own unified value type?
2. Best approach for streaming large XML files while maintaining query capability?
3. Query syntax: JQ-compatible vs simpler path-based?
4. How to handle namespaces in XML elegantly?

## Development Workflow

```bash
# Initial setup
cargo new qf
cd qf

# Add dependencies
cargo add serde serde_json serde_yaml clap anyhow

# Run tests
cargo test

# Build release
cargo build --release

# Install locally
cargo install --path .
```

## Notes

- This builds on xmlshift's proven patterns for large file handling
- Focus on real-world use cases (Kubernetes, Docker, telecom configs)
- Keep CLI simple and intuitive (learn from yq's UX)
- Performance is key differentiator - leverage Rust's strengths
