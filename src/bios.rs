use std::{ptr, slice};

// The BIOS ABI supplies a pointer to a readable stream and writable output,
// but no compressed length. Walk the tokens to find the slice's exact end.
unsafe fn lz77_uncomp(src: *const u8, dest: *mut u8) {
    unsafe {
        let header = src.cast::<[u8; 4]>().read();
        let mut remaining = u32::from_le_bytes([header[1], header[2], header[3], 0]) as usize;
        // Vanilla Emerald passes raw trainer back sprites whose first
        // four bytes look like a zero-length header to the BIOS.
        if remaining == 0 {
            return;
        }
        assert_eq!(header[0], 0x10, "invalid LZ10 compression type");

        let mut input_len = 4;
        while remaining != 0 {
            let flags = src.add(input_len).read();
            input_len += 1;
            for bit in (0..8).rev() {
                if flags & (1 << bit) == 0 {
                    input_len += 1;
                    remaining -= 1;
                } else {
                    let length = usize::from(src.add(input_len).read() >> 4) + 3;
                    input_len += 2;

                    // Include the whole token. The decoder clips the bytes
                    // it produces at the declared output size.
                    remaining = remaining.saturating_sub(length);
                }
                if remaining == 0 {
                    break;
                }
            }
        }

        let decoded =
            pokemon_sprite_compression::gen3::decompress(slice::from_raw_parts(src, input_len));
        ptr::copy_nonoverlapping(decoded.as_ptr(), dest, decoded.len());
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn LZ77UnCompWram(src: *const u8, dest: *mut u8) {
    unsafe { lz77_uncomp(src, dest) };
}

#[unsafe(no_mangle)]
unsafe extern "C" fn LZ77UnCompVram(src: *const u8, dest: *mut u8) {
    unsafe { lz77_uncomp(src, dest) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha1::{Digest, Sha1};
    use std::path::Path;

    #[test]
    fn zero_length_headers_leave_the_destination_untouched() {
        for input in [[0; 4], [0x10, 0, 0, 0]] {
            for decompress in [LZ77UnCompWram, LZ77UnCompVram] {
                let mut output = [0xaa; 4];
                unsafe { decompress(input.as_ptr(), output.as_mut_ptr()) };
                assert_eq!(output, [0xaa; 4]);
            }
        }
    }

    #[test]
    fn clips_a_final_match_to_the_declared_size() {
        // One literal and a three-byte match at distance 1. Only two match
        // bytes fit in the declared output.
        let input = [0x10, 3, 0, 0, 0x40, 0x42, 0, 0];
        let mut output = [0xff; 4];
        unsafe { LZ77UnCompWram(input.as_ptr(), output.as_mut_ptr()) };
        assert_eq!(output, [0x42, 0x42, 0x42, 0xff]);
    }

    #[test]
    #[ignore = "requires the English Emerald revision 0 ROM"]
    fn decompresses_battle_sprites_from_emerald_rom() {
        let rom = crate::rom::read(Path::new(crate::rom::DEFAULT_FILE)).unwrap();
        crate::rom::validate(&rom).unwrap();

        // Pikachu's still and animated front graphics, with raw tile hashes
        // from the corresponding pret/pokeemerald gbagfx outputs.
        for (offset, size, expected) in [
            (0xc40254, 2048, "4965e9653db5b3a71e484d1c7b29fc00dd9bfbab"),
            (0xb09964, 4096, "500ab62cf24a5f5fdfa422e5e394c01f9a8ac3f6"),
        ] {
            let mut output = vec![0xff; size + 1];
            for decompress in [LZ77UnCompWram, LZ77UnCompVram] {
                unsafe { decompress(rom[offset..].as_ptr(), output.as_mut_ptr()) };
                assert_eq!(
                    format!("{:x}", Sha1::digest(&output[..size])),
                    expected,
                    "sprite at {offset:#x}",
                );
                assert_eq!(output[size], 0xff);
                output.fill(0xff);
            }
        }
    }
}
