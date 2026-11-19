use sha1::{Digest, Sha1};
use std::{fs, path::Path};

pub const DEFAULT_FILE: &str = "pokeemerald.gba";
pub const SHA1: &str = "f3ae088181bf583e55daf962a92bb46f4f1d07b7";

pub fn read(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|err| format!("could not read {}: {err}", path.display()))
}

// Native code hooks and ROM data offsets are specific to this cartridge revision.
pub fn validate(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() != 0x1000000 {
        return Err(format!(
            "expected a 16 MiB English Pokémon Emerald ROM, got {} bytes",
            bytes.len()
        ));
    }
    let hash = format!("{:x}", Sha1::digest(bytes));
    if hash != SHA1 {
        return Err(format!(
            "unsupported Emerald ROM (SHA-1 {hash}, expected {SHA1})"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_truncated_rom() {
        assert!(validate(b"BPEE").is_err());
    }

    #[test]
    fn rejects_modified_cartridge_of_the_correct_size() {
        assert!(validate(&vec![0; 0x1000000]).unwrap_err().contains("SHA-1"));
    }
}
