use std::{
    env,
    ffi::OsString,
    fs,
    io::{self, Read, Write},
    process::ExitCode,
};

const USAGE: &str = "Usage: dtcg-validator [--json] <FILE|->";
const HELP: &str = "Validate a Design Tokens Community Group 2025.10 Format document.

Usage: dtcg-validator [--json] <FILE|->

Arguments:
  <FILE|->   Read one JSON document from FILE, or from standard input with -.

Options:
  --json         Print the shared JSON validation report to standard output.
  -h, --help     Print this help and exit.
  -V, --version  Print the version and exit.
  --             Treat all remaining arguments as operands.

Valid documents are silent unless --json is supplied. Otherwise diagnostics go
to standard error with their JSON Pointer path (\"\" for root), category, and message.
Exit status: 0 valid/help/version, 1 invalid document, 2 usage or I/O error.

Examples:
  dtcg-validator tokens.json
  dtcg-validator --json -
  dtcg-validator -- -tokens.json
";

enum Command {
    Help,
    Version,
    Validate { json: bool, path: OsString },
}

fn main() -> ExitCode {
    let command = match parse_args() {
        Ok(command) => command,
        Err(error) => {
            let _ = writeln!(
                io::stderr().lock(),
                "dtcg-validator: {error}\n\n{USAGE}\nTry 'dtcg-validator --help' for more information."
            );
            return ExitCode::from(2);
        }
    };

    match execute(command) {
        Ok(status) => status,
        Err(error) => {
            let _ = writeln!(io::stderr().lock(), "dtcg-validator: {error}");
            ExitCode::from(2)
        }
    }
}

fn parse_args() -> Result<Command, String> {
    let mut json = false;
    let mut help = false;
    let mut version = false;
    let mut options = true;
    let mut path = None;

    for argument in env::args_os().skip(1) {
        if options {
            match argument.to_str() {
                Some("--") => {
                    options = false;
                    continue;
                }
                Some("--json") => {
                    json = true;
                    continue;
                }
                Some("--help" | "-h") => {
                    help = true;
                    continue;
                }
                Some("--version" | "-V") => {
                    version = true;
                    continue;
                }
                _ => {
                    if argument != "-" && argument.as_encoded_bytes().starts_with(b"-") {
                        return Err(format!("unknown option {argument:?}"));
                    }
                }
            }
        }

        if path.is_some() {
            return Err("expected exactly one input document; received multiple operands".into());
        }
        path = Some(argument);
    }

    if help {
        return Ok(Command::Help);
    }
    if version {
        return Ok(Command::Version);
    }
    let path = path.ok_or_else(|| "missing input document; provide a file or -".to_owned())?;
    Ok(Command::Validate { json, path })
}

fn execute(command: Command) -> Result<ExitCode, String> {
    let (json, path) = match command {
        Command::Help => {
            write_stdout(HELP.as_bytes())?;
            return Ok(ExitCode::SUCCESS);
        }
        Command::Version => {
            write_stdout(concat!("dtcg-validator ", env!("CARGO_PKG_VERSION"), "\n").as_bytes())?;
            return Ok(ExitCode::SUCCESS);
        }
        Command::Validate { json, path } => (json, path),
    };

    let input = if path == "-" {
        let mut input = String::new();
        io::stdin()
            .lock()
            .read_to_string(&mut input)
            .map_err(|error| format!("cannot read standard input: {error}"))?;
        input
    } else {
        fs::read_to_string(&path).map_err(|error| format!("cannot read {path:?}: {error}"))?
    };

    let report = dtcg_validator::validate(&input);
    let status = if report.valid {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    };

    if json {
        let mut output = serde_json::to_vec(&report)
            .map_err(|error| format!("cannot serialize validation report: {error}"))?;
        output.push(b'\n');
        write_stdout(&output)?;
    } else {
        let mut stderr = io::stderr().lock();
        for diagnostic in report.diagnostics {
            writeln!(
                stderr,
                "{:?} [{}]: {}",
                diagnostic.path, diagnostic.code, diagnostic.message
            )
            .map_err(|error| format!("cannot write standard error: {error}"))?;
        }
    }

    Ok(status)
}

fn write_stdout(output: &[u8]) -> Result<(), String> {
    let mut stdout = io::stdout().lock();
    stdout
        .write_all(output)
        .and_then(|_| stdout.flush())
        .map_err(|error| format!("cannot write standard output: {error}"))
}
