use std::{
    fmt::Write as _,
    fs,
    io::{Read, Write as _},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
};

const VALID_INPUT: &str = include_str!("fixtures/dtcg-2025.10/positive/colors/srgb-basic.json");
const INVALID_INPUT: &str =
    include_str!("fixtures/dtcg-2025.10/negative/numbers/number-string.json");
const VALID_FILE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/dtcg-2025.10/positive/colors/srgb-basic.json"
);
const INVALID_FILE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/dtcg-2025.10/negative/numbers/number-string.json"
);
const USAGE: &str = "Usage: dtcg-validator [--json] <FILE|->";

struct ValidationCase {
    file: &'static str,
    input: &'static str,
    valid: bool,
    diagnostic_path: Option<&'static str>,
}

const CASES: [ValidationCase; 3] = [
    ValidationCase {
        file: VALID_FILE,
        input: VALID_INPUT,
        valid: true,
        diagnostic_path: None,
    },
    ValidationCase {
        file: INVALID_FILE,
        input: INVALID_INPUT,
        valid: false,
        diagnostic_path: Some("/invalid-number/$value"),
    },
    ValidationCase {
        file: "malformed.json",
        input: "{\n",
        valid: false,
        diagnostic_path: Some(""),
    },
];

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);
        loop {
            let number = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "dtcg-validator-cli-{}-{number}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("cannot create test directory {}: {error}", path.display()),
            }
        }
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0) {
            if std::thread::panicking() {
                eprintln!("cannot remove test directory {}: {error}", self.0.display());
            } else {
                panic!("cannot remove test directory {}: {error}", self.0.display());
            }
        }
    }
}

fn run(args: &[&str], input: Option<&[u8]>, directory: Option<&Path>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_dtcg-validator"));
    command
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(directory) = directory {
        command.current_dir(directory);
    }
    let mut child = command.spawn().expect("the Cargo-built CLI must start");
    if let Some(input) = input {
        // Dropping the pipe here delivers EOF before waiting for the report.
        let written = child
            .stdin
            .take()
            .expect("stdin must be piped")
            .write_all(input);
        if let Err(error) = written {
            let output = child.wait_with_output().expect("the CLI must exit");
            panic!("cannot write CLI input: {error}; output: {output:?}");
        }
    }
    child.wait_with_output().expect("the CLI must exit")
}

fn assert_validation(output: &Output, case: &ValidationCase, json: bool) {
    let report = dtcg_validator::validate(case.input);
    assert_eq!(
        report.valid, case.valid,
        "core fixture result: {}",
        case.file
    );
    assert_eq!(report.valid, report.diagnostics.is_empty());
    if let Some(path) = case.diagnostic_path {
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.path == path),
            "{} must retain source path {path:?}: {report:?}",
            case.file
        );
    }
    assert_eq!(
        output.status.code(),
        Some(if case.valid { 0 } else { 1 }),
        "{} (json={json}): {output:?}",
        case.file
    );
    if json {
        // Exact shared-core bytes also require one final newline and no banners.
        let mut expected = serde_json::to_vec(&report).expect("the core report must serialize");
        expected.push(b'\n');
        assert_eq!(output.stdout, expected, "{}: {output:?}", case.file);
        assert!(output.stderr.is_empty(), "{}: {output:?}", case.file);
    } else {
        let mut expected = String::new();
        for diagnostic in &report.diagnostics {
            writeln!(
                expected,
                "{:?} [{}]: {}",
                diagnostic.path, diagnostic.code, diagnostic.message
            )
            .expect("writing to a String cannot fail");
        }
        assert!(output.stdout.is_empty(), "{}: {output:?}", case.file);
        assert_eq!(
            output.stderr,
            expected.as_bytes(),
            "{}: {output:?}",
            case.file
        );
    }
}

fn assert_error(output: &Output, expected_stderr: &str) {
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(
        output.stdout.is_empty(),
        "errors must not emit partial reports: {output:?}"
    );
    assert_eq!(output.stderr, expected_stderr.as_bytes(), "{output:?}");
}

#[test]
fn stdin_valid_invalid_and_malformed_documents_match_the_shared_core() {
    for case in &CASES {
        for json in [false, true] {
            let args: &[&str] = if json { &["--json", "-"] } else { &["-"] };
            let output = run(args, Some(case.input.as_bytes()), None);
            assert_validation(&output, case, json);
        }
    }
}

#[test]
fn files_valid_invalid_and_malformed_documents_match_the_shared_core() {
    let directory = TestDirectory::new();
    fs::write(directory.0.join("malformed.json"), CASES[2].input)
        .expect("the malformed input file must be written");
    for case in &CASES {
        for json in [false, true] {
            let output = if json {
                run(&["--json", case.file], None, Some(&directory.0))
            } else {
                run(&[case.file], None, Some(&directory.0))
            };
            assert_validation(&output, case, json);
        }
    }
}

#[test]
fn usage_errors_emit_only_stderr_and_exit_two() {
    let cases: &[(&[&str], &str)] = &[
        (&[], "missing input document; provide a file or -"),
        (&["--json"], "missing input document; provide a file or -"),
        (&["--"], "missing input document; provide a file or -"),
        (
            &["--json", "--"],
            "missing input document; provide a file or -",
        ),
        (
            &[VALID_FILE, INVALID_FILE],
            "expected exactly one input document; received multiple operands",
        ),
        (
            &["--json", VALID_FILE, INVALID_FILE],
            "expected exactly one input document; received multiple operands",
        ),
        (
            &["--", VALID_FILE, "--json"],
            "expected exactly one input document; received multiple operands",
        ),
        (&["--unknown"], "unknown option \"--unknown\""),
        (
            &["--json", "--unknown", "-"],
            "unknown option \"--unknown\"",
        ),
        (&["-", "--unknown"], "unknown option \"--unknown\""),
        (&["-tokens.json"], "unknown option \"-tokens.json\""),
    ];
    for (args, reason) in cases {
        let expected = format!(
            "dtcg-validator: {reason}\n\n{USAGE}\nTry 'dtcg-validator --help' for more information.\n"
        );
        assert_error(&run(args, None, None), &expected);
    }
}

#[test]
fn file_io_errors_emit_only_stderr_even_in_json_mode() {
    let directory = TestDirectory::new();
    fs::write(directory.0.join("non-utf8.json"), [0xff])
        .expect("the non-UTF-8 input file must be written");
    for file in ["missing.json", ".", "non-utf8.json"] {
        let error = fs::read_to_string(directory.0.join(file))
            .expect_err("each I/O test path must be unreadable as UTF-8 text");
        let expected = format!("dtcg-validator: cannot read {file:?}: {error}\n");
        for json in [false, true] {
            let output = if json {
                run(&["--json", file], None, Some(&directory.0))
            } else {
                run(&[file], None, Some(&directory.0))
            };
            assert_error(&output, &expected);
        }
    }
}

#[test]
fn stdin_io_errors_emit_no_partial_json_report() {
    let bytes = [0xff];
    let error = bytes
        .as_slice()
        .read_to_string(&mut String::new())
        .expect_err("non-UTF-8 stdin must be an I/O error, not a document error");
    let expected = format!("dtcg-validator: cannot read standard input: {error}\n");
    for args in [&["-"][..], &["--json", "-"][..]] {
        assert_error(&run(args, Some(&bytes), None), &expected);
    }
}

#[test]
fn help_aliases_print_the_same_help_only_to_stdout() {
    let long = run(&["--help"], None, None);
    let short = run(&["-h"], None, None);
    for output in [&long, &short] {
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let help = std::str::from_utf8(&output.stdout).expect("help must be UTF-8");
        assert!(help.contains(USAGE), "{help}");
        assert!(help.contains("--json"), "{help}");
        assert!(help.contains("standard input"), "{help}");
        assert!(help.ends_with('\n'), "{help}");
    }
    assert_eq!(long.stdout, short.stdout);
}

#[test]
fn version_aliases_print_the_package_version_only_to_stdout() {
    let expected = concat!("dtcg-validator ", env!("CARGO_PKG_VERSION"), "\n");
    for flag in ["--version", "-V"] {
        let output = run(&[flag], None, None);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, expected.as_bytes(), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
    }
}

#[test]
fn option_termination_accepts_flag_like_filenames_in_both_output_modes() {
    let directory = TestDirectory::new();
    for (file, base) in [
        ("--json", &CASES[0]),
        ("--help", &CASES[1]),
        ("--version", &CASES[0]),
        ("-tokens.json", &CASES[1]),
    ] {
        fs::write(directory.0.join(file), base.input).expect("the flag-like file must be written");
        for json in [false, true] {
            let output = if json {
                run(&["--json", "--", file], None, Some(&directory.0))
            } else {
                run(&["--", file], None, Some(&directory.0))
            };
            assert_validation(&output, base, json);
        }
    }
}

#[test]
fn option_termination_preserves_the_stdin_operand() {
    assert_validation(
        &run(&["--", "-"], Some(VALID_INPUT.as_bytes()), None),
        &CASES[0],
        false,
    );
    assert_validation(
        &run(&["--json", "--", "-"], Some(INVALID_INPUT.as_bytes()), None),
        &CASES[1],
        true,
    );
}
