# jq Compatibility & Format Mapping Guide

This document describes the `jq` language subset supported by `qf`, known divergences from `jq`, format-specific mapping conventions, and `--stream` behavior.

---

## 1. Supported `jq` Syntax

`qf` implements a substantial subset of the [jq language](https://jqlang.github.io/jq/manual/):

### Paths and Navigation
- `.` (identity), `..` (recursive descent)
- `.key`, `.a.b.c`, `.key?` (optional field access)
- `.[0]`, `.[-1]`, `.[expr]`, `.[expr]?` (indexing)
- `.[m:n]`, `.[m:]`, `.[:n]` (array/string slicing)
- `.[]`, `.[]?` (array/object iteration)

### Operators
- `|` (pipe), `,` (multiple outputs)
- `+`, `-`, `*`, `/`, `%`, unary `-`
- `==`, `!=`, `<`, `<=`, `>`, `>=`
- `and`, `or`, `not`
- `//` (alternative operator)

### Construction & Literals
- `null`, `true`, `false`, numbers, strings
- `"Hello \(.name)"` (string interpolation)
- `[expr]` (array construction)
- `{key: expr, (expr): value, shorthand, @fmt}` (object construction)

### Control Flow & Bindings
- `if C then T elif C2 then T2 else F end`
- `try E catch F`, `try E`, `E?`
- `expr as $var | body` (including array/object destructuring patterns)
- `reduce expr as $var (init; update)`
- `foreach expr as $var (init; update; extract)` and 2-argument `foreach expr as $var (init; update)`
- `label $name | expr`, `break $name`
- `def name: body;` and `def name(p1; p2): body;` (user-defined functions)

### Assignment & Update
- `=` (plain assignment)
- `|=` (update assignment)
- `+=`, `-=`, `*=`, `/=`, `%=` (arithmetic update)
- `//=` (alternative assignment)

### Format Strings
- `@base64`, `@base64d`, `@uri`, `@csv`, `@tsv`, `@sh`, `@html`, `@json`, `@text`

---

## 2. Supported Built-in Functions

The following built-ins are registered in `src/query/builtins.rs`:

- **Type & Inspection**: `length`, `utf8bytelength`, `keys`, `keys_unsorted`, `values`, `objects`, `arrays`, `strings`, `numbers`, `booleans`, `nulls`, `iterables`, `scalars`, `has`, `in`, `IN`, `type`, `builtins`, `infinite`, `nan`, `isinfinite`, `isnan`, `isnormal`
- **Selection & Control**: `select`, `empty`, `isempty`, `error`, `debug`, `not`
- **Mapping & Transformation**: `map`, `map_values`, `to_entries`, `from_entries`, `with_entries`, `transpose`, `add`, `any`, `all`, `flatten`, `range`, `walk`
- **Sorting & Aggregation**: `sort`, `sort_by`, `group_by`, `unique`, `unique_by`, `reverse`, `min`, `max`, `min_by`, `max_by`
- **Searching**: `contains`, `inside`, `indices`, `index`, `rindex`
- **Strings**: `tostring`, `tonumber`, `ascii_downcase`, `ascii_upcase`, `ltrimstr`, `rtrimstr`, `trim`, `split`, `splits`, `join`, `startswith`, `endswith`, `ascii`, `explode`, `implode`
- **Regex**: `test`, `match`, `capture`, `scan`, `sub`, `gsub`
- **Iteration & Streams**: `first`, `last`, `nth`, `limit`, `recurse`, `until`, `while`, `repeat`, `tostream`, `fromstream`
- **Math**: `floor`, `ceil`, `round`, `fabs`, `abs`, `sqrt`, `log`, `log2`, `log10`, `exp`, `exp2`, `pow`, `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2`
- **Date/Time**: `now`, `todate`, `fromdate`
- **JSON**: `tojson`, `fromjson`
- **Paths**: `path`, `paths`, `leaf_paths`, `getpath`, `setpath`, `delpaths`, `del`
- **Environment & Literals**: `env`, `null`, `true`, `false`
- **Stubbed**: `input`, `inputs` (see [Known Divergences](#3-known-divergences--limitations-vs-jq))

---

## 3. Known Divergences & Limitations vs. `jq`

| Area | `jq` Behavior | `qf` Behavior |
|------|---------------|---------------|
| `input` / `inputs` | Reads the next value(s) from the input stream. | Stubbed: `input` always returns `null` and `inputs` produces no outputs. Use `-s` (`--slurp`), `--jsonl`, or `--stream` for multi-record inputs. |
| Modules (`import` / `include`) | Loads external `.jq` modules and metadata. | `import` and `include` are reserved tokens in the lexer, but module loading is not implemented in the parser/evaluator. |
| Number precision | Arbitrary-precision decimals (`decNumber`) when built with big-number support. | Uses `serde_json` without `arbitrary_precision`: integers are 64-bit (`i64`/`u64`) and floats are `f64`. Integers larger than 64 bits or needing exact decimal precision are not preserved as arbitrary-precision numbers. |
| `--stream` CLI flag | Emits `[path, leaf]` streaming events. | Performs record/element-level streaming (see [Section 5](#5---stream-behavior)). Use the `tostream` / `fromstream` built-ins for `[path, leaf]` representation inside queries. |
| CLI flags | Full `jq` CLI surface (`--slurpfile`, `--rawfile`, `--argfile`, `--unbuffered`, `--seq`, etc.). | Supports `-p`, `-o`, `-i`, `-c`, `-r`, `-s`, `-R`, `-j` (implies `-r`), `-n`, `-e`, `-f`, `-S`, `-C`, `-M`, `--color`, `--no-color`, `--arg`, `--argjson`, `--indent`, `--tab`, `--stream`, `--jsonl`. |

---

## 4. Format-Mapping Conventions

`qf` uses `serde_json::Value` (with `preserve_order` enabled) as its unified intermediate representation.

### JSON & NDJSON
- Object key order is preserved by default unless `-S` (`--sort-keys`) is specified.
- `--jsonl` reads newline-delimited JSON one line at a time.

### XML
- **Root key preserved**: The root element name becomes the single top-level object key (e.g. `<config><port>8080</port></config>` -> `{"config": {"port": "8080"}}`).
- **Attributes**: Element attributes are mapped to keys prefixed with `@` (e.g. `<item id="42"/>` -> `{"item": {"@id": "42"}}`).
- **Text content**:
  - Elements with only text become plain strings (`"8080"`). Empty elements become `null`.
  - Elements with attributes or child elements alongside text (mixed/attributed content) store the text under `"$text"` (e.g. `<msg level="info">ok</msg>` -> `{"msg": {"@level": "info", "$text": "ok"}}`).
- **Repeated siblings**: Repeated sibling elements with the same tag name are collected into an array.
- **XML output**: Serialization reverses these conventions (single-key object becomes the root element, `@attr` keys become attributes, `$text` becomes text content, arrays become repeated elements).

### CSV & TSV
- **String fields by default**: Headered CSV/TSV parses each row into an object whose field values are strings (no implicit number/boolean coercion by default).
- **Output**: CSV/TSV output requires an array of objects or an array of arrays. When writing an array of objects, the output header row is the **union of keys** across all rows in first-seen order; rows missing a key emit an empty cell.

### TOML
- **Query output**: When outputting TOML, objects are serialized as TOML tables, while scalar query results (`string`, `number`, `bool`, `null`) and arrays are formatted cleanly (scalars directly, arrays as pretty JSON) instead of failing because TOML has no scalar root.
- **Null keys**: TOML has no `null` value; object keys whose value is `null` are omitted when serializing to TOML.
- **Datetimes**: TOML datetimes are represented as strings; RFC3339 date+time strings are round-tripped back to native TOML datetimes.
- **Lossless `-i` editing**: In-place editing (`-i`) on a TOML file (when `-S` is not set) uses `toml_edit` to apply a structural diff, preserving comments, blank lines, key ordering, and formatting on untouched keys.

### YAML
- **Multi-document input**: Single-document YAML returns the document value directly; empty input returns `null`; multi-document YAML (`---`) returns a JSON array of documents.
- **Merge keys (`<<`)**: YAML 1.1 merge keys are resolved prior to query evaluation. Explicit keys in a mapping take precedence over merged keys; when `<<` is a sequence of mappings, earlier mappings take precedence over later ones.
- **In-place `-i` editing**: In-place editing on YAML re-serializes the transformed value via `serde_yaml` and does **not** preserve comments, anchors/aliases, or original formatting.

---

## 5. `--stream` Behavior

Unlike `jq --stream` (which decomposes a single JSON document into `[path, leaf]` pairs), `qf --stream` performs **record/element-level streaming** so large files can be processed incrementally without loading the entire document into memory:

| Format | `--stream` Unit |
|--------|-----------------|
| `json` | Each top-level JSON value, or each element of a top-level JSON array |
| `jsonl` (`--jsonl`) | Each newline-delimited JSON line |
| `xml` | Each direct child element under the XML root element |
| `csv` / `tsv` | Each data row (as an object keyed by the header row) |
| `yaml` / `toml` | Not supported (`--stream` returns a runtime error) |
