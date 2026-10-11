//! tab2lily: convert ASCII guitar tab into LilyPond source.
//!
//! The tab is read from the file given on the command line (or standard
//! input when no file, or `-`, is given) and the LilyPond source is written
//! to standard output.  Warnings about ignored or unsupported input are
//! written to standard error.

mod lily;
mod music;
mod parser;

use std::io::{self, Read};
use std::process::ExitCode;

const USAGE: &str = "usage: tab2lily [FILE]\n       tab2lily < FILE\n\nReads ASCII guitar tab from FILE (or standard input) and writes\nLilyPond source to standard output.";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let arg = args.next();
    if matches!(arg.as_deref(), Some("-h") | Some("--help")) {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if args.next().is_some() {
        eprintln!("{USAGE}");
        return ExitCode::FAILURE;
    }

    let input = match arg.as_deref() {
        None | Some("-") => {
            let mut buffer = String::new();
            if let Err(err) = io::stdin().read_to_string(&mut buffer) {
                eprintln!("tab2lily: failed to read standard input: {err}");
                return ExitCode::FAILURE;
            }
            buffer
        }
        Some(path) => match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) => {
                eprintln!("tab2lily: cannot read {path}: {err}");
                return ExitCode::FAILURE;
            }
        },
    };

    let tab = match parser::parse(&input) {
        Ok(tab) => tab,
        Err(err) => {
            eprintln!("tab2lily: {err}");
            return ExitCode::FAILURE;
        }
    };
    let (lilypond, warnings) = lily::generate(&tab);
    for warning in tab.warnings.iter().chain(warnings.iter()) {
        eprintln!("tab2lily: warning: {warning}");
    }
    print!("{lilypond}");
    ExitCode::SUCCESS
}
