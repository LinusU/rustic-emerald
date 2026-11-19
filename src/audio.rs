//! Emerald's music player issues commands during its frame callback. Consume
//! those commands on the same thread and synthesize at the audio device rate.
use std::{cell::RefCell, ffi::c_void, ptr::NonNull};

unsafe extern "C" {
    fn rustic_audio_bank(rom: *const u8, size: u32) -> *mut c_void;
    fn rustic_audio_free(bank: *mut c_void);
    fn rustic_audio_start(bank: *mut c_void, song: u16) -> *mut c_void;
    fn mp2k_render(player: *mut c_void, samples: *mut i16, count: i32) -> bool;
    fn mp2k_delete(player: *mut c_void);
}

struct Event {
    channel: u8,
    action: u8,
    value: u16,
}
thread_local! { static EVENTS: RefCell<Vec<Event>> = const { RefCell::new(Vec::new()) }; }

#[unsafe(no_mangle)]
extern "C" fn RusticAudioEvent(channel: u8, action: u8, value: u16) {
    EVENTS.with_borrow_mut(|events| {
        events.push(Event {
            channel,
            action,
            value,
        })
    });
}

pub fn clear_events() {
    EVENTS.with_borrow_mut(Vec::clear);
}

struct Player(NonNull<c_void>);
impl Drop for Player {
    fn drop(&mut self) {
        unsafe { mp2k_delete(self.0.as_ptr()) }
    }
}

struct Bank(NonNull<c_void>);
impl Drop for Bank {
    fn drop(&mut self) {
        unsafe { rustic_audio_free(self.0.as_ptr()) }
    }
}

#[derive(Default)]
struct Channel {
    player: Option<Player>,
    paused: bool,
    volume: u16,
    pan: i8,
}

pub struct Mixer {
    // MP2K players borrow their sequences/instruments from the bank. Drop in order.
    channels: [Channel; 4],
    bank: Bank,
    phase: u32,
    scratch: Vec<i16>,
    output: Vec<i16>,
}

impl Mixer {
    pub fn new(rom: &[u8]) -> Result<Self, String> {
        crate::rom::validate(rom)?;
        // The C extractor accepts this exact retail ROM, builds an owned bank
        // and releases all borrows of the cartridge before returning.
        let bank = NonNull::new(unsafe { rustic_audio_bank(rom.as_ptr(), rom.len() as u32) })
            .ok_or("could not read Emerald's instrument bank")?;
        Ok(Self {
            channels: Default::default(),
            bank: Bank(bank),
            phase: 0,
            scratch: Vec::new(),
            output: Vec::new(),
        })
    }

    pub fn apply_events(&mut self) {
        EVENTS.with_borrow_mut(|events| {
            for event in events.drain(..) {
                let Some(channel) = self.channels.get_mut(usize::from(event.channel)) else {
                    continue;
                };
                match event.action {
                    0 => {
                        channel.player = NonNull::new(unsafe {
                            rustic_audio_start(self.bank.0.as_ptr(), event.value)
                        })
                        .map(Player);
                        channel.paused = false;
                        channel.volume = 256;
                        channel.pan = 0;
                    }
                    1 => channel.player = None,
                    2 => channel.paused = true,
                    3 => channel.paused = false,
                    4 => channel.volume = event.value,
                    5 => channel.pan = (event.value as i8).clamp(-64, 64),
                    _ => {}
                }
            }
        })
    }

    pub fn reset(&mut self) {
        self.channels = Default::default();
        self.phase = 0;
    }

    pub fn render(&mut self) -> &[i16] {
        // 280896 GBA cycles/frame, 512 cycles/sample at 32768 Hz.
        self.phase += 280896;
        let count = (self.phase / 512 * 2) as usize;
        self.phase %= 512;
        self.output.clear();
        self.output.resize(count, 0);
        self.scratch.resize(count, 0);
        for channel in &mut self.channels {
            let Some(player) = &channel.player else {
                continue;
            };
            if channel.paused {
                continue;
            }
            self.scratch.fill(0);
            let playing =
                unsafe { mp2k_render(player.0.as_ptr(), self.scratch.as_mut_ptr(), count as i32) };
            let pan = i32::from(channel.pan);
            let gain = [64 - pan.max(0), 64 + pan.min(0)]
                .map(|balance| i32::from(channel.volume) * balance / 64);
            let (frames, _) = self.output.as_chunks_mut::<2>();
            let (input_frames, _) = self.scratch.as_chunks::<2>();
            for (out, input) in frames.iter_mut().zip(input_frames) {
                for side in 0..2 {
                    let mix = i32::from(out[side]) + i32::from(input[side]) * gain[side] / 256;
                    out[side] = mix.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
                }
            }
            if !playing {
                channel.player = None;
            }
        }
        crate::cries::render(&mut self.output);
        &self.output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires the original English Emerald cartridge"]
    fn title_music_and_cursor_effect_produce_stereo_audio() {
        let rom = crate::rom::read(std::path::Path::new(crate::rom::DEFAULT_FILE)).unwrap();
        let mut mixer = Mixer::new(&rom).unwrap();
        for (channel, song) in [(0, 413), (2, 5)] {
            RusticAudioEvent(channel, 0, song);
            mixer.apply_events();
            assert!(
                (0..30).any(|_| mixer.render().iter().any(|&x| x != 0)),
                "song {song}"
            );
            RusticAudioEvent(channel, 0, song);
            RusticAudioEvent(channel, 5, (-64_i8) as u16);
            mixer.apply_events();
            let (stereo, _) = mixer.render().as_chunks::<2>();
            assert!(stereo.iter().any(|s| s[0] != 0));
            assert!(stereo.iter().all(|s| s[1] == 0));
            RusticAudioEvent(channel, 4, 0);
            mixer.apply_events();
            assert!(mixer.render().iter().all(|&x| x == 0));
            RusticAudioEvent(channel, 4, 256);
            RusticAudioEvent(channel, 2, 0);
            mixer.apply_events();
            assert!(mixer.render().iter().all(|&x| x == 0));
            RusticAudioEvent(channel, 1, 0);
            mixer.apply_events();
        }
    }
}
