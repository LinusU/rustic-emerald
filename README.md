# Rustic Emerald

Play Pokémon Emerald on the desktop with a Rust runtime, the original Hoenn maps and scripts, and multiple independent adventures. Game logic runs as native code. C2Rust translations and the remaining C functions call each other directly in the same process.

## Run

The port runs on a little-endian 64-bit Unix host and has been tested on Apple Silicon. Install Rust 1.89 or newer, Clang, GNU Make, Python 3, pkg-config, libpng and SDL2 development files. On macOS, the Xcode command-line tools supply Clang and Make. Install the remaining macOS libraries with Homebrew.

```sh
brew install pkgconf libpng sdl2
```

Initialize the pinned sources and launch the game from the project directory.

```sh
git submodule update --init --recursive
cargo run --release
```

For cartridge music and sound effects, place an English revision 0 ROM named `pokeemerald.gba` in the working directory, or pass `--rom /path/to/pokeemerald.gba`. Its SHA-1 is `f3ae088181bf583e55daf962a92bb46f4f1d07b7`. Use `cargo run --release -- --mute` to play without it. The first build converts the decompilation's graphics, maps and scripts. Subsequent builds reuse them.

| Action | Keys |
| --- | --- |
| Move | Arrow keys or WASD |
| A / B | Z / X or N / M |
| L / R | Q / E |
| Start / Select | Enter / Space |
| Set speed | 1, 2 or 4 for the corresponding multiplier |
| Quit | Esc |

Hold A, B, Start and Select together for a soft reset. Your cartridge save survives a reset. The window pauses when it loses focus. Fast-forward advances gameplay while music plays at its normal sample rate.

## Saves

Choose **Save** in Emerald's in-game menu to record progress. The first launch uses **Adventure 1**. On later launches, the save picker shows the trainer, play time and badge count. Select an adventure or choose **New game** for a fresh slot. Each file is a 128 KiB Emerald battery save.

```sh
cargo run --release -- --slot May
cargo run --release -- --list-saves
cargo run --release -- --save-dir ./saves --slot Brendan
```

On macOS the default directory is `~/Library/Application Support/Rustic Emerald/saves`. On Linux it is `$XDG_DATA_HOME/Rustic Emerald/saves`, defaulting to `~/.local/share/Rustic Emerald/saves`. To import a battery save, copy it into that directory as `<slot>.sav` with the game closed. Export by copying the file out.

## Native port

The pinned engine provides the game data, native pointer translation for Emerald's 32-bit scripts, graphics compositor and BIOS helpers. Cargo compiles the C units still in use and links them to the Rust game modules. Rust owns the window, input, audio mixing, cartridge clock, save selection and disk persistence. The MP2K synthesizer reads the original instrument bank and sequences from the verified cartridge. Pokémon cries use the corresponding PCM recordings in the decompilation.

Of Emerald's 313 C game files, 21 now run as C2Rust-generated Rust. These include the battle script interpreter, trainer AI, starter selection, evolution, wild encounters and task scheduler. The cartridge clock has a separate handwritten Rust implementation. The other game files compile as native C and call Rust through the original C ABI.

`tools/rust-units.txt` lists the translated files. Both the generator and the native build read that list, so each routine has one implementation in the binary. The generator writes to `src/translated/`, checks structure sizes, alignments and field offsets against Clang, and marks mutable game globals for reset.

Normal builds use the committed translations. With C2Rust 0.22.1 installed, these commands regenerate and verify the Rust modules.

```sh
python3 tools/translate.py
python3 tools/translate.py --check
cargo test --release
```

The gameplay test follows the moving truck through Littleroot, picks Mudkip, wins the first battle and checks that Birch hands over the starter in his lab. It also writes a cartridge save, boots it again and verifies that a soft reset preserves it. The replay runner accepts the C reference build's input recordings and can write 240×160 PPM images for inspection.

```sh
cargo run --release -- --replay vendor/pokeemerald/tools/wasm_replays/mudkip_starter.txt --capture-dir /tmp/emerald-frames
```

The cartridge audio test can be run with `cargo test --release -- --ignored`. Link cable and wireless multiplayer are outside the current desktop runtime.

## Credits

- Game Freak and the original Pokémon Emerald team.
- The pret team for the Emerald decompilation. [pret/pokeemerald](https://github.com/pret/pokeemerald)
- tripplyons and contributors for the native engine and gameplay replay. [pokeemerald-wasm](https://github.com/tripplyons/pokeemerald-wasm)
- Immunant and contributors for C2Rust. [C2Rust](https://github.com/immunant/c2rust)
- The gba-audio-tools project for its MP2K extractor and synthesizer, distributed under GPL-3.0-or-later. [gba-audio-tools](https://github.com/mudassarzahid/gba-audio-tools)
