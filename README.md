# Rustic Emerald

Play Pokémon Emerald on the desktop with a Rust runtime, the original Hoenn maps and scripts, and multiple independent adventures. The game logic and graphics compositor run as native Rust code.

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

All 313 game translation units in the pinned Emerald source have Rust implementations. C2Rust generated 312 modules. The S-3511A cartridge clock uses the handwritten driver in `src/rtc.rs` to read desktop time. Cargo compiles the generated modules in parallel across 16 crates. The normal build checks the source inventory and links zero C objects from Emerald's `src/` directory.

Rust owns the window, input, rendering, audio mixing, clock, save selection and disk persistence. The native support code maps 32-bit script addresses to host pointers, supplies BIOS services and serializes cartridge flash. The original maps and script bytecode remain game data. The MP2K synthesizer reads instruments and sequences from the verified cartridge, and cries use the PCM recordings in the decompilation.

`tools/rust-units.txt` records the C2Rust inventory. During regeneration, Clang supplies structure sizes, alignments and field offsets for compile-time Rust assertions. Mutable globals are restored when the game resets. The port preserves two cartridge behaviors that relied on adjacent memory in C. Magnitude uses animation argument 15, and the Battle Factory fade reads color 5 of the interface palette.

Normal builds use the committed translations. To regenerate them, install CMake, a C++ compiler, and LLVM 21 with its Clang development files. On macOS, run `brew install cmake llvm@21`. On Linux, set `LLVM_CONFIG_PATH` to the path of your LLVM 21 `llvm-config` executable if it is outside your search path.

The bootstrap fetches C2Rust 0.22.1 and verifies its Git revision and local patch. The patch handles recursive C types and preserves inclusive ranges in Emerald's switch statements. The bootstrap builds the translator with locked Cargo dependencies and keeps its source and binary under `target/c2rust/`. Run these commands from the project directory.

```sh
python3 tools/c2rust/bootstrap.py
python3 tools/translate.py
python3 tools/translate.py --check
cargo test --release
```

To compare the port with the C game build, run the command below. Both executables receive the same input and clock. The comparison checks the framebuffer after every frame from the moving truck through the first battle and a soft reset. With `--all-starters`, it verifies that Birch gives the player Mudkip, Treecko and Torchic on their respective routes.

The tests also check 1,158 serialized party Pokémon across all 386 species at levels 5, 50 and 100. Each build writes and reloads a 128 KiB flash save, and the comparison checks that the two files match byte for byte.

```sh
python3 tools/compare-replay.py --all-starters
```

To capture named checkpoints from the starter replay as 240×160 PPM images, run the replay directly.

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
