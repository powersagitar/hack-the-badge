//! Parser for the ESP-IDF app image format (`esp_image_header_t` +
//! interleaved segment headers/data), as used by `public/firmware/factory.bin`.
//!
//! This is a from-scratch reimplementation of the *shape* of ESP-IDF's
//! `esp_image_header_t`/`esp_image_segment_header_t` structs (documented in
//! ESP-IDF's `esp_app_format.h`), not a copy of any ESP-IDF source — we only
//! need enough of the format to locate each segment's `load_addr` and raw
//! bytes within the image file, not to verify checksums/signatures.
//!
//! Layout of the whole image (confirmed against `factory.bin`'s real bytes,
//! see the task brief/report):
//! ```text
//! [ 24-byte esp_image_header_t ]
//! [ segment 0 header (8 bytes) ][ segment 0 data (data_len bytes) ]
//! [ segment 1 header (8 bytes) ][ segment 1 data (data_len bytes) ]
//! ... (repeated `segment_count` times total) ...
//! [ 1 checksum byte ][ zero padding to 16-byte boundary ][ 32-byte SHA-256 ]
//! ```
//! We stop after `segment_count` segments and never attempt to interpret the
//! trailing checksum/hash bytes as another segment.

/// Size in bytes of the fixed `esp_image_header_t` prefix.
pub const IMAGE_HEADER_LEN: usize = 24;

/// Size in bytes of one `esp_image_segment_header_t` (`load_addr` +
/// `data_len`, both little-endian `u32`).
pub const SEGMENT_HEADER_LEN: usize = 8;

/// Required value of `esp_image_header_t::magic` for a valid ESP-IDF app
/// image.
pub const IMAGE_MAGIC: u8 = 0xE9;

/// The fixed-size header at the start of an ESP-IDF app image
/// (`esp_image_header_t`, 24 bytes). Field layout/order/sizes below are
/// taken from ESP-IDF's `esp_app_format.h`; we parse the struct fully
/// (rather than only the fields this task's boot path needs) so a
/// misunderstanding of one field's width can't silently misalign the ones
/// after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageHeader {
    pub magic: u8,
    pub segment_count: u8,
    pub spi_mode: u8,
    /// Packed nibbles (spi speed / flash size); unused by this task.
    pub spi_speed_size: u8,
    pub entry_addr: u32,
    pub wp_pin: u8,
    pub spi_pin_drv: [u8; 3],
    pub chip_id: u16,
    pub min_chip_rev: u8,
    pub min_chip_rev_full: u16,
    pub max_chip_rev_full: u16,
    pub reserved: [u8; 4],
    /// If non-zero, a 32-byte SHA-256 hash follows the last segment's data
    /// (after 1 checksum byte + zero-padding to a 16-byte boundary). This
    /// parser doesn't need to read that trailing hash, but callers that want
    /// to locate it can use this flag plus the offset returned alongside the
    /// last parsed segment.
    pub hash_appended: u8,
}

/// One segment's location within the image file plus its ESP32-C3 load
/// address. Deliberately doesn't own a copy of the segment's bytes — the
/// caller (see `crate::mem::bus::FirmwareBus`) decides whether to reference
/// the original image bytes in place (XIP) or copy them into a fresh RAM
/// buffer, based on [`SegmentDescriptor::load_addr`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentDescriptor {
    pub load_addr: u32,
    /// Byte offset of this segment's data within the original image buffer.
    pub file_offset: usize,
    pub len: usize,
}

/// Everything [`parse_image`] extracts from an app image: the header plus
/// every segment descriptor (exactly `header.segment_count` of them).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedImage {
    pub header: ImageHeader,
    pub segments: Vec<SegmentDescriptor>,
}

/// Everything that can go wrong parsing an image. Deliberately returned as a
/// `Result` rather than panicking — a corrupt/truncated image is bad input,
/// not a programming bug, and (per this task's "never panic" rule for the
/// bus) the same care applies to the parser that feeds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageParseError {
    /// Fewer than [`IMAGE_HEADER_LEN`] bytes total.
    TooShortForHeader { len: usize },
    /// `magic` byte wasn't [`IMAGE_MAGIC`].
    BadMagic { found: u8 },
    /// Not enough bytes left to read segment `index`'s 8-byte header.
    SegmentHeaderOutOfBounds { index: u8, file_offset: usize },
    /// Segment `index`'s header claims more data than remains in the image.
    SegmentDataOutOfBounds {
        index: u8,
        file_offset: usize,
        data_len: usize,
        image_len: usize,
    },
}

/// Parses an ESP-IDF app image's header and segment table. Does not
/// validate the trailing checksum/SHA-256 (out of scope for booting).
pub fn parse_image(bytes: &[u8]) -> Result<ParsedImage, ImageParseError> {
    if bytes.len() < IMAGE_HEADER_LEN {
        return Err(ImageParseError::TooShortForHeader { len: bytes.len() });
    }

    let magic = bytes[0];
    if magic != IMAGE_MAGIC {
        return Err(ImageParseError::BadMagic { found: magic });
    }

    let header = ImageHeader {
        magic,
        segment_count: bytes[1],
        spi_mode: bytes[2],
        spi_speed_size: bytes[3],
        entry_addr: u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
        wp_pin: bytes[8],
        spi_pin_drv: [bytes[9], bytes[10], bytes[11]],
        chip_id: u16::from_le_bytes(bytes[12..14].try_into().unwrap()),
        min_chip_rev: bytes[14],
        min_chip_rev_full: u16::from_le_bytes(bytes[15..17].try_into().unwrap()),
        max_chip_rev_full: u16::from_le_bytes(bytes[17..19].try_into().unwrap()),
        reserved: [bytes[19], bytes[20], bytes[21], bytes[22]],
        hash_appended: bytes[23],
    };

    let mut offset = IMAGE_HEADER_LEN;
    let mut segments = Vec::with_capacity(header.segment_count as usize);

    for index in 0..header.segment_count {
        if offset + SEGMENT_HEADER_LEN > bytes.len() {
            return Err(ImageParseError::SegmentHeaderOutOfBounds {
                index,
                file_offset: offset,
            });
        }
        let load_addr = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        let data_len =
            u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
        offset += SEGMENT_HEADER_LEN;

        if offset + data_len > bytes.len() {
            return Err(ImageParseError::SegmentDataOutOfBounds {
                index,
                file_offset: offset,
                data_len,
                image_len: bytes.len(),
            });
        }
        segments.push(SegmentDescriptor {
            load_addr,
            file_offset: offset,
            len: data_len,
        });
        offset += data_len;
    }

    Ok(ParsedImage { header, segments })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a synthetic image: a 24-byte header (magic/segment_count/
    /// entry_addr set, everything else zeroed) followed by the given
    /// segments (each as `(load_addr, data)`), interleaved
    /// header-then-data per the real format, plus a trailing byte after the
    /// last segment that must NOT be parsed as a 7th segment.
    fn build_synthetic_image(entry_addr: u32, segments: &[(u32, &[u8])]) -> Vec<u8> {
        let mut buf = vec![0u8; IMAGE_HEADER_LEN];
        buf[0] = IMAGE_MAGIC;
        buf[1] = segments.len() as u8;
        buf[4..8].copy_from_slice(&entry_addr.to_le_bytes());

        for (load_addr, data) in segments {
            buf.extend_from_slice(&load_addr.to_le_bytes());
            buf.extend_from_slice(&(data.len() as u32).to_le_bytes());
            buf.extend_from_slice(data);
        }

        // Trailing "checksum + hash"-ish bytes that a buggy parser might
        // misread as an extra segment header.
        buf.extend_from_slice(&[0xAA; 40]);
        buf
    }

    #[test]
    fn parses_header_fields_in_order() {
        let mut bytes = vec![0u8; IMAGE_HEADER_LEN];
        bytes[0] = 0xe9;
        bytes[1] = 6; // segment_count
        bytes[2] = 0x02; // spi_mode
        bytes[3] = 0x2f; // spi_speed_size
        bytes[4..8].copy_from_slice(&0x403803fcu32.to_le_bytes()); // entry_addr
        bytes[8] = 0xee; // wp_pin
        bytes[9..12].copy_from_slice(&[0x00, 0x00, 0x00]); // spi_pin_drv
        bytes[12..14].copy_from_slice(&5u16.to_le_bytes()); // chip_id (ESP32-C3)
        bytes[14] = 0x03; // min_chip_rev
        bytes[15..17].copy_from_slice(&0x0003u16.to_le_bytes()); // min_chip_rev_full
        bytes[17..19].copy_from_slice(&0x00c7u16.to_le_bytes()); // max_chip_rev_full
        bytes[19..23].copy_from_slice(&[0, 0, 0, 0]); // reserved
        bytes[23] = 1; // hash_appended

        // segment_count=6, but we only care about header-field parsing
        // here, so pad with 6 zero-length segment headers (load_addr=0,
        // data_len=0) rather than real segment data.
        for _ in 0..6 {
            bytes.extend_from_slice(&[0u8; SEGMENT_HEADER_LEN]);
        }

        let parsed = parse_image(&bytes).expect("valid header-only image parses");
        assert_eq!(parsed.header.magic, 0xe9);
        assert_eq!(parsed.header.segment_count, 6);
        assert_eq!(parsed.header.entry_addr, 0x403803fc);
        assert_eq!(parsed.header.wp_pin, 0xee);
        assert_eq!(parsed.header.chip_id, 5);
        assert_eq!(parsed.header.min_chip_rev, 3);
        assert_eq!(parsed.header.max_chip_rev_full, 0x00c7);
        assert_eq!(parsed.header.hash_appended, 1);
        assert_eq!(parsed.segments.len(), 6);
        assert!(parsed.segments.iter().all(|s| s.len == 0));
    }

    #[test]
    fn matches_real_factory_bin_first_32_bytes() {
        // Exactly the first 32 bytes of public/firmware/factory.bin, per the
        // task brief's confirmed ground truth: 24-byte header + segment 0's
        // 8-byte header (load_addr=0x3c130020, data_len=1_277_576).
        let bytes: [u8; 32] = [
            0xe9, 0x06, 0x02, 0x2f, 0xfc, 0x03, 0x38, 0x40, 0xee, 0x00, 0x00, 0x00, 0x05, 0x00,
            0x03, 0x03, 0x00, 0xc7, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x20, 0x00, 0x13, 0x3c,
            0x88, 0x7e, 0x13, 0x00,
        ];
        // Segment 0's data isn't present in this 32-byte slice, so parsing
        // it fully would fail with SegmentDataOutOfBounds; here we just
        // check the header + segment-0-header fields parse correctly by
        // parsing only up to (not including) segment 0's declared data.
        let header_and_seg0_header = &bytes[..32];
        let err = parse_image(header_and_seg0_header).unwrap_err();
        match err {
            ImageParseError::SegmentDataOutOfBounds {
                index,
                data_len,
                image_len,
                ..
            } => {
                assert_eq!(index, 0);
                assert_eq!(data_len, 1_277_576);
                assert_eq!(image_len, 32);
            }
            other => panic!("expected SegmentDataOutOfBounds, got {other:?}"),
        }
    }

    #[test]
    fn extracts_load_addr_and_data_for_each_segment() {
        let seg0_data = [1u8, 2, 3, 4, 5];
        let seg1_data = [0xAAu8; 16];
        let seg2_data = [0xFFu8; 3];
        let image = build_synthetic_image(
            0x1000,
            &[
                (0x3c000010, &seg0_data),
                (0x3fc80000, &seg1_data),
                (0x50000000, &seg2_data),
            ],
        );

        let parsed = parse_image(&image).expect("synthetic image parses");
        assert_eq!(parsed.header.entry_addr, 0x1000);
        assert_eq!(parsed.segments.len(), 3);

        assert_eq!(parsed.segments[0].load_addr, 0x3c000010);
        assert_eq!(parsed.segments[0].len, 5);
        assert_eq!(
            &image[parsed.segments[0].file_offset..parsed.segments[0].file_offset + 5],
            &seg0_data
        );

        assert_eq!(parsed.segments[1].load_addr, 0x3fc80000);
        assert_eq!(parsed.segments[1].len, 16);
        assert_eq!(
            &image[parsed.segments[1].file_offset..parsed.segments[1].file_offset + 16],
            &seg1_data
        );

        assert_eq!(parsed.segments[2].load_addr, 0x50000000);
        assert_eq!(parsed.segments[2].len, 3);
        assert_eq!(
            &image[parsed.segments[2].file_offset..parsed.segments[2].file_offset + 3],
            &seg2_data
        );
    }

    #[test]
    fn stops_after_segment_count_and_ignores_trailing_checksum_hash_bytes() {
        // segment_count=1, but the buffer has trailing 0xAA bytes after that
        // one segment's data (standing in for checksum + padding + SHA-256).
        // A buggy parser might try to read those as a second segment header.
        let seg_data = [7u8, 8, 9];
        let image = build_synthetic_image(0, &[(0x42000000, &seg_data)]);

        let parsed = parse_image(&image).expect("parses despite trailing junk bytes");
        assert_eq!(parsed.segments.len(), 1);
        assert_eq!(parsed.segments[0].load_addr, 0x42000000);
        assert_eq!(parsed.segments[0].len, 3);
    }

    #[test]
    fn rejects_bad_magic() {
        let mut image = build_synthetic_image(0, &[]);
        image[0] = 0x00;
        assert_eq!(
            parse_image(&image),
            Err(ImageParseError::BadMagic { found: 0x00 })
        );
    }

    #[test]
    fn rejects_too_short_buffer() {
        assert_eq!(
            parse_image(&[0xe9, 0x01]),
            Err(ImageParseError::TooShortForHeader { len: 2 })
        );
    }

    #[test]
    fn rejects_truncated_segment_header() {
        // Valid 24-byte header claiming 1 segment, but with zero bytes
        // after it (not even a full 8-byte segment header).
        let mut image = vec![0u8; IMAGE_HEADER_LEN];
        image[0] = IMAGE_MAGIC;
        image[1] = 1;
        image.extend_from_slice(&[0x00, 0x00, 0x00]); // only 3 of 8 header bytes
        let err = parse_image(&image).unwrap_err();
        assert!(matches!(
            err,
            ImageParseError::SegmentHeaderOutOfBounds { index: 0, .. }
        ));
    }

    #[test]
    fn rejects_segment_claiming_more_data_than_remains() {
        let mut image = vec![0u8; IMAGE_HEADER_LEN];
        image[0] = IMAGE_MAGIC;
        image[1] = 1;
        image.extend_from_slice(&0x3c000000u32.to_le_bytes()); // load_addr
        image.extend_from_slice(&1000u32.to_le_bytes()); // data_len (way too big)
        image.extend_from_slice(&[0u8; 5]); // only 5 bytes actually present
        let err = parse_image(&image).unwrap_err();
        assert!(matches!(
            err,
            ImageParseError::SegmentDataOutOfBounds {
                index: 0,
                data_len: 1000,
                ..
            }
        ));
    }
}
