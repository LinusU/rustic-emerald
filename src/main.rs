mod rom;

use std::{path::Path, process::ExitCode};

fn main() -> ExitCode {
    match rom::read(Path::new(rom::DEFAULT_FILE)).and_then(|rom| rom::validate(&rom)) {
        Ok(()) => {
            println!("Pokémon Emerald (English, revision 0) verified.");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("Rustic Emerald: {message}");
            ExitCode::FAILURE
        }
    }
}
