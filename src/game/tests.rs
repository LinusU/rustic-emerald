use super::*;
use crate::save::SaveFile;

unsafe extern "C" {
    fn NativeTestSave() -> i32;
    fn NativeTestLoad() -> i32;
    fn ClearSaveData();
    static gBattleOutcome: u8;
    fn VarGet(id: u16) -> u16;
}

#[test]
fn mudkip_wins_birch_rescue_and_cartridge_save_survives_reboot() {
    let mut game = Game::new(None).unwrap();
    let replay = include_str!("../../vendor/pokeemerald/tools/wasm_replays/mudkip_starter.txt");
    crate::replay::play(&mut game, replay, |frame, _, pixels| {
        let expected = match frame {
            23126 => 0xae3ef35597420a01, // Route 101
            26108 => 0x4260789f1210ad1f, // Birch's bag
            27228 => 0xc52762b7d0822c58, // first battle with Mudkip
            _ => return Ok(()),
        };
        assert_eq!(crate::replay::hash(pixels), expected, "frame {frame}");
        Ok(())
    })
    .unwrap();

    // Keep choosing FIGHT and TACKLE, then advance Birch's dialogue. Only
    // the script in his lab sets this variable to 3 after giving the starter.
    const VAR_BIRCH_LAB_STATE: u16 = 0x4084;
    for frame in 27229..35000 {
        let press_a = frame >= 27240 && (frame - 27240) % 24 < 4;
        game.set_keys(u16::from(press_a));
        game.frame();
        if unsafe { VarGet(VAR_BIRCH_LAB_STATE) } == 3 {
            break;
        }
    }
    assert_eq!(unsafe { gBattleOutcome }, 1, "Mudkip won the first battle");
    assert_eq!(
        unsafe { VarGet(VAR_BIRCH_LAB_STATE) },
        3,
        "Birch gave the starter in his lab"
    );

    // The native fixture calls Emerald's own save routine and populates two
    // values in SaveBlock1. Read them back after a fresh boot from disk.
    let directory = tempfile::tempdir().unwrap();
    let mut slot = SaveFile::open(directory.path(), "Mudkip").unwrap();
    assert_eq!(unsafe { NativeTestSave() }, 0);
    slot.flush(game.save_data()).unwrap();
    drop((game, slot));
    let slot = SaveFile::open(directory.path(), "Mudkip").unwrap();
    assert_eq!(slot.bytes().unwrap().len(), FLASH_LEN);
    let mut game = Game::new(slot.bytes()).unwrap();
    for _ in 0..300 {
        game.frame();
    }
    let boot_screen = crate::replay::hash(game.render());
    assert_eq!(unsafe { NativeTestLoad() }, 0);

    // Press the game's reset chord and keep holding it across reboot. The
    // same boot screen must return without resetting on every held frame.
    for speed in [1, 2, 4] {
        game.set_keys(0);
        game.set_keys(RESET_CHORD);
        assert!(game.frame());
        assert_eq!(game.save_data(), slot.bytes().unwrap());
        // The desktop samples input once per batch of accelerated frames.
        for _ in 0..300 / speed {
            game.set_keys(RESET_CHORD);
            for _ in 0..speed {
                assert!(!game.frame());
            }
        }
        assert_eq!(crate::replay::hash(game.render()), boot_screen);
        assert_eq!(unsafe { NativeTestLoad() }, 0);
    }
    game.set_keys(0);

    // The save deletion screen erases physical flash before its BIOS reset.
    // Preserve that erasure as faithfully as a populated cartridge.
    unsafe { ClearSaveData() };
    SoftReset(0xff);
    assert!(game.frame());
    assert!(game.save_data().iter().all(|&byte| byte == 0xff));
}
