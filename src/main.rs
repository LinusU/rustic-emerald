mod audio;
mod cries;
mod desktop;
mod game;
mod picker;
mod replay;
mod rom;
mod rtc;
mod save;

use clap::Parser;
#[cfg(not(feature = "reference-c"))]
use emerald_game as _;
use std::{path::PathBuf, process::ExitCode};

#[derive(Parser)]
#[command(version, about)]
struct Options {
    /// English Emerald cartridge used for its original sound bank
    #[arg(long, default_value = rom::DEFAULT_FILE)]
    rom: PathBuf,
    /// Play without cartridge audio
    #[arg(long)]
    mute: bool,
    /// Open or create a named adventure, bypassing the save picker
    #[arg(long)]
    slot: Option<String>,
    /// Override the platform's Rustic Emerald save directory
    #[arg(long)]
    save_dir: Option<PathBuf>,
    /// Print the available adventures, trainers, play times, and badges
    #[arg(long)]
    list_saves: bool,
    /// Replay a frame-indexed input script headlessly from a fresh boot
    #[arg(long, conflicts_with_all = ["slot", "list_saves", "check_rom"])]
    replay: Option<PathBuf>,
    /// Write replay screenshots as PPM files
    #[arg(long, requires = "replay")]
    capture_dir: Option<PathBuf>,
    /// Verify the audio cartridge and exit
    #[arg(long)]
    check_rom: bool,
}

fn run(options: Options) -> Result<(), String> {
    if let Some(path) = &options.replay {
        return rtc::with_replay_clock(|| replay::run(path, options.capture_dir.as_deref()));
    }
    if options.check_rom {
        rom::validate(&rom::read(&options.rom)?)?;
        println!("Pokémon Emerald (English, revision 0) verified.");
        return Ok(());
    }
    let directory = options
        .save_dir
        .or_else(|| dirs::data_local_dir().map(|base| base.join("Rustic Emerald").join("saves")))
        .ok_or("could not locate the user data directory; supply --save-dir")?;
    if options.list_saves {
        let slots = save::list(&directory)?;
        if slots.is_empty() {
            println!("No saved adventures in {}", directory.display());
        }
        for (name, preview) in slots {
            println!("{name}\t{preview}");
        }
        return Ok(());
    }
    let name = match options.slot {
        Some(name) => name,
        None => {
            let slots = save::list(&directory)?;
            if slots.is_empty() {
                "Adventure 1".into()
            } else if let Some(name) = picker::choose(&slots)? {
                name
            } else {
                return Ok(());
            }
        }
    };
    let mut mixer = if options.mute {
        None
    } else {
        Some(audio::Mixer::new(&rom::read(&options.rom)?)?)
    };
    let mut save = save::SaveFile::open(&directory, &name)?;
    let mut game = game::Game::new(save.bytes())?;
    desktop::run(&mut game, &mut save, mixer.as_mut())
}

fn main() -> ExitCode {
    match run(Options::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("Rustic Emerald: {message}");
            ExitCode::FAILURE
        }
    }
}
