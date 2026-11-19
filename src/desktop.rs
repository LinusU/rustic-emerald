use crate::{
    audio::{self, Mixer},
    game::{Game, HEIGHT, WIDTH},
    save::SaveFile,
};
use sdl2::{
    audio::{AudioQueue, AudioSpecDesired},
    event::{Event, WindowEvent},
    keyboard::{KeyboardState, Scancode},
    pixels::{Color, PixelFormatEnum},
};
use std::time::{Duration, Instant};

const SAMPLE_RATE: i32 = 32_768;
// 228 scanlines, each lasting 1232 cycles at 16,777,216 Hz.
const FRAME_TIME: Duration = Duration::from_nanos(16_742_706);

pub fn run(game: &mut Game, save: &mut SaveFile, mixer: Option<&mut Mixer>) -> Result<(), String> {
    let session = play(game, save, mixer);
    let flush = save.flush(game.save_data());
    match (session, flush) {
        (Err(session), Err(flush)) => Err(format!("{session}\n{flush}")),
        (session, flush) => session.and(flush),
    }
}

fn play(game: &mut Game, save: &mut SaveFile, mut mixer: Option<&mut Mixer>) -> Result<(), String> {
    let sdl = sdl2::init()?;
    let video = sdl.video()?;
    let title = format!("Rustic Emerald — {}", save.name());
    let mut window = video
        .window(&title, WIDTH * 3, HEIGHT * 3)
        .position_centered()
        .resizable()
        .allow_highdpi()
        .build()
        .map_err(|err| err.to_string())?;
    window
        .set_minimum_size(WIDTH, HEIGHT)
        .map_err(|err| err.to_string())?;
    let mut canvas = window
        .into_canvas()
        .build()
        .map_err(|err| err.to_string())?;
    canvas
        .set_logical_size(WIDTH, HEIGHT)
        .map_err(|err| err.to_string())?;
    canvas.set_draw_color(Color::BLACK);
    sdl2::hint::set("SDL_RENDER_SCALE_QUALITY", "nearest");
    let creator = canvas.texture_creator();
    let mut texture = creator
        .create_texture_streaming(PixelFormatEnum::RGBA32, WIDTH, HEIGHT)
        .map_err(|err| err.to_string())?;

    let audio = mixer.as_ref().and_then(|_| {
        open_audio(&sdl)
            .inspect_err(|err| eprintln!("Audio unavailable: {err}"))
            .ok()
    });
    let mut events = sdl.event_pump()?;
    let mut speed = 1;
    let mut focused = true;
    let mut next_frame = Instant::now();
    let mut last_flush = Instant::now();
    loop {
        for event in events.poll_iter() {
            match event {
                Event::Quit { .. }
                | Event::Window {
                    win_event: WindowEvent::Close,
                    ..
                }
                | Event::KeyDown {
                    scancode: Some(Scancode::Escape),
                    ..
                } => return Ok(()),
                Event::Window {
                    win_event: WindowEvent::FocusLost,
                    ..
                } => {
                    focused = false;
                    game.set_keys(0);
                    if let Some(queue) = &audio {
                        queue.clear();
                    }
                    save.flush(game.save_data())?;
                }
                Event::Window {
                    win_event: WindowEvent::FocusGained,
                    ..
                } => {
                    focused = true;
                    next_frame = Instant::now();
                }
                Event::KeyDown {
                    scancode: Some(key),
                    repeat: false,
                    ..
                } => {
                    speed = match key {
                        Scancode::Num1 => 1,
                        Scancode::Num2 => 2,
                        Scancode::Num4 => 4,
                        _ => continue,
                    };
                    if let Some(queue) = &audio {
                        queue.clear();
                    }
                    let speed_title = if speed == 1 {
                        title.clone()
                    } else {
                        format!("{title} ({speed}×)")
                    };
                    canvas
                        .window_mut()
                        .set_title(&speed_title)
                        .map_err(|err| err.to_string())?;
                }
                _ => {}
            }
        }
        if !focused {
            std::thread::sleep(Duration::from_millis(30));
            continue;
        }

        game.set_keys(pressed_keys(&events.keyboard_state()));
        for _ in 0..speed {
            if game.frame() {
                if let Some(queue) = &audio {
                    queue.clear();
                }
                if let Some(mixer) = &mut mixer {
                    mixer.reset();
                }
                break;
            }
        }
        if let Some(mixer) = &mut mixer {
            mixer.apply_events();
            // Render one audio frame per display tick, including in fast-forward.
            let samples = mixer.render();
            if let Some(queue) = &audio
                && queue.size() < SAMPLE_RATE as u32 * 4 / 10
            {
                queue.queue_audio(samples)?;
            }
        } else {
            audio::clear_events();
        }
        texture
            .update(None, game.render(), WIDTH as usize * 4)
            .map_err(|err| err.to_string())?;
        canvas.clear();
        canvas.copy(&texture, None, None)?;
        canvas.present();

        if last_flush.elapsed() >= Duration::from_secs(1) {
            save.flush(game.save_data())?;
            last_flush = Instant::now();
        }
        next_frame += FRAME_TIME;
        let now = Instant::now();
        if now < next_frame {
            std::thread::sleep(next_frame - now);
        } else if now - next_frame >= FRAME_TIME {
            next_frame = now;
        }
    }
}

fn open_audio(sdl: &sdl2::Sdl) -> Result<AudioQueue<i16>, String> {
    let spec = AudioSpecDesired {
        freq: Some(SAMPLE_RATE),
        channels: Some(2),
        samples: Some(512),
    };
    let queue = sdl.audio()?.open_queue(None, &spec)?;
    queue.resume();
    Ok(queue)
}

fn pressed_keys(keyboard: &KeyboardState<'_>) -> u16 {
    use Scancode::*;
    let bindings: &[&[Scancode]] = &[
        &[Z, N],
        &[X, M],
        &[Space],
        &[Return],
        &[Right, D],
        &[Left, A],
        &[Up, W],
        &[Down, S],
        &[E],
        &[Q],
    ];
    let mut keys = 0;
    for (bit, alternatives) in bindings.iter().enumerate() {
        if alternatives
            .iter()
            .any(|&key| keyboard.is_scancode_pressed(key))
        {
            keys |= 1 << bit;
        }
    }
    keys
}
