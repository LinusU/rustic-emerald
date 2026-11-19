use super::*;
use crate::save::SaveFile;

unsafe extern "C" {
    fn NativeTestSave() -> i32;
    fn NativeTestLoad() -> i32;
    fn ClearSaveData();
    fn CreateMon(
        mon: *mut PartyPokemon,
        species: u16,
        level: u8,
        iv: u8,
        fixed_personality: u8,
        personality: u32,
        ot_type: u8,
        ot_id: u32,
    );
    static gBattleOutcome: u8;
    fn VarGet(id: u16) -> u16;
}

// The native C layout and the on-cartridge party record are both 100 bytes.
// A fixed OT and personality make the ciphertext, moves, and stats comparable.
#[repr(C, align(4))]
struct PartyPokemon([u8; 100]);

fn check_party_records() {
    let mut records = Vec::with_capacity(386 * 3 * 100);
    // 252–276 are unused internal Unown placeholders.
    for species in (1..=251).chain(277..=411) {
        for level in [5, 50, 100] {
            let mut mon = PartyPokemon([0; 100]);
            let personality = u32::from(species) * 65537 + u32::from(level);
            unsafe { CreateMon(&mut mon, species, level, 31, 1, personality, 1, 0x12345678) };
            records.extend_from_slice(&mon.0);
        }
    }
    assert_eq!(
        crate::replay::hash(&records),
        0x80ebba10e18fd7af,
        "party records differ from the C reference"
    );
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

    check_party_records();

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
