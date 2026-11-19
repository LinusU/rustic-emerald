//! Build the sample bank in Emerald's cry-table order. In particular Hoenn's
//! cry IDs do not follow either internal species IDs or National Dex numbers.
use std::{collections::HashMap, env, fs, path::PathBuf};

pub fn build() {
    let sound = PathBuf::from("vendor/pokeemerald/sound");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let definitions = fs::read_to_string(sound.join("direct_sound_data.inc")).unwrap();
    let mut files = HashMap::new();
    let mut lines = definitions.lines().map(str::trim);
    while let Some(line) = lines.next() {
        if let Some(label) = line.strip_suffix("::").filter(|s| s.starts_with("Cry_")) {
            let path = lines
                .next()
                .unwrap()
                .strip_prefix(".incbin \"")
                .unwrap()
                .strip_suffix(".bin\"")
                .unwrap();
            files.insert(
                label,
                PathBuf::from("vendor/pokeemerald")
                    .join(path)
                    .with_extension("wav"),
            );
        }
    }

    let table = fs::read_to_string(sound.join("cry_tables.inc")).unwrap();
    let mut bank = Vec::new();
    let mut index = String::from("const CRIES: &[(usize, usize)] = &[\n");
    for label in table
        .lines()
        .filter_map(|line| line.trim().strip_prefix("cry "))
    {
        let path = &files[label];
        let wav = fs::read(path).unwrap();
        assert!(wav.starts_with(b"RIFF") && wav[8..12] == *b"WAVE");
        let mut format_ok = false;
        let mut samples = None;
        let mut offset = 12;
        while offset + 8 <= wav.len() {
            let size = u32::from_le_bytes(wav[offset + 4..offset + 8].try_into().unwrap()) as usize;
            let chunk = &wav[offset + 8..offset + 8 + size];
            match &wav[offset..offset + 4] {
                b"fmt " => {
                    format_ok = chunk[0..4] == [1, 0, 1, 0] // PCM, mono
                        && chunk[4..8] == 10512u32.to_le_bytes() && chunk[14..16] == [8, 0];
                }
                b"data" => samples = Some(chunk),
                _ => (),
            }
            offset += 8 + size + size % 2;
        }
        assert!(
            format_ok,
            "unsupported cry WAV format in {}",
            path.display()
        );
        let samples = samples.expect("missing cry WAV data");
        assert!(!samples.is_empty());
        index.push_str(&format!("    ({}, {}),\n", bank.len(), samples.len()));
        bank.extend_from_slice(samples);
    }
    assert!(!bank.is_empty(), "empty Emerald cry table");
    index.push_str("];\n");
    fs::write(out.join("cry_bank.rs"), index).unwrap();
    fs::write(out.join("cry_bank.pcm"), bank).unwrap();
}
