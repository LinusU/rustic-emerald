//! Replays use the frame/button/screenshot format from the native reference
//! implementation so that the same inputs can exercise both builds.
use crate::game::{Game, HEIGHT, WIDTH};
use std::{fs, io::Write, path::Path};

enum Action<'a> {
    Button(u16, bool),
    CheckTrainerId,
    Screenshot(&'a str),
}

fn parse(line: &str) -> Option<(u32, Action<'_>)> {
    let mut words = line.split_whitespace();
    let frame = words.next()?.parse().ok()?;
    let action = match words.next()? {
        "button" => {
            let key = words.next()?;
            let bit = [
                "a", "b", "select", "start", "right", "left", "up", "down", "r", "l",
            ]
            .iter()
            .position(|&b| b == key)?;
            let pressed = match words.next()? {
                "on" => true,
                "off" => false,
                _ => return None,
            };
            Action::Button(1 << bit, pressed)
        }
        "probe" if words.next()? == "trainer-id-nonzero" => Action::CheckTrainerId,
        "screenshot" => {
            let name = words.next()?;
            if !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            {
                return None;
            }
            Action::Screenshot(name)
        }
        _ => return None,
    };
    words.next().is_none().then_some((frame, action))
}

pub fn play(
    game: &mut Game,
    script: &str,
    mut checkpoint: impl FnMut(u32, &str, &[u8]) -> Result<(), String>,
) -> Result<(), String> {
    let mut events = Vec::new();
    for (line, raw) in script.lines().enumerate() {
        let command = raw.split('#').next().unwrap().trim();
        if command.is_empty() {
            continue;
        }
        let event =
            parse(command).ok_or_else(|| format!("invalid replay line {}: {command}", line + 1))?;
        events.push(event);
    }
    // Source scripts can add events out of sequence. Preserve file order
    // for multiple input changes scheduled on the same frame.
    events.sort_by_key(|event| event.0);
    let last = events.last().ok_or("empty replay")?.0;
    let mut events = events.into_iter().peekable();
    let mut held = 0;
    let mut captures = Vec::new();
    for frame in 0..=last {
        let mut check_trainer_id = false;
        while let Some((_, action)) = events.next_if(|event| event.0 == frame) {
            match action {
                Action::Button(mask, true) => held |= mask,
                Action::Button(mask, false) => held &= !mask,
                Action::CheckTrainerId => check_trainer_id = true,
                Action::Screenshot(name) => captures.push(name),
            }
        }
        crate::rtc::set_replay_frame(frame);
        game.set_keys(held);
        game.frame();
        crate::audio::clear_events();
        if check_trainer_id && game.trainer_id() == 0 {
            return Err(format!("trainer ID is zero at frame {frame}"));
        }
        for name in captures.drain(..) {
            checkpoint(frame, name, game.render())?;
        }
    }
    Ok(())
}

pub fn hash(rgba: &[u8]) -> u64 {
    rgba.iter().fold(0xcbf29ce484222325u64, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}

pub fn run(path: &Path, directory: Option<&Path>) -> Result<(), String> {
    let script = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if let Some(dir) = directory {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut game = Game::new(None)?;
    play(&mut game, &script, |frame, name, pixels| {
        println!("{frame:6}  {:016x}  {name}", hash(pixels));
        if let Some(dir) = directory {
            let path = dir.join(format!("{frame}-{name}.ppm"));
            let write = || -> std::io::Result<()> {
                let mut out = std::io::BufWriter::new(fs::File::create(&path)?);
                write!(out, "P6\n{WIDTH} {HEIGHT}\n255\n")?;
                let (rgba_pixels, _) = pixels.as_chunks::<4>();
                for pixel in rgba_pixels {
                    out.write_all(&pixel[..3])?;
                }
                out.flush()
            };
            write().map_err(|e| format!("{}: {e}", path.display()))?;
        }
        Ok(())
    })
}
