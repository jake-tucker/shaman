//! Writes the Pairing wire's schema, and checks it.
//!
//! ```text
//! cargo run -p shaman-pairing-wire -- write
//! cargo run -p shaman-pairing-wire -- check [--base <schema>]
//! ```
//!
//! `write` writes `pairing-wire.json` from the types. `check` fails where
//! that file is not what `write` would write, and, given the schema a change
//! started from as `--base`, where the change is one an older side could not
//! survive and the floor did not rise with it.

use std::{fs, path::PathBuf, process::ExitCode};

use serde_json::Value;
use shaman_pairing_wire::{SCHEMA_FILE, judge, pairing_wire, written};

const USAGE: &str = "usage: shaman-pairing-wire write | check [--base <schema>]";

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    let result = match arguments[..] {
        ["write"] => write(),
        ["check"] => check(None),
        ["check", "--base", base] => check(Some(PathBuf::from(base))),
        _ => Err(USAGE.to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn write() -> Result<(), String> {
    fs::write(SCHEMA_FILE, written(&pairing_wire()))
        .map_err(|error| format!("could not write {SCHEMA_FILE}: {error}"))?;
    println!("wrote {SCHEMA_FILE}");
    Ok(())
}

fn check(base: Option<PathBuf>) -> Result<(), String> {
    let current = pairing_wire();
    let committed = fs::read_to_string(SCHEMA_FILE)
        .map_err(|error| format!("could not read {SCHEMA_FILE}: {error}"))?;
    // A checkout on Windows may have turned the file's line endings.
    if committed.replace("\r\n", "\n") != written(&current) {
        return Err(format!(
            "{SCHEMA_FILE} is not the schema the Pairing wire's types describe; write it again \
             with `cargo run -p shaman-pairing-wire -- write` and commit it"
        ));
    }
    let Some(base) = base else {
        println!("{SCHEMA_FILE} is current");
        return Ok(());
    };
    let base: Value = serde_json::from_str(
        &fs::read_to_string(&base)
            .map_err(|error| format!("could not read {}: {error}", base.display()))?,
    )
    .map_err(|error| format!("{} is not a schema: {error}", base.display()))?;
    let verdict = judge(&base, &current)?;
    print!("{}", verdict.report());
    if verdict.passes() {
        Ok(())
    } else {
        Err("the Pairing wire's schema does not pass its check: see each error above".to_owned())
    }
}
