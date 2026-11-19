//! Emerald stores one PCM recording per cry ID. PlayCryInternal supplies the
//! game's pitch, panning, duration, release and reverse-playback settings.
use std::cell::RefCell;

include!(concat!(env!("OUT_DIR"), "/cry_bank.rs"));
static PCM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/cry_bank.pcm"));

const OUTPUT_HZ: f64 = 32768.0;
const SOURCE_HZ: f64 = 10512.0;
const SAMPLES_PER_FRAME: f64 = 280896.0 / 512.0;

struct Voice {
    pcm: &'static [u8],
    speed: f64,
    chorus_speed: Option<f64>,
    reverse: bool,
    gains: [f64; 2],
    gate: u32,
    release: f64,
    start_frame: u32,
    total_samples: u32,
    position: u32,
}

impl Voice {
    fn playing(&self, frame: u32) -> bool {
        f64::from(frame.wrapping_sub(self.start_frame)) * SAMPLES_PER_FRAME
            < f64::from(self.total_samples)
    }

    fn sample(&self, speed: f64) -> f64 {
        let offset = f64::from(self.position) * speed;
        if offset >= self.pcm.len() as f64 {
            return 0.0;
        }
        let pos = if self.reverse {
            (self.pcm.len() as f64 - 1.0 - offset).max(0.0)
        } else {
            offset
        };
        let index = pos as usize;
        let a = f64::from(self.pcm[index]);
        let b = f64::from(*self.pcm.get(index + 1).unwrap_or(&self.pcm[index]));
        (a + (b - a) * pos.fract() - 128.0) * 256.0
    }

    fn mix(&mut self, output: &mut [i16]) {
        let remaining = self.total_samples - self.position;
        for frame in output.chunks_exact_mut(2).take(remaining as usize) {
            let mut sample = self.sample(self.speed);
            if let Some(chorus) = self.chorus_speed {
                sample = (sample + self.sample(chorus)) * 0.5;
            }
            if self.position >= self.gate {
                let frames = f64::from(self.position - self.gate) / SAMPLES_PER_FRAME;
                sample *= self.release.powf(frames + 1.0);
            }
            for (out, gain) in frame.iter_mut().zip(self.gains) {
                *out = out.saturating_add((sample * gain) as i16);
            }
            self.position += 1;
        }
    }
}

// The game keeps two cry players for double battles. Status queries and StopCry
// address the most recently issued cry. The engine and mixer run on one thread.
#[derive(Default)]
struct Playback {
    voices: [Option<Voice>; 2],
    latest: usize,
}

thread_local! { static PLAYBACK: RefCell<Playback> = RefCell::default(); }

#[unsafe(no_mangle)]
extern "C" fn RusticCryStart(
    id: u16,
    pan: i8,
    volume: i8,
    pitch: u32,
    length: u32,
    release: u32,
    chorus: u32,
    reverse: u32,
    frame: u32,
) {
    let Some(&(offset, size)) = CRIES.get(usize::from(id)) else {
        return;
    };
    // Each semitone in SetPokemonCryPitch occupies 256 pitch units; natural
    // playback is MIDI key 60. Chorus detunes track two in 1/64 semitones.
    let semitones = (f64::from(pitch) - 15360.0) / 256.0;
    let speed = SOURCE_HZ / OUTPUT_HZ * 2.0f64.powf(semitones / 12.0);
    let chorus_speed = (chorus != 0).then(|| {
        // MP2K wraps the second track's seven-bit TUNE operand.
        let tune = ((pitch as i32 + 128) >> 1) & 127;
        let delta = ((tune + chorus as i32) & 127) - tune;
        speed * 2.0f64.powf(f64::from(delta) / (64.0 * 12.0))
    });
    let gate = (f64::from(length) * SAMPLES_PER_FRAME).ceil() as u32;
    let release = f64::from(release.min(255)) / 256.0;
    let release_frames = if release == 0.0 {
        0.0
    } else {
        (1.0 / 256.0f64).ln() / release.ln()
    };
    let natural_samples = (size as f64 / speed.min(chorus_speed.unwrap_or(speed))).ceil() as u32;
    let total_samples =
        natural_samples.min(gate + (release_frames * SAMPLES_PER_FRAME).ceil() as u32);
    let pan = f64::from(pan.clamp(-64, 63));
    let volume = f64::from(volume.clamp(0, 127)) / 127.0;
    let voice = Voice {
        pcm: &PCM[offset..offset + size],
        speed,
        chorus_speed,
        reverse: reverse != 0,
        gains: [volume * (63.0 - pan) / 127.0, volume * (pan + 64.0) / 127.0],
        gate,
        release,
        start_frame: frame,
        total_samples,
        position: 0,
    };
    PLAYBACK.with_borrow_mut(|playback| {
        let index = playback
            .voices
            .iter()
            .position(|voice| voice.as_ref().is_none_or(|voice| !voice.playing(frame)))
            .unwrap_or(1 - playback.latest);
        playback.latest = index;
        playback.voices[index] = Some(voice);
    });
}

#[unsafe(no_mangle)]
extern "C" fn RusticCryPlaying(frame: u32) -> u32 {
    PLAYBACK.with_borrow(|p| {
        // Game time is authoritative even with fast-forward or no audio device.
        p.voices[p.latest]
            .as_ref()
            .is_some_and(|v| v.playing(frame)) as u32
    })
}

#[unsafe(no_mangle)]
extern "C" fn RusticCryStop() {
    PLAYBACK.with_borrow_mut(|p| p.voices[p.latest] = None);
}

#[unsafe(no_mangle)]
extern "C" fn RusticCryStopAll() {
    reset();
}

pub fn reset() {
    PLAYBACK.with_borrow_mut(|p| *p = Playback::default());
}

pub fn render(samples: &mut [i16]) {
    PLAYBACK.with_borrow_mut(|p| {
        for voice in p.voices.iter_mut().flatten() {
            voice.mix(samples);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampled_cry_respects_pan_stop_and_silent_game_time() {
        assert_eq!(CRIES.len(), 388);
        reset();
        let start = u32::MAX - 7;
        RusticCryStart(0, -64, 120, 15360, 140, 0, 0, 0, start);
        let mut samples = [0; 4096];
        render(&mut samples);
        assert!(samples.chunks_exact(2).any(|s| s[0] != 0));
        assert!(samples.chunks_exact(2).all(|s| s[1] == 0));
        assert_eq!(RusticCryPlaying(start + 2), 1);
        assert_eq!(RusticCryPlaying(start.wrapping_add(140)), 0);
        RusticCryStop();
        let mut silence = [10; 128];
        render(&mut silence);
        assert_eq!(silence, [10; 128]);
        assert_eq!(RusticCryPlaying(start), 0);
    }
}
