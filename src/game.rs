//! Single-owner boundary around Emerald's native C/Rust engine. The engine
//! retains global game state. UI and callbacks run on the thread that owns it.
use crate::audio;
use std::{
    cell::Cell,
    ffi::{c_ulong, c_void},
    ptr::NonNull,
    slice,
    sync::{Mutex, MutexGuard},
};

pub const WIDTH: u32 = 240;
pub const HEIGHT: u32 = 160;
const FLASH_LEN: usize = 128 * 1024;
static ENGINE: Mutex<()> = Mutex::new(());
const RESET_CHORD: u16 = 0b1111; // A, B, Select and Start
thread_local! { static RESET_PENDING: Cell<bool> = const { Cell::new(false) }; }

// The BIOS would jump to the cartridge entry point. The desktop must first
// return from all native game callbacks before recreating their global state.
#[unsafe(no_mangle)]
extern "C" fn SoftReset(_flags: u32) {
    RESET_PENDING.set(true);
}

unsafe extern "C" {
    fn native_engine_create() -> *mut c_void;
    fn native_engine_destroy(engine: *mut c_void);
    fn native_engine_reset(engine: *mut c_void);
    fn native_engine_boot(engine: *mut c_void);
    fn native_engine_set_keys(engine: *mut c_void, pressed: u32);
    fn native_engine_run_frame(engine: *mut c_void);
    fn native_engine_render(engine: *mut c_void);
    fn native_engine_display_buffer(engine: *mut c_void) -> *const u8;
    fn NativeDecodePointer(word: c_ulong) -> *mut u8;
    fn GetPlayerIDAsU32() -> u32;
    fn RtcInit();
    fn m4aSoundInit();
}

pub struct Game {
    engine: NonNull<c_void>,
    // The flash region has this fixed address for the engine's entire lifetime.
    flash: NonNull<u8>,
    wait_for_reset_release: bool,
    _exclusive: MutexGuard<'static, ()>,
}

impl Game {
    pub fn new(save: Option<&[u8]>) -> Result<Self, String> {
        if save.is_some_and(|bytes| bytes.len() != FLASH_LEN) {
            return Err("expected a 128 KiB Emerald flash save".into());
        }
        let exclusive = ENGINE.try_lock().map_err(|_| "game already running")?;
        let engine = NonNull::new(unsafe { native_engine_create() })
            .ok_or("could not initialize the native game")?;
        // native_engine_create installs the entire GBA address space before
        // NativeDecodePointer can be called. 0x0e000000 is cartridge flash.
        let flash = NonNull::new(unsafe { NativeDecodePointer(0x0e000000) })
            .expect("initialized engine has flash");
        let mut game = Self {
            engine,
            flash,
            wait_for_reset_release: false,
            _exclusive: exclusive,
        };
        if let Some(bytes) = save {
            unsafe {
                game.flash
                    .as_ptr()
                    .copy_from_nonoverlapping(bytes.as_ptr(), FLASH_LEN)
            };
        }
        game.boot();
        Ok(game)
    }

    fn boot(&mut self) {
        RESET_PENDING.set(false);
        audio::clear_events();
        crate::cries::reset();
        unsafe {
            native_engine_boot(self.engine.as_ptr());
            m4aSoundInit();
            RtcInit();
        }
    }

    pub fn set_keys(&mut self, pressed: u16) {
        self.wait_for_reset_release &= pressed & RESET_CHORD == RESET_CHORD;
        let keys = if self.wait_for_reset_release {
            0
        } else {
            pressed
        };
        unsafe { native_engine_set_keys(self.engine.as_ptr(), keys.into()) }
    }

    /// Returns true after a cartridge reset so the frontend can clear queued audio.
    pub fn frame(&mut self) -> bool {
        unsafe { native_engine_run_frame(self.engine.as_ptr()) };
        if !RESET_PENDING.get() {
            return false;
        }
        unsafe { native_engine_reset(self.engine.as_ptr()) };
        self.boot();
        self.wait_for_reset_release = true;
        true
    }

    /// The compositor exposes 240×160 tightly packed RGBA8 pixels. A mutable
    /// borrow prevents the next frame or a second render from changing them.
    pub fn render(&mut self) -> &[u8] {
        unsafe {
            native_engine_render(self.engine.as_ptr());
            slice::from_raw_parts(
                native_engine_display_buffer(self.engine.as_ptr()),
                (WIDTH * HEIGHT * 4) as usize,
            )
        }
    }

    pub fn trainer_id(&self) -> u32 {
        unsafe { GetPlayerIDAsU32() }
    }

    pub fn save_data(&self) -> &[u8] {
        unsafe { slice::from_raw_parts(self.flash.as_ptr(), FLASH_LEN) }
    }
}

impl Drop for Game {
    fn drop(&mut self) {
        unsafe { native_engine_destroy(self.engine.as_ptr()) };
        audio::clear_events();
    }
}

#[cfg(test)]
mod tests;
