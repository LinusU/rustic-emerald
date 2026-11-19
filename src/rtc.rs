//! Cartridge clock boundary. Emerald stores its user-selected clock offset
//! in the save. The chip supplies BCD wall time independently of frame speed.
use chrono::{Datelike, Local, Timelike};

#[repr(C)]
struct Clock {
    year: u8,
    month: u8,
    day: u8,
    weekday: u8,
    hour: u8,
    minute: u8,
    second: u8,
    status: u8,
    alarm_hour: u8,
    alarm_minute: u8,
}

fn bcd(value: u32) -> u8 {
    (((value / 10) << 4) | (value % 10)) as u8
}

// These are the S-3511A driver operations used by Emerald's rtc.c.
// No bus protection is needed when reads use the host's clock.
#[unsafe(no_mangle)]
extern "C" fn SiiRtcUnprotect() {}
#[unsafe(no_mangle)]
extern "C" fn SiiRtcProtect() {}
#[unsafe(no_mangle)]
extern "C" fn SiiRtcProbe() -> u8 {
    1
}
#[unsafe(no_mangle)]
extern "C" fn SiiRtcReset() -> u8 {
    1
}

#[unsafe(no_mangle)]
unsafe extern "C" fn SiiRtcGetStatus(clock: *mut Clock) -> u8 {
    unsafe { (*clock).status = 0x40 };
    1
}

#[unsafe(no_mangle)]
unsafe extern "C" fn SiiRtcGetDateTime(clock: *mut Clock) -> u8 {
    let now = Local::now();
    // Write only the seven calendar bytes, as on the cartridge.
    unsafe {
        (*clock).year = bcd(now.year().rem_euclid(100) as u32);
        (*clock).month = bcd(now.month());
        (*clock).day = bcd(now.day());
        (*clock).weekday = now.weekday().num_days_from_sunday() as u8;
        (*clock).hour = bcd(now.hour());
        (*clock).minute = bcd(now.minute());
        (*clock).second = bcd(now.second());
    }
    1
}
