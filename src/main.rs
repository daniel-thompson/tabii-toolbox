//! tab2lily: convert ASCII guitar tab into LilyPond source or MIDI.
//!
//! The tab is read from the file given on the command line (or standard
//! input when no file, or `-`, is given).  By default the LilyPond source
//! is written to standard output; `--midi` writes a standard MIDI file
//! instead.  Warnings about ignored or unsupported input are written to
//! standard error.

mod lily;
mod midi;
mod music;
mod parser;

use std::io::{self, Read, Write};
use std::process::ExitCode;

const USAGE: &str = "usage: tab2lily [--midi] [FILE]\n       tab2lily [--midi] < FILE\n\nReads ASCII guitar tab from FILE (or standard input) and writes\nLilyPond source to standard output, or a standard MIDI file with\n--midi.";

fn main() -> ExitCode {
    let mut midi_output = false;
    let mut file: Option<String> = None;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "--midi" => midi_output = true,
            _ => {
                if file.is_some() {
                    eprintln!("{USAGE}");
                    return ExitCode::FAILURE;
                }
                file = Some(arg);
            }
        }
    }

    let input = match file.as_deref() {
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
    if midi_output {
        let (bytes, warnings) = midi::generate(&tab);
        report(&tab, &warnings);
        if let Err(err) = io::stdout().write_all(&bytes) {
            eprintln!("tab2lily: failed to write MIDI output: {err}");
            return ExitCode::FAILURE;
        }
    } else {
        let (lilypond, warnings) = lily::generate(&tab);
        report(&tab, &warnings);
        print!("{lilypond}");
    }
    ExitCode::SUCCESS
}

fn report(tab: &parser::Tab, extra: &[String]) {
    for warning in tab.warnings.iter().chain(extra.iter()) {
        eprintln!("tab2lily: warning: {warning}");
    }
}
