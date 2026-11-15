use std::io::Read;
use std::process::{Command, Stdio};

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn broken_pipe_exits_zero_without_panic() {
    let bin = assert_cmd::cargo::cargo_bin!("qf");
    let mut child = Command::new(bin)
        .args(["-n", "range(0; 10000)"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    // Read a single byte then drop stdout to trigger EPIPE on the writer.
    let mut stdout = child.stdout.take().unwrap();
    let mut buf = [0u8; 1];
    let _ = stdout.read(&mut buf);
    drop(stdout);

    let status = child.wait().unwrap();
    assert!(
        status.success(),
        "expected exit 0 on broken pipe, got {status}"
    );

    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(
        !stderr.contains("panicked"),
        "unexpected panic on stderr: {stderr}"
    );
}

#[test]
fn in_place_multiple_files_and_rejections() {
    let dir = TempDir::new().unwrap();
    let f1 = dir.path().join("a.json");
    let f2 = dir.path().join("b.json");
    std::fs::write(&f1, r#"{"a": 1}"#).unwrap();
    std::fs::write(&f2, r#"{"a": 2}"#).unwrap();

    cargo_bin_cmd!("qf")
        .args(["-i", ".a = 10"])
        .arg(&f1)
        .arg(&f2)
        .assert()
        .success();

    let v1: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&f1).unwrap()).unwrap();
    let v2: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&f2).unwrap()).unwrap();
    assert_eq!(v1["a"], 10);
    assert_eq!(v2["a"], 10);

    // Reject -i with different -o format
    cargo_bin_cmd!("qf")
        .args(["-i", "-o", "yaml", "."])
        .arg(&f1)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("qf: error:"));

    // Reject -i when query produces != 1 result
    let arr = dir.path().join("arr.json");
    std::fs::write(&arr, "[1, 2, 3]").unwrap();
    cargo_bin_cmd!("qf")
        .args(["-i", ".[]"])
        .arg(&arr)
        .assert()
        .code(5)
        .stderr(predicate::str::contains("qf: error:"));

    // Reject -i with no files
    cargo_bin_cmd!("qf")
        .args(["-i", "."])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("qf: error:"));
}

#[cfg(unix)]
#[test]
fn in_place_preserves_permissions_and_follows_symlinks() {
    use std::os::unix::fs::{symlink, PermissionsExt};

    let dir = TempDir::new().unwrap();
    let target = dir.path().join("target.json");
    let link = dir.path().join("link.json");
    std::fs::write(&target, r#"{"x": 1}"#).unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o640)).unwrap();
    symlink(&target, &link).unwrap();

    cargo_bin_cmd!("qf")
        .args(["-i", ".x = 42"])
        .arg(&link)
        .assert()
        .success();

    // Symlink must still be a symlink
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink(),
        "symlink should not be replaced by a regular file"
    );

    // Target file must be updated and retain 0o640 permissions
    let val: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&target).unwrap()).unwrap();
    assert_eq!(val["x"], 42);
    let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o640);
}

#[test]
fn multi_file_and_json_stream_and_slurp() {
    let dir = TempDir::new().unwrap();
    let f1 = dir.path().join("a.json");
    let f2 = dir.path().join("b.json");
    std::fs::write(&f1, r#"{"a": 1}"#).unwrap();
    std::fs::write(&f2, r#"{"a": 2}"#).unwrap();

    // Multiple files
    cargo_bin_cmd!("qf")
        .args([".a"])
        .arg(&f1)
        .arg(&f2)
        .assert()
        .success()
        .stdout("1\n2\n");

    // Multiple files with -s slurp
    cargo_bin_cmd!("qf")
        .args(["-s", "-c", "map(.a)"])
        .arg(&f1)
        .arg(&f2)
        .assert()
        .success()
        .stdout("[1,2]\n");

    // Multiple JSON values on stdin
    cargo_bin_cmd!("qf")
        .args([".a"])
        .write_stdin("{\"a\": 10}\n{\"a\": 20}\n")
        .assert()
        .success()
        .stdout("10\n20\n");

    // Multiple JSON values on stdin with -s
    cargo_bin_cmd!("qf")
        .args(["-s", "add"])
        .write_stdin("1\n2\n3\n")
        .assert()
        .success()
        .stdout("6\n");
}

#[test]
fn format_detection_jsonl_ndjson_unknown_ext_and_content_sniffing() {
    let dir = TempDir::new().unwrap();
    let jsonl = dir.path().join("items.jsonl");
    let ndjson = dir.path().join("items.ndjson");
    let unknown = dir.path().join("data.unknown");
    let no_ext = dir.path().join("plainfile");

    std::fs::write(&jsonl, "{\"x\":1}\n{\"x\":2}\n").unwrap();
    std::fs::write(&ndjson, "{\"x\":3}\n{\"x\":4}\n").unwrap();
    std::fs::write(&unknown, "key = \"hello\"\n").unwrap();
    std::fs::write(&no_ext, "<root><item>ok</item></root>").unwrap();

    cargo_bin_cmd!("qf")
        .args([".x"])
        .arg(&jsonl)
        .assert()
        .success()
        .stdout("1\n2\n");

    cargo_bin_cmd!("qf")
        .args([".x"])
        .arg(&ndjson)
        .assert()
        .success()
        .stdout("3\n4\n");

    cargo_bin_cmd!("qf")
        .args(["-r", ".key"])
        .arg(&unknown)
        .assert()
        .success()
        .stdout("hello\n");

    cargo_bin_cmd!("qf")
        .args(["-o", "json", "-c", "."])
        .arg(&no_ext)
        .assert()
        .success()
        .stdout(predicate::str::contains("ok"));

    // Stdin scalars
    cargo_bin_cmd!("qf")
        .args(["."])
        .write_stdin(r#""hi""#)
        .assert()
        .success()
        .stdout("\"hi\"\n");

    cargo_bin_cmd!("qf")
        .args([". + 1"])
        .write_stdin("41")
        .assert()
        .success()
        .stdout("42\n");

    cargo_bin_cmd!("qf")
        .args(["not"])
        .write_stdin("true")
        .assert()
        .success()
        .stdout("false\n");
}

#[test]
fn cli_flags_arg_argjson_exit_status_from_file_sort_color_indent_tab_join() {
    let dir = TempDir::new().unwrap();
    let query_file = dir.path().join("q.jq");
    let data_file = dir.path().join("d.json");
    std::fs::write(&query_file, "{b: $s, a: $n, orig: .x}").unwrap();
    std::fs::write(&data_file, r#"{"x": 5}"#).unwrap();

    // --arg, --argjson, -f, -S, -c
    cargo_bin_cmd!("qf")
        .args(["-S", "-c", "--arg", "s", "hi", "--argjson", "n", "7", "-f"])
        .arg(&query_file)
        .arg(&data_file)
        .assert()
        .success()
        .stdout("{\"a\":7,\"b\":\"hi\",\"orig\":5}\n");

    // -e / --exit-status: false/null -> 1, empty -> 4, truthy -> 0
    cargo_bin_cmd!("qf")
        .args(["-e", "-n", "false"])
        .assert()
        .code(1);
    cargo_bin_cmd!("qf")
        .args(["-e", "-n", "null"])
        .assert()
        .code(1);
    cargo_bin_cmd!("qf")
        .args(["-e", "-n", "empty"])
        .assert()
        .code(4);
    cargo_bin_cmd!("qf")
        .args(["-e", "-n", "true"])
        .assert()
        .success();

    // -C forces ANSI color even when piped; -M disables color
    cargo_bin_cmd!("qf")
        .args(["-C", "-n", "{a: 1}"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\x1b["));

    cargo_bin_cmd!("qf")
        .args(["-M", "-n", "{a: 1}"])
        .assert()
        .success()
        .stdout("{\n  \"a\": 1\n}\n");

    // --indent and --tab
    cargo_bin_cmd!("qf")
        .args(["--indent", "4", "-n", "{a: 1}"])
        .assert()
        .success()
        .stdout("{\n    \"a\": 1\n}\n");

    cargo_bin_cmd!("qf")
        .args(["--tab", "-n", "{a: 1}"])
        .assert()
        .success()
        .stdout("{\n\t\"a\": 1\n}\n");

    // -j implies -r (no quotes, no newline)
    cargo_bin_cmd!("qf")
        .args(["-j", "-n", r#""hello", "world""#])
        .assert()
        .success()
        .stdout("helloworld");
}

#[test]
fn error_ux_and_jq_exit_codes() {
    // Missing file -> exit 2, clean message
    cargo_bin_cmd!("qf")
        .args([".", "/nonexistent/path/to/file.json"])
        .assert()
        .code(2)
        .stderr(predicate::str::starts_with("qf: error: "))
        .stderr(predicate::str::contains("Caused by:").not());

    // Syntax/compile error -> exit 3
    cargo_bin_cmd!("qf")
        .args(["-n", ".["])
        .assert()
        .code(3)
        .stderr(predicate::str::starts_with("qf: error: "));

    // Runtime error -> exit 5
    cargo_bin_cmd!("qf")
        .args(["-n", r#"error("boom")"#])
        .assert()
        .code(5)
        .stderr(predicate::str::starts_with("qf: error: "));
}
