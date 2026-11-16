use qf::query::query;
use serde_json::Value;

struct ConformanceCase {
    name: &'static str,
    program: &'static str,
    input: &'static str,
    expected: &'static [&'static str],
}

const CASES: &[ConformanceCase] = &[
    // --- Paths, slicing (including UTF-8), optional ?, .., del, |= empty ---
    ConformanceCase {
        name: "nested_path_and_index",
        program: ".a.b[1]",
        input: r#"{"a":{"b":[10,20,30]}}"#,
        expected: &["20"],
    },
    ConformanceCase {
        name: "array_slice_positive_and_negative",
        program: ".[1:3], .[-2:]",
        input: "[10, 20, 30, 40, 50]",
        expected: &["[20,30]", "[40,50]"],
    },
    ConformanceCase {
        name: "string_slice_utf8_chars",
        program: ".[1:3], .[0:2]",
        input: r#""héllo🦀""#,
        expected: &[r#""él""#, r#""hé""#],
    },
    ConformanceCase {
        name: "string_slice_emoji_boundaries",
        program: ".[1:3]",
        input: r#""a🦀🚀b""#,
        expected: &[r#""🦀🚀""#],
    },
    ConformanceCase {
        name: "optional_suppresses_type_errors",
        program: ".foo?, .[0]?",
        input: "42",
        expected: &[],
    },
    ConformanceCase {
        name: "recursive_descent_numbers",
        program: "[.. | numbers]",
        input: r#"{"a":1,"b":[2,"x",{"c":3}]}"#,
        expected: &["[1,2,3]"],
    },
    ConformanceCase {
        name: "del_object_key_and_nested_path",
        program: "del(.b), del(.c.x)",
        input: r#"{"a":1,"b":2,"c":{"x":10,"y":20}}"#,
        expected: &[
            r#"{"a":1,"c":{"x":10,"y":20}}"#,
            r#"{"a":1,"b":2,"c":{"y":20}}"#,
        ],
    },
    ConformanceCase {
        name: "del_array_index",
        program: "del(.[1])",
        input: "[10, 20, 30]",
        expected: &["[10,30]"],
    },
    ConformanceCase {
        name: "update_assign_empty_deletes_elements",
        program: ".[] |= if . > 2 then empty else . end",
        input: "[1, 2, 3, 4]",
        expected: &["[1,2]"],
    },
    ConformanceCase {
        name: "update_assign_empty_object_field",
        program: ".a |= empty",
        input: r#"{"a":1,"b":2}"#,
        expected: &[r#"{"b":2}"#],
    },
    // --- Generators, select with multi-output predicates, if/elif/else, reduce, foreach ---
    ConformanceCase {
        name: "range_two_and_three_args",
        program: "range(1; 4), range(0; 6; 2)",
        input: "null",
        expected: &["1", "2", "3", "0", "2", "4"],
    },
    ConformanceCase {
        name: "cartesian_generator_multiplication",
        program: "(1, 2) * (10, 20)",
        input: "null",
        expected: &["10", "20", "20", "40"],
    },
    ConformanceCase {
        name: "select_with_multi_output_predicate",
        program: "[.[] | select(. == (2, 4))]",
        input: "[1, 2, 3, 4, 5]",
        expected: &["[2,4]"],
    },
    ConformanceCase {
        name: "if_elif_else_branches",
        program: "[.[] | if . > 0 then \"pos\" elif . < 0 then \"neg\" else \"zero\" end]",
        input: "[3, 0, -2]",
        expected: &[r#"["pos","zero","neg"]"#],
    },
    ConformanceCase {
        name: "reduce_sum",
        program: "reduce .[] as $x (0; . + $x)",
        input: "[1, 2, 3, 4, 5]",
        expected: &["15"],
    },
    ConformanceCase {
        name: "foreach_running_total_with_extract",
        program: "[foreach .[] as $x (0; . + $x; [$x, .])]",
        input: "[10, 20, 30]",
        expected: &["[[10,10],[20,30],[30,60]]"],
    },
    // --- def closures & recursion, label/break, try/catch, //, and/or short-circuiting ---
    ConformanceCase {
        name: "def_with_filter_arg_and_closure",
        program: "def outer(x): def inner: . + x; inner; outer(5)",
        input: "10",
        expected: &["15"],
    },
    ConformanceCase {
        name: "def_recursion_factorial",
        program: "def fact: if . <= 1 then 1 else . * ((. - 1) | fact) end; fact",
        input: "5",
        expected: &["120"],
    },
    ConformanceCase {
        name: "label_and_break_early_exit",
        program: "[label $out | .[] | if . == 3 then break $out else . end]",
        input: "[1, 2, 3, 4, 5]",
        expected: &["[1,2]"],
    },
    ConformanceCase {
        name: "try_catch_non_string_error",
        program: "try error({code: 42, msg: \"bad\"}) catch .code",
        input: "null",
        expected: &["42"],
    },
    ConformanceCase {
        name: "try_catch_runtime_error",
        program: "try (1 / 0) catch \"div0\"",
        input: "null",
        expected: &[r#""div0""#],
    },
    ConformanceCase {
        name: "alternative_operator_suppresses_error",
        program: "error(\"boom\") // \"fallback\"",
        input: "null",
        expected: &[r#""fallback""#],
    },
    ConformanceCase {
        name: "and_or_short_circuiting",
        program: "(false and error(\"boom\")), (true or error(\"boom\"))",
        input: "null",
        expected: &["false", "true"],
    },
    // --- {$a} object destructuring shorthand, @format "str \(interp)", quoted field chains ---
    ConformanceCase {
        name: "object_construction_shorthand_var_and_field",
        program: "10 as $a | { $a, b }",
        input: r#"{"b": 20, "c": 30}"#,
        expected: &[r#"{"a":10,"b":20}"#],
    },
    ConformanceCase {
        name: "object_destructuring_pattern",
        program: ". as {$a, $b} | $a + $b",
        input: r#"{"a": 7, "b": 8}"#,
        expected: &["15"],
    },
    ConformanceCase {
        name: "format_string_interpolation",
        program: r#"@uri "https://example.com/?q=\(.q)""#,
        input: r#"{"q": "hello world"}"#,
        expected: &[r#""https://example.com/?q=hello%20world""#],
    },
    ConformanceCase {
        name: "quoted_field_chains",
        program: r#".a."b-c"."d e""#,
        input: r#"{"a":{"b-c":{"d e":99}}}"#,
        expected: &["99"],
    },
    // --- Builtins ---
    ConformanceCase {
        name: "values_filters_nulls",
        program: "[.[] | values]",
        input: "[1, null, false, 0, null, \"ok\"]",
        expected: &["[1,false,0,\"ok\"]"],
    },
    ConformanceCase {
        name: "map_values_empty",
        program: "map_values(empty), map_values(if . == null then empty else . + 1 end)",
        input: r#"{"a":1,"b":null,"c":3}"#,
        expected: &["{}", r#"{"a":2,"c":4}"#],
    },
    ConformanceCase {
        name: "to_entries_from_entries_with_entries",
        program: "to_entries, (to_entries | from_entries), with_entries(.value += 10)",
        input: r#"{"a":1,"b":2}"#,
        expected: &[
            r#"[{"key":"a","value":1},{"key":"b","value":2}]"#,
            r#"{"a":1,"b":2}"#,
            r#"{"a":11,"b":12}"#,
        ],
    },
    ConformanceCase {
        name: "sort_unique_group_by",
        program: "sort, unique, ([{k:1,v:\"a\"},{k:2,v:\"b\"},{k:1,v:\"c\"}] | group_by(.k))",
        input: "[3, 1, 2, 1, 3]",
        expected: &[
            "[1,1,2,3,3]",
            "[1,2,3]",
            r#"[[{"k":1,"v":"a"},{"k":1,"v":"c"}],[{"k":2,"v":"b"}]]"#,
        ],
    },
    ConformanceCase {
        name: "sub_with_flags_and_named_captures",
        program: r#"sub("(?<word>[a-z]+)"; "[\(.word)]"; "i")"#,
        input: r#""Hello world""#,
        expected: &[r#""[Hello] world""#],
    },
    ConformanceCase {
        name: "gsub_with_case_insensitive_flag",
        program: r#"gsub("foo"; "bar"; "i")"#,
        input: r#""Foo and fOO""#,
        expected: &[r#""bar and bar""#],
    },
    ConformanceCase {
        name: "walk_recursive_transform",
        program: "walk(if type == \"number\" then . * 2 else . end)",
        input: r#"{"a":1,"b":[2,{"c":3}]}"#,
        expected: &[r#"{"a":2,"b":[4,{"c":6}]}"#],
    },
    ConformanceCase {
        name: "tostream_and_fromstream_roundtrip",
        program: "[tostream], fromstream(tostream)",
        input: r#"{"a":1,"b":[2,{}]}"#,
        expected: &[
            r#"[[["a"],1],[["b",0],2],[["b",1],{}],[["b",1]],[["b"]]]"#,
            r#"{"a":1,"b":[2,{}]}"#,
        ],
    },
    ConformanceCase {
        name: "splits_regex",
        program: r#"[splits(",\\s*")]"#,
        input: r#""a, b,c,  d""#,
        expected: &[r#"["a","b","c","d"]"#],
    },
    ConformanceCase {
        name: "abs_numbers",
        program: "map(abs)",
        input: "[-5, 0, 3.5, -2.25]",
        expected: &["[5,0,3.5,2.25]"],
    },
    ConformanceCase {
        name: "objects_and_numbers_type_selectors",
        program: "[.[] | objects], [.[] | numbers]",
        input: r#"[1, {"a":1}, "s", 2.5, null, []]"#,
        expected: &[r#"[{"a":1}]"#, "[1,2.5]"],
    },
    ConformanceCase {
        name: "any_and_all_predicates",
        program: "any(. > 2), all(. > 0), all(. > 2)",
        input: "[1, 2, 3]",
        expected: &["true", "true", "false"],
    },
    ConformanceCase {
        name: "isempty_builtin",
        program: "isempty(empty), isempty(1, 2)",
        input: "null",
        expected: &["true", "false"],
    },
    ConformanceCase {
        name: "in_builtin",
        program: "(2 | IN(1, 2, 3)), (5 | IN(1, 2, 3))",
        input: "null",
        expected: &["true", "false"],
    },
    ConformanceCase {
        name: "format_csv",
        program: "@csv",
        input: r#"["a", "b,c", 42, null]"#,
        expected: &[r#""\"a\",\"b,c\",42,""#],
    },
    ConformanceCase {
        name: "format_sh",
        program: "@sh",
        input: r#"["echo", "it's ok"]"#,
        expected: &[r#""'echo' 'it'\\''s ok'""#],
    },
    ConformanceCase {
        name: "format_base64_and_base64d",
        program: "@base64, (@base64 | @base64d)",
        input: r#""hello, qf!""#,
        expected: &[r#""aGVsbG8sIHFmIQ==""#, r#""hello, qf!""#],
    },
    // --- Number fidelity & object key order preservation ---
    ConformanceCase {
        name: "large_integer_fidelity",
        program: ". + 0, . * 1",
        input: "9007199254740993",
        expected: &["9007199254740993", "9007199254740993"],
    },
    ConformanceCase {
        name: "integer_and_float_arithmetic",
        program: "1 + 2, 1 + 2.5, 5 - 3, 6 * 7",
        input: "null",
        expected: &["3", "3.5", "2", "42"],
    },
    ConformanceCase {
        name: "object_key_order_keys_unsorted",
        program: "keys_unsorted",
        input: r#"{"z":1,"a":2,"m":3}"#,
        expected: &[r#"["z","a","m"]"#],
    },
    // --- Additional conformance cases for 50+ coverage ---
    ConformanceCase {
        name: "comma_and_pipe_precedence",
        program: "(.a, .b) | . * 10",
        input: r#"{"a":2,"b":3}"#,
        expected: &["20", "30"],
    },
    ConformanceCase {
        name: "array_construct_from_generator",
        program: "[.[] | . * .]",
        input: "[1, 2, 3, 4]",
        expected: &["[1,4,9,16]"],
    },
    ConformanceCase {
        name: "object_construct_dynamic_key",
        program: "{(.k): .v}",
        input: r#"{"k":"dyn","v":123}"#,
        expected: &[r#"{"dyn":123}"#],
    },
    ConformanceCase {
        name: "object_merge_with_plus",
        program: ".a + .b",
        input: r#"{"a":{"x":1,"y":2},"b":{"y":9,"z":3}}"#,
        expected: &[r#"{"x":1,"y":9,"z":3}"#],
    },
    ConformanceCase {
        name: "recursive_descent_paths",
        program: "[paths(scalars)]",
        input: r#"{"a":[1,{"b":2}]}"#,
        expected: &[r#"[["a",0],["a",1,"b"]]"#],
    },
    ConformanceCase {
        name: "getpath_and_setpath",
        program: "getpath([\"a\", 1]), setpath([\"a\", 1]; 99)",
        input: r#"{"a":[10,20]}"#,
        expected: &["20", r#"{"a":[10,99]}"#],
    },
    ConformanceCase {
        name: "has_and_in_builtins",
        program: "has(\"a\"), has(\"z\"), (\"a\" | in({\"a\":1}))",
        input: r#"{"a":null}"#,
        expected: &["true", "false", "true"],
    },
    ConformanceCase {
        name: "flatten_default_and_depth",
        program: "flatten, flatten(1)",
        input: "[1, [2, [3, 4]]]",
        expected: &["[1,2,3,4]", "[1,2,[3,4]]"],
    },
    ConformanceCase {
        name: "reverse_and_min_max",
        program: "reverse, min, max",
        input: "[3, 1, 4, 2]",
        expected: &["[2,4,1,3]", "1", "4"],
    },
    ConformanceCase {
        name: "split_and_join",
        program: r#"split(",") | join("-") "#,
        input: r#""one,two,three""#,
        expected: &[r#""one-two-three""#],
    },
    ConformanceCase {
        name: "tojson_and_fromjson",
        program: "tojson, (tojson | fromjson)",
        input: r#"{"k":[1,true]}"#,
        expected: &[r#""{\"k\":[1,true]}""#, r#"{"k":[1,true]}"#],
    },
    ConformanceCase {
        name: "ascii_downcase_and_upcase",
        program: "ascii_downcase, ascii_upcase",
        input: r#""Qf Tool""#,
        expected: &[r#""qf tool""#, r#""QF TOOL""#],
    },
    ConformanceCase {
        name: "floor_ceil_round",
        program: "floor, ceil, round",
        input: "3.7",
        expected: &["3", "4", "4"],
    },
    ConformanceCase {
        name: "contains_nested",
        program: r#"contains({"a":[2]})"#,
        input: r#"{"a":[1,2,3],"b":4}"#,
        expected: &["true"],
    },
    ConformanceCase {
        name: "env_variable_binding_across_pipe",
        program: ".x as $v | .items | map(. + $v)",
        input: r#"{"x":10,"items":[1,2,3]}"#,
        expected: &["[11,12,13]"],
    },
];

#[test]
fn jq_conformance_suite() {
    assert!(
        CASES.len() >= 50,
        "expected at least 50 conformance cases, found {}",
        CASES.len()
    );

    for case in CASES {
        let input: Value = serde_json::from_str(case.input)
            .unwrap_or_else(|e| panic!("[{}] invalid input JSON: {e}", case.name));
        let expected: Vec<Value> = case
            .expected
            .iter()
            .map(|s| {
                serde_json::from_str(s)
                    .unwrap_or_else(|e| panic!("[{}] invalid expected JSON `{s}`: {e}", case.name))
            })
            .collect();

        let actual = query(&input, case.program)
            .unwrap_or_else(|e| panic!("[{}] query `{}` failed: {e}", case.name, case.program));

        assert_eq!(
            actual, expected,
            "[{}] mismatch for program `{}` on input `{}`",
            case.name, case.program, case.input
        );
    }
}

#[test]
fn jq_conformance_preserves_object_key_order_on_serialize() {
    let input: Value = serde_json::from_str(r#"{"z":1,"a":2,"m":3}"#).unwrap();
    let results = query(&input, ". | {z, a, m, extra: 4}").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(
        serde_json::to_string(&results[0]).unwrap(),
        r#"{"z":1,"a":2,"m":3,"extra":4}"#
    );
}
