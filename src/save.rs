//! Emerald's 128 KiB cartridge flash. The first 28 sectors hold two rotating
//! copies of the adventure. The last four hold Hall of Fame and other records.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

const FLASH_SIZE: usize = 128 * 1024;
const SECTOR_SIZE: usize = 0x1000;
const SECTION_SIZE: usize = 0xf80;
const BANK_SECTORS: usize = 14;
const BANK_SIZE: usize = SECTOR_SIZE * BANK_SECTORS;
const SIGNATURE: u32 = 0x0801_2025;

// SaveBlock2, four chunks of SaveBlock1, and nine chunks of PokemonStorage.
// Only the occupied part of each sector is included in the game's checksum.
const SECTION_LENGTHS: [usize; BANK_SECTORS] = [
    0xf2c,
    SECTION_SIZE,
    SECTION_SIZE,
    SECTION_SIZE,
    0xf08,
    SECTION_SIZE,
    SECTION_SIZE,
    SECTION_SIZE,
    SECTION_SIZE,
    SECTION_SIZE,
    SECTION_SIZE,
    SECTION_SIZE,
    SECTION_SIZE,
    0x7d0,
];

fn u16_at(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(data[offset..offset + 2].try_into().unwrap())
}

fn u32_at(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

fn checksum(data: &[u8]) -> u16 {
    let sum = data.chunks_exact(4).fold(0u32, |sum, word| {
        sum.wrapping_add(u32::from_le_bytes(word.try_into().unwrap()))
    });
    ((sum >> 16) as u16).wrapping_add(sum as u16)
}

struct Bank<'a> {
    counter: u32,
    sections: [&'a [u8]; BANK_SECTORS],
}

impl<'a> Bank<'a> {
    fn read(data: &'a [u8]) -> Option<Self> {
        let mut sections: [&[u8]; BANK_SECTORS] = [&[]; BANK_SECTORS];
        let counter = u32_at(data, 0xffc);
        for sector in data.chunks_exact(SECTOR_SIZE) {
            let id = usize::from(u16_at(sector, 0xff4));
            let section = &sector[..*SECTION_LENGTHS.get(id)?];
            if !sections[id].is_empty()
                || u32_at(sector, 0xff8) != SIGNATURE
                || u32_at(sector, 0xffc) != counter
                || checksum(section) != u16_at(sector, 0xff6)
            {
                return None;
            }
            sections[id] = section;
        }
        Some(Self { counter, sections })
    }

    fn latest(flash: &'a [u8]) -> Option<Self> {
        let first = Self::read(&flash[..BANK_SIZE]);
        let second = Self::read(&flash[BANK_SIZE..2 * BANK_SIZE]);
        match (first, second) {
            (Some(a), Some(b)) => {
                // Emerald's counter wraps from u32::MAX to zero.
                let second_is_newer = match (a.counter, b.counter) {
                    (u32::MAX, 0) => true,
                    (0, u32::MAX) => false,
                    (a, b) => b > a,
                };
                Some(if second_is_newer { b } else { a })
            }
            (a, b) => a.or(b),
        }
    }

    fn summary(&self) -> String {
        let trainer = self.sections[0];
        let name: String = trainer[..7]
            .iter()
            .take_while(|&&ch| ch != 0xff)
            .map(|&ch| trainer_char(ch))
            .collect();
        let hours = u16_at(trainer, 0xe);
        let minutes = trainer[0x10];
        // SaveBlock1.flags starts at 0x1270 in the block, inside section 2.
        let flags = &self.sections[2][0x1270 - SECTION_SIZE..];
        let badges = (0x867..=0x86e)
            .filter(|&flag| flags[flag / 8] & (1 << (flag % 8)) != 0)
            .count();
        let badge_label = if badges == 1 { "badge" } else { "badges" };
        format!("{name} · {hours}:{minutes:02} · {badges} {badge_label}")
    }
}

// Characters available on the English Emerald trainer naming screen.
fn trainer_char(ch: u8) -> char {
    match ch {
        0 => ' ',
        0xa1..=0xaa => (b'0' + (ch - 0xa1)) as char,
        0xbb..=0xd4 => (b'A' + (ch - 0xbb)) as char,
        0xd5..=0xee => (b'a' + (ch - 0xd5)) as char,
        0xab => '!',
        0xac => '?',
        0xad => '.',
        0xae => '-',
        0xb0 => '…',
        0xb1 => '“',
        0xb2 => '”',
        0xb3 => '‘',
        0xb4 => '’',
        0xb5 => '♂',
        0xb6 => '♀',
        0xb8 => ',',
        0xba => '/',
        _ => '�',
    }
}

fn read_flash(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    if !metadata.is_file() {
        return Err(format!("{} is not a regular save file", path.display()));
    }
    let mut data = Vec::with_capacity(FLASH_SIZE);
    File::open(path)
        .and_then(|file| file.take((FLASH_SIZE + 1) as u64).read_to_end(&mut data))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if data.len() != FLASH_SIZE {
        return Err(format!(
            "{} must contain exactly 128 KiB of raw flash",
            path.display()
        ));
    }
    Ok(Some(data))
}

fn summary(data: &[u8]) -> Result<String, String> {
    if let Some(bank) = Bank::latest(data) {
        Ok(bank.summary())
    } else if data.iter().all(|&byte| byte == 0xff) {
        Ok("Empty cartridge".to_string())
    } else {
        Err("No complete Emerald save bank with valid checksums".to_string())
    }
}

pub fn list(directory: &Path) -> Result<Vec<(String, String)>, String> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", directory.display())),
    };
    let mut saves = Vec::new();
    for entry in entries {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("sav") {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|name| name.to_str()) else {
            continue;
        };
        let preview = match read_flash(&path) {
            Ok(Some(data)) => summary(&data).unwrap_or_else(|e| e),
            Ok(None) => continue,
            Err(e) => e,
        };
        saves.push((name.to_owned(), preview));
    }
    saves.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(saves)
}

pub struct SaveFile {
    name: String,
    path: PathBuf,
    data: Option<Vec<u8>>,
    // Lock a stable sibling inode; the save itself is atomically replaced.
    // OS locks are released on process exit, including abnormal termination.
    _lock: File,
}

impl SaveFile {
    pub fn open(directory: &Path, name: &str) -> Result<Self, String> {
        if name.trim().is_empty()
            || matches!(name, "." | "..")
            || name.contains(['/', '\\', ':'])
            || name.chars().any(char::is_control)
        {
            return Err("Choose a save name without path separators or control characters".into());
        }
        fs::create_dir_all(directory).map_err(|e| format!("{}: {e}", directory.display()))?;
        let path = directory.join(format!("{name}.sav"));
        let lock = File::options()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join(format!(".{name}.lock")))
            .map_err(|e| format!("Cannot open save lock for {name}: {e}"))?;
        lock.try_lock()
            .map_err(|e| format!("Cannot lock save {name}: {e}"))?;
        let data = read_flash(&path)?;
        if let Some(bytes) = data.as_deref() {
            summary(bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        Ok(Self {
            name: name.to_owned(),
            path,
            data,
            _lock: lock,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn bytes(&self) -> Option<&[u8]> {
        self.data.as_deref()
    }

    pub fn flush(&mut self, data: &[u8]) -> Result<(), String> {
        if data.len() != FLASH_SIZE {
            return Err(format!(
                "Expected 128 KiB of flash, received {} bytes",
                data.len()
            ));
        }
        if self.bytes() == Some(data) {
            return Ok(());
        }
        // Flash is programmed a sector at a time. A first save may be only
        // partially written when the frontend checks for changes.
        if Bank::latest(data).is_none() {
            let erased = data.iter().all(|&byte| byte == 0xff);
            if !erased || self.data.is_none() {
                return Ok(());
            }
        }
        if read_flash(&self.path)? != self.data {
            return Err(format!(
                "{} changed on disk during play",
                self.path.display()
            ));
        }
        let write = || -> io::Result<()> {
            let mut temp = NamedTempFile::new_in(self.path.parent().unwrap())?;
            temp.write_all(data)?;
            temp.as_file().sync_all()?;
            if self.data.is_none() {
                temp.persist_noclobber(&self.path)?;
            } else {
                temp.persist(&self.path)?;
            }
            Ok(())
        };
        write().map_err(|e| format!("Cannot write {}: {e}", self.path.display()))?;
        self.data = Some(data.to_vec());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Build a rotated Emerald bank with a short trainer name and two badges
    // spanning the flag-byte boundary. Leave the other bank erased.
    fn write_bank(flash: &mut [u8], slot: usize, counter: u32, hour: u16) {
        for (id, length) in SECTION_LENGTHS.iter().copied().enumerate() {
            let physical = slot * BANK_SECTORS + (id + 5) % BANK_SECTORS;
            let sector = &mut flash[physical * SECTOR_SIZE..(physical + 1) * SECTOR_SIZE];
            sector.fill(0);
            match id {
                0 => {
                    sector[..4].copy_from_slice(&[0xc7, 0xd5, 0xed, 0xff]); // May
                    sector[0xe..0x10].copy_from_slice(&hour.to_le_bytes());
                    sector[0x10] = 9;
                }
                2 => sector[0x3fc..0x3fe].copy_from_slice(&[0x80, 0x40]),
                _ => (),
            }
            let check = checksum(&sector[..length]);
            sector[0xff4..0xff6].copy_from_slice(&(id as u16).to_le_bytes());
            sector[0xff6..0xff8].copy_from_slice(&check.to_le_bytes());
            sector[0xff8..0xffc].copy_from_slice(&SIGNATURE.to_le_bytes());
            sector[0xffc..].copy_from_slice(&counter.to_le_bytes());
        }
    }

    #[test]
    fn newest_complete_rotated_bank_supplies_the_preview() {
        let mut flash = vec![0xff; FLASH_SIZE];
        write_bank(&mut flash, 1, u32::MAX, 45);
        write_bank(&mut flash, 0, 0, 46);
        assert_eq!(summary(&flash).unwrap(), "May · 46:09 · 2 badges");
        // Corrupt a PC-storage section; the trainer section itself is intact.
        flash[SECTOR_SIZE + 100] ^= 1;
        assert_eq!(summary(&flash).unwrap(), "May · 45:09 · 2 badges");
        flash[BANK_SIZE + 0xff8] = 0;
        assert!(summary(&flash).is_err());
    }

    #[test]
    fn malformed_sector_ids_and_duplicate_sections_are_rejected() {
        let mut flash = vec![0xff; FLASH_SIZE];
        write_bank(&mut flash, 0, 0, 3);
        flash[0xff4..0xff6].copy_from_slice(&14u16.to_le_bytes());
        assert!(summary(&flash).is_err());
        flash[0xff4..0xff6].copy_from_slice(&10u16.to_le_bytes());
        assert!(summary(&flash).is_err());
        flash[0xff4..0xff6].copy_from_slice(&9u16.to_le_bytes());
        flash[0xffc] = 1;
        assert!(summary(&flash).is_err());
    }

    #[test]
    fn sessions_round_trip_raw_flash_and_keep_other_saves_independent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Hoenn.sav");
        let mut game = SaveFile::open(dir.path(), "Hoenn").unwrap();
        assert_eq!(game.name(), "Hoenn");
        assert!(SaveFile::open(dir.path(), "Hoenn").is_err());
        let other = SaveFile::open(dir.path(), "Hoenn 2").unwrap();
        let mut flash = vec![0xff; FLASH_SIZE];
        game.flush(&flash).unwrap();
        write_bank(&mut flash, 1, 1, 36);
        let mut partial = flash.clone();
        partial[BANK_SIZE..BANK_SIZE + SECTOR_SIZE].fill(0xff);
        game.flush(&partial).unwrap();
        assert!(!path.exists());
        game.flush(&flash).unwrap();
        assert_eq!(fs::read(&path).unwrap(), flash);
        assert_eq!(
            list(dir.path()).unwrap(),
            [("Hoenn".into(), "May · 36:09 · 2 badges".into())]
        );
        drop(game);
        assert_eq!(
            SaveFile::open(dir.path(), "Hoenn").unwrap().bytes(),
            Some(&flash[..])
        );
        assert_eq!(other.bytes(), None);
    }

    #[test]
    fn bad_files_and_external_changes_are_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Journey.sav");
        let mut game = SaveFile::open(dir.path(), "Journey").unwrap();
        let mut flash = vec![0xff; FLASH_SIZE];
        write_bank(&mut flash, 1, 1, 5);
        game.flush(&flash).unwrap();
        let corrupt = vec![0x55; FLASH_SIZE];
        fs::write(&path, &corrupt).unwrap();
        write_bank(&mut flash, 0, 2, 6);
        assert!(game.flush(&flash).is_err());
        drop(game);
        assert!(SaveFile::open(dir.path(), "Journey").is_err());
        assert_eq!(fs::read(&path).unwrap(), corrupt);
        fs::write(&path, [5, 6]).unwrap();
        assert!(SaveFile::open(dir.path(), "Journey").is_err());
        assert_eq!(fs::read(path).unwrap(), [5, 6]);
        assert!(SaveFile::open(dir.path(), "../elsewhere").is_err());
    }
}
