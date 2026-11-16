use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;
use tempfile::TempDir;

#[test]
fn yaml_json_roundtrip_multidoc_and_merge_keys() {
    let dir = TempDir::new().unwrap();

    // 1. YAML -> JSON -> YAML round-trip
    let yaml_file = dir.path().join("config.yaml");
    std::fs::write(
        &yaml_file,
        "name: qf\nversion: 1\ntags:\n  - fast\n  - universal\n",
    )
    .unwrap();

    let json_out = cargo_bin_cmd!("qf")
        .args(["-o", "json", "-c", "."])
        .arg(&yaml_file)
        .output()
        .unwrap();
    assert!(json_out.status.success(), "yaml->json failed: {json_out:?}");
    let json_str = String::from_utf8(json_out.stdout).unwrap();
    let parsed: Value = serde_json::from_str(json_str.trim()).unwrap();
    assert_eq!(parsed["name"], "qf");
    assert_eq!(parsed["tags"], serde_json::json!(["fast", "universal"]));

    let roundtrip_yaml = cargo_bin_cmd!("qf")
        .args(["-p", "json", "-o", "yaml", "."])
        .write_stdin(json_str.as_bytes())
        .output()
        .unwrap();
    assert!(roundtrip_yaml.status.success());
    let yaml_str = String::from_utf8(roundtrip_yaml.stdout).unwrap();
    assert!(yaml_str.contains("name: qf"));
    assert!(yaml_str.contains("- fast"));

    // 2. Multi-document YAML
    let multidoc = dir.path().join("multi.yaml");
    std::fs::write(&multidoc, "---\nid: 1\n---\nid: 2\n").unwrap();
    let multi_out = cargo_bin_cmd!("qf")
        .args(["-o", "json", "-c", ".[].id"])
        .arg(&multidoc)
        .output()
        .unwrap();
    assert!(multi_out.status.success());
    assert_eq!(String::from_utf8(multi_out.stdout).unwrap(), "1\n2\n");

    // 3. YAML << merge keys
    let merge_yaml = dir.path().join("merge.yaml");
    std::fs::write(
        &merge_yaml,
        "defaults: &defs\n  timeout: 30\n  retries: 3\nservice:\n  <<: *defs\n  retries: 5\n",
    )
    .unwrap();
    let merge_out = cargo_bin_cmd!("qf")
        .args(["-o", "json", "-c", ".service"])
        .arg(&merge_yaml)
        .output()
        .unwrap();
    assert!(merge_out.status.success());
    let merged: Value =
        serde_json::from_str(String::from_utf8(merge_out.stdout).unwrap().trim()).unwrap();
    assert_eq!(merged["timeout"], 30);
    assert_eq!(merged["retries"], 5);
}

#[test]
fn xml_json_roundtrip_repeated_attrs_text_and_root() {
    let dir = TempDir::new().unwrap();
    let xml_file = dir.path().join("catalog.xml");
    let xml_input = concat!(
        "<catalog version=\"2\">",
        "<book id=\"1\"><title>Rust</title></book>",
        "<book id=\"2\"><title>JQ</title></book>",
        "</catalog>"
    );
    std::fs::write(&xml_file, xml_input).unwrap();

    // XML -> JSON: preserves root tag, attributes, repeated elements, text-only nodes
    let json_out = cargo_bin_cmd!("qf")
        .args(["-o", "json", "-c", "."])
        .arg(&xml_file)
        .output()
        .unwrap();
    assert!(json_out.status.success(), "xml->json failed: {json_out:?}");
    let json_str = String::from_utf8(json_out.stdout).unwrap();
    let val: Value = serde_json::from_str(json_str.trim()).unwrap();

    assert_eq!(val["catalog"]["@version"], "2");
    assert_eq!(val["catalog"]["book"][0]["@id"], "1");
    assert_eq!(val["catalog"]["book"][0]["title"], "Rust");
    assert_eq!(val["catalog"]["book"][1]["@id"], "2");
    assert_eq!(val["catalog"]["book"][1]["title"], "JQ");

    // JSON -> XML round-trip
    let xml_out = cargo_bin_cmd!("qf")
        .args(["-p", "json", "-o", "xml", "."])
        .write_stdin(json_str.as_bytes())
        .output()
        .unwrap();
    assert!(xml_out.status.success(), "json->xml failed: {xml_out:?}");
    let xml_rt = String::from_utf8(xml_out.stdout).unwrap();
    assert!(xml_rt.contains("<catalog"));
    assert!(xml_rt.contains("version=\"2\""));
    assert!(xml_rt.contains("<title>Rust</title>"));
    assert!(xml_rt.contains("<title>JQ</title>"));

    // Re-parse round-tripped XML back to JSON and verify structural equality
    let json_rt_out = cargo_bin_cmd!("qf")
        .args(["-p", "xml", "-o", "json", "-c", "."])
        .write_stdin(xml_rt.as_bytes())
        .output()
        .unwrap();
    assert!(json_rt_out.status.success());
    let val_rt: Value =
        serde_json::from_str(String::from_utf8(json_rt_out.stdout).unwrap().trim()).unwrap();
    assert_eq!(val_rt, val);
}

#[test]
fn toml_scalar_array_null_omission_and_datetime_preservation() {
    let dir = TempDir::new().unwrap();
    let toml_file = dir.path().join("settings.toml");
    std::fs::write(
        &toml_file,
        "title = \"qf\"\nports = [8080, 8443]\ncreated = 2026-01-15T10:30:00Z\n",
    )
    .unwrap();

    // Scalar query to TOML
    let scalar_out = cargo_bin_cmd!("qf")
        .args([".title"])
        .arg(&toml_file)
        .output()
        .unwrap();
    assert!(scalar_out.status.success());
    assert_eq!(String::from_utf8(scalar_out.stdout).unwrap().trim(), "qf");

    // Array query to TOML
    let array_out = cargo_bin_cmd!("qf")
        .args([".ports"])
        .arg(&toml_file)
        .output()
        .unwrap();
    assert!(array_out.status.success());
    let array_str = String::from_utf8(array_out.stdout).unwrap();
    assert!(array_str.contains("8080"));
    assert!(array_str.contains("8443"));

    // Null key omission & datetime preservation when writing TOML
    let updated_out = cargo_bin_cmd!("qf")
        .args(["-o", "toml", ". + {extra: null, active: true}"])
        .arg(&toml_file)
        .output()
        .unwrap();
    assert!(
        updated_out.status.success(),
        "toml update failed: {updated_out:?}"
    );
    let updated_str = String::from_utf8(updated_out.stdout).unwrap();
    assert!(
        !updated_str.contains("extra"),
        "null key should be omitted in TOML output: {updated_str}"
    );
    assert!(updated_str.contains("active = true"));
    assert!(
        updated_str.contains("2026-01-15T10:30:00Z"),
        "datetime should be preserved in TOML output: {updated_str}"
    );
}

#[test]
fn csv_and_tsv_union_headers_and_roundtrip() {
    let dir = TempDir::new().unwrap();

    // CSV -> JSON -> CSV
    let csv_file = dir.path().join("users.csv");
    std::fs::write(&csv_file, "name,age\nAlice,30\nBob,25\n").unwrap();

    let json_out = cargo_bin_cmd!("qf")
        .args(["-o", "json", "-c", "."])
        .arg(&csv_file)
        .output()
        .unwrap();
    assert!(json_out.status.success());
    let parsed: Value =
        serde_json::from_str(String::from_utf8(json_out.stdout).unwrap().trim()).unwrap();
    assert_eq!(parsed[0]["name"], "Alice");
    assert_eq!(parsed[0]["age"], "30");

    // Union headers across heterogeneous objects when outputting CSV
    let union_csv = cargo_bin_cmd!("qf")
        .args(["-p", "json", "-o", "csv", "."])
        .write_stdin(r#"[{"a":1,"b":2},{"b":3,"c":4}]"#)
        .output()
        .unwrap();
    assert!(union_csv.status.success());
    let csv_str = String::from_utf8(union_csv.stdout).unwrap();
    let mut lines = csv_str.lines();
    assert_eq!(lines.next().unwrap(), "a,b,c");
    assert_eq!(lines.next().unwrap(), "1,2,");
    assert_eq!(lines.next().unwrap(), ",3,4");

    // TSV round-trip
    let tsv_file = dir.path().join("items.tsv");
    std::fs::write(&tsv_file, "id\tname\n1\talpha\n2\tbeta\n").unwrap();
    let tsv_rt = cargo_bin_cmd!("qf")
        .args(["-o", "tsv", "map(.name |= ascii_upcase)"])
        .arg(&tsv_file)
        .output()
        .unwrap();
    assert!(tsv_rt.status.success());
    let tsv_str = String::from_utf8(tsv_rt.stdout).unwrap();
    assert_eq!(tsv_str, "id\tname\n1\tALPHA\n2\tBETA\n");
}

#[test]
fn stream_mode_json_array_ndjson_xml_and_csv() {
    let dir = TempDir::new().unwrap();

    // 1. --stream on JSON array
    let json_arr = dir.path().join("arr.json");
    std::fs::write(&json_arr, r#"[{"id":1},{"id":2},{"id":3}]"#).unwrap();
    let out_json = cargo_bin_cmd!("qf")
        .args(["--stream", ".id"])
        .arg(&json_arr)
        .output()
        .unwrap();
    assert!(out_json.status.success());
    assert_eq!(String::from_utf8(out_json.stdout).unwrap(), "1\n2\n3\n");

    // 2. --stream on NDJSON (.jsonl)
    let jsonl = dir.path().join("events.jsonl");
    std::fs::write(&jsonl, "{\"ev\":\"a\"}\n{\"ev\":\"b\"}\n").unwrap();
    let out_jsonl = cargo_bin_cmd!("qf")
        .args(["--stream", "-r", ".ev"])
        .arg(&jsonl)
        .output()
        .unwrap();
    assert!(out_jsonl.status.success());
    assert_eq!(String::from_utf8(out_jsonl.stdout).unwrap(), "a\nb\n");

    // 3. --stream on XML
    let xml_file = dir.path().join("items.xml");
    std::fs::write(
        &xml_file,
        "<items><item><name>one</name></item><item><name>two</name></item></items>",
    )
    .unwrap();
    let out_xml = cargo_bin_cmd!("qf")
        .args(["--stream", "-r", ".name"])
        .arg(&xml_file)
        .output()
        .unwrap();
    assert!(out_xml.status.success(), "stream xml failed: {out_xml:?}");
    assert_eq!(String::from_utf8(out_xml.stdout).unwrap(), "one\ntwo\n");

    // 4. --stream on CSV
    let csv_file = dir.path().join("rows.csv");
    std::fs::write(&csv_file, "user,score\nana,10\nben,20\n").unwrap();
    let out_csv = cargo_bin_cmd!("qf")
        .args(["--stream", "-r", ".user"])
        .arg(&csv_file)
        .output()
        .unwrap();
    assert!(out_csv.status.success(), "stream csv failed: {out_csv:?}");
    assert_eq!(String::from_utf8(out_csv.stdout).unwrap(), "ana\nben\n");
}
