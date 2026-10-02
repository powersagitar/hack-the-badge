//! MD5 (RFC 1321), in the shape of Colin Plumb's public-domain
//! implementation: a `struct MD5Context { uint32_t buf[4]; uint32_t bits[2];
//! uint8_t in[64]; }` driven by `MD5Init`/`MD5Update`/`MD5Final`.
//!
//! Chip-agnostic. It has two users:
//!
//! - `crate::peripherals::flash`'s tests, which recompute the synthesized
//!   partition table's MD5 entry ([`md5`]).
//! - The ROM MD5 stubs (`crate::cpu::rom_stubs::RomStubEffect::Md5`), which
//!   load a guest `md5_context_t` through the bus into an [`Md5Context`],
//!   apply one call, and store it back. ESP-IDF v5.5.3's
//!   `components/esp_rom/include/esp_rom_md5.h` declares that type, for
//!   every target but the ESP32-C2, as exactly Plumb's struct: `uint32_t
//!   buf[4]; uint32_t bits[2]; uint8_t in[64];`, 88 bytes
//!   ([`CONTEXT_LEN`]). The ESP32-C3 mask ROM's own `MD5Init`/`MD5Update`/
//!   `MD5Final` (`0x400369d8`/`0x40036a0a`/`0x40036ad2` in Espressif's
//!   `esp32c3_rev3_rom.elf`) are that code, compiled: the disassembly
//!   matches [`Md5Context::init`], [`Md5Context::update`] and
//!   [`Md5Context::finalize`] step for step, including which bytes of the
//!   context each one writes. So a stubbed call leaves guest memory as the
//!   real ROM would, and a partial block buffered by one `MD5Update`
//!   survives in guest memory for the next.
//!
//! On a little-endian host Plumb's `byteReverse` is a no-op, and the
//! context's words are stored little-endian, as the RV32 guest stores them.

/// `ESP_ROM_MD5_DIGEST_LEN`.
pub const DIGEST_LEN: usize = 16;
/// MD5's block size.
pub const BLOCK_LEN: usize = 64;
/// `sizeof(md5_context_t)`: `buf[4]` (16) + `bits[2]` (8) + `in[64]`.
pub const CONTEXT_LEN: usize = 88;
/// Byte offset of `bits` in `md5_context_t`.
pub const BITS_OFFSET: usize = 16;
/// Byte offset of `in` in `md5_context_t`.
pub const IN_OFFSET: usize = 24;
/// The bytes `MD5Init` writes: `buf` and `bits` (it leaves `in` alone).
pub const INIT_WRITE_LEN: usize = IN_OFFSET;

/// RFC 1321 section 3.3's initial chaining value (`MD5Init`'s `buf`).
pub const INITIAL_STATE: [u32; 4] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];

/// Per-step left-rotate amounts (RFC 1321 section 3.4).
const S: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9,
    14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15,
    21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

/// `T[i] = floor(|sin(i + 1)| * 2^32)` (RFC 1321 section 3.4).
const K: [u32; 64] = [
    0xd76a_a478,
    0xe8c7_b756,
    0x2420_70db,
    0xc1bd_ceee,
    0xf57c_0faf,
    0x4787_c62a,
    0xa830_4613,
    0xfd46_9501,
    0x6980_98d8,
    0x8b44_f7af,
    0xffff_5bb1,
    0x895c_d7be,
    0x6b90_1122,
    0xfd98_7193,
    0xa679_438e,
    0x49b4_0821,
    0xf61e_2562,
    0xc040_b340,
    0x265e_5a51,
    0xe9b6_c7aa,
    0xd62f_105d,
    0x0244_1453,
    0xd8a1_e681,
    0xe7d3_fbc8,
    0x21e1_cde6,
    0xc337_07d6,
    0xf4d5_0d87,
    0x455a_14ed,
    0xa9e3_e905,
    0xfcef_a3f8,
    0x676f_02d9,
    0x8d2a_4c8a,
    0xfffa_3942,
    0x8771_f681,
    0x6d9d_6122,
    0xfde5_380c,
    0xa4be_ea44,
    0x4bde_cfa9,
    0xf6bb_4b60,
    0xbebf_bc70,
    0x289b_7ec6,
    0xeaa1_27fa,
    0xd4ef_3085,
    0x0488_1d05,
    0xd9d4_d039,
    0xe6db_99e5,
    0x1fa2_7cf8,
    0xc4ac_5665,
    0xf429_2244,
    0x432a_ff97,
    0xab94_23a7,
    0xfc93_a039,
    0x655b_59c3,
    0x8f0c_cc92,
    0xffef_f47d,
    0x8584_5dd1,
    0x6fa8_7e4f,
    0xfe2c_e6e0,
    0xa301_4314,
    0x4e08_11a1,
    0xf753_7e82,
    0xbd3a_f235,
    0x2ad7_d2bb,
    0xeb86_d391,
];

/// The MD5 compression function (Plumb's `MD5Transform`): folds one
/// 64-byte block into the chaining value.
pub fn transform(state: &mut [u32; 4], block: &[u8; BLOCK_LEN]) {
    let m: [u32; 16] = core::array::from_fn(|i| {
        u32::from_le_bytes([
            block[4 * i],
            block[4 * i + 1],
            block[4 * i + 2],
            block[4 * i + 3],
        ])
    });
    let [mut a, mut b, mut c, mut d] = *state;
    for i in 0..64 {
        let (f, g) = match i / 16 {
            0 => ((b & c) | (!b & d), i),
            1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
            2 => (b ^ c ^ d, (3 * i + 5) % 16),
            _ => (c ^ (b | !d), (7 * i) % 16),
        };
        let rotated = a
            .wrapping_add(f)
            .wrapping_add(K[i])
            .wrapping_add(m[g])
            .rotate_left(S[i]);
        a = d;
        d = c;
        c = b;
        b = b.wrapping_add(rotated);
    }
    for (h, v) in state.iter_mut().zip([a, b, c, d]) {
        *h = h.wrapping_add(v);
    }
}

/// Plumb's `struct MD5Context`, field for field (see the module doc).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Md5Context {
    /// The chaining value.
    pub buf: [u32; 4],
    /// The message length in bits, mod 2^64, low word first.
    pub bits: [u32; 2],
    /// The partial-block buffer.
    pub input: [u8; BLOCK_LEN],
}

impl Default for Md5Context {
    fn default() -> Self {
        Self::new()
    }
}

impl Md5Context {
    /// A freshly initialized context (`in` zeroed).
    pub fn new() -> Self {
        let mut ctx = Self {
            buf: [0; 4],
            bits: [0; 2],
            input: [0; BLOCK_LEN],
        };
        ctx.init();
        ctx
    }

    /// `MD5Init`: the initial chaining value and a zero bit count. Like the
    /// ROM, it does not touch `in`.
    pub fn init(&mut self) {
        self.buf = INITIAL_STATE;
        self.bits = [0; 2];
    }

    /// `MD5Update`: counts `data.len() * 8` bits into `bits` (32-bit
    /// arithmetic with a carry into `bits[1]`, plus `len >> 29`, as the C
    /// does), tops up a partial block in `in`, compresses each full block
    /// (copying it into `in` first, as the C's `memcpy` does), and leaves
    /// the tail at the start of `in`. Split-invariant: any split of the
    /// input across calls gives the same digest.
    pub fn update(&mut self, mut data: &[u8]) {
        // The C's `len` is a 32-bit `unsigned`.
        let len = data.len() as u32;
        let t = self.bits[0];
        self.bits[0] = t.wrapping_add(len << 3);
        if self.bits[0] < t {
            self.bits[1] = self.bits[1].wrapping_add(1);
        }
        self.bits[1] = self.bits[1].wrapping_add(len >> 29);

        let used = ((t >> 3) & 0x3f) as usize;
        if used != 0 {
            let room = BLOCK_LEN - used;
            if data.len() < room {
                self.input[used..used + data.len()].copy_from_slice(data);
                return;
            }
            self.input[used..].copy_from_slice(&data[..room]);
            transform(&mut self.buf, &self.input);
            data = &data[room..];
        }
        while data.len() >= BLOCK_LEN {
            self.input.copy_from_slice(&data[..BLOCK_LEN]);
            transform(&mut self.buf, &self.input);
            data = &data[BLOCK_LEN..];
        }
        self.input[..data.len()].copy_from_slice(data);
    }

    /// `MD5Final`: pads (`0x80`, zeros, then the 64-bit bit count in the
    /// last 8 bytes of a block, needing an extra block if fewer than 8
    /// bytes are left), returns the digest (`buf`, little-endian), and
    /// zeroes the whole context, all 88 bytes, as the ROM's closing
    /// `memset(ctx, 0, 0x58)` does.
    pub fn finalize(&mut self) -> [u8; DIGEST_LEN] {
        let count = ((self.bits[0] >> 3) & 0x3f) as usize;
        self.input[count] = 0x80;
        let left = BLOCK_LEN - 1 - count;
        if left < 8 {
            self.input[count + 1..].fill(0);
            transform(&mut self.buf, &self.input);
            self.input[..56].fill(0);
        } else {
            self.input[count + 1..56].fill(0);
        }
        self.input[56..60].copy_from_slice(&self.bits[0].to_le_bytes());
        self.input[60..64].copy_from_slice(&self.bits[1].to_le_bytes());
        transform(&mut self.buf, &self.input);
        let mut digest = [0u8; DIGEST_LEN];
        for (i, word) in self.buf.iter().enumerate() {
            digest[4 * i..4 * i + 4].copy_from_slice(&word.to_le_bytes());
        }
        *self = Self {
            buf: [0; 4],
            bits: [0; 2],
            input: [0; BLOCK_LEN],
        };
        digest
    }

    /// Reads a context from its 88-byte `md5_context_t` memory image.
    pub fn from_bytes(raw: &[u8; CONTEXT_LEN]) -> Self {
        let word = |at: usize| u32::from_le_bytes([raw[at], raw[at + 1], raw[at + 2], raw[at + 3]]);
        let mut input = [0u8; BLOCK_LEN];
        input.copy_from_slice(&raw[IN_OFFSET..]);
        Self {
            buf: core::array::from_fn(|i| word(4 * i)),
            bits: [word(BITS_OFFSET), word(BITS_OFFSET + 4)],
            input,
        }
    }

    /// The context's 88-byte `md5_context_t` memory image.
    pub fn to_bytes(&self) -> [u8; CONTEXT_LEN] {
        let mut raw = [0u8; CONTEXT_LEN];
        for (i, word) in self.buf.iter().chain(self.bits.iter()).enumerate() {
            raw[4 * i..4 * i + 4].copy_from_slice(&word.to_le_bytes());
        }
        raw[IN_OFFSET..].copy_from_slice(&self.input);
        raw
    }
}

/// One-shot MD5 of `msg`.
pub fn md5(msg: &[u8]) -> [u8; DIGEST_LEN] {
    let mut ctx = Md5Context::new();
    ctx.update(msg);
    ctx.finalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(d: [u8; 16]) -> String {
        d.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// RFC 1321 appendix A.5's test suite.
    const RFC_1321_VECTORS: &[(&[u8], &str)] = &[
        (b"", "d41d8cd98f00b204e9800998ecf8427e"),
        (b"a", "0cc175b9c0f1b6a831c399e269772661"),
        (b"abc", "900150983cd24fb0d6963f7d28e17f72"),
        (b"message digest", "f96b697d7cb7938d525a2f31aaf161d0"),
        (
            b"abcdefghijklmnopqrstuvwxyz",
            "c3fcd3d76192e4007dfb496cca67e13b",
        ),
        (
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789",
            "d174ab98d277d9f5a5611c2c9f419d9f",
        ),
        (
            b"12345678901234567890123456789012345678901234567890123456789012345678901234567890",
            "57edf4a22be3c955ac49da2e2107b67a",
        ),
    ];

    #[test]
    fn one_shot_md5_matches_the_rfc_1321_vectors() {
        for (msg, want) in RFC_1321_VECTORS {
            assert_eq!(hex(md5(msg)), *want, "{:?}", String::from_utf8_lossy(msg));
        }
    }

    #[test]
    fn streaming_context_matches_one_shot_for_any_split() {
        let msg: Vec<u8> = (0..200u32).map(|i| (i * 7 + 3) as u8).collect();
        let want = md5(&msg);
        for splits in [
            &[1usize, 63, 65, 7][..],
            &[0, 64, 0, 64, 72],
            &[55, 1, 8],
            &[56],
            &[63, 1],
            &[200],
        ] {
            let mut ctx = Md5Context::new();
            let mut at = 0;
            for n in splits {
                ctx.update(&msg[at..at + n]);
                at += n;
            }
            assert_eq!(ctx.finalize(), md5(&msg[..at]), "splits {splits:?}");
            if at == msg.len() {
                assert_eq!(md5(&msg[..at]), want);
            }
        }
    }

    #[test]
    fn init_writes_the_rfc_1321_iv_and_zero_bit_count_but_leaves_the_buffer() {
        let mut ctx = Md5Context::from_bytes(&[0xAB; CONTEXT_LEN]);
        ctx.init();
        let bytes = ctx.to_bytes();
        assert_eq!(
            bytes[..16],
            [
                0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54,
                0x32, 0x10
            ]
        );
        assert_eq!(bytes[16..24], [0; 8], "bits[2] zeroed");
        assert!(bytes[24..].iter().all(|b| *b == 0xAB), "in[] untouched");
    }

    #[test]
    fn update_keeps_a_64_bit_bit_count_low_word_first_and_buffers_the_tail() {
        let mut ctx = Md5Context::new();
        ctx.update(&[0x11; 70]);
        let bytes = ctx.to_bytes();
        assert_eq!(bytes[16..20], (70u32 * 8).to_le_bytes(), "bits[0]");
        assert_eq!(bytes[20..24], [0; 4], "bits[1]");
        // The 6-byte tail is copied over the start of in[]; the rest of in[]
        // still holds the last full block, as Plumb's memcpy leaves it.
        assert_eq!(bytes[24..30], [0x11; 6]);
        // Carry into bits[1]: start bits[0] just below 2^32.
        let mut raw = [0u8; CONTEXT_LEN];
        raw[16..20].copy_from_slice(&0xFFFF_FFF8u32.to_le_bytes());
        let mut ctx = Md5Context::from_bytes(&raw);
        ctx.update(&[0; 2]);
        let bytes = ctx.to_bytes();
        assert_eq!(bytes[16..20], 8u32.to_le_bytes());
        assert_eq!(bytes[20..24], 1u32.to_le_bytes());
    }

    #[test]
    fn finalize_zeroes_the_whole_context() {
        let mut ctx = Md5Context::new();
        ctx.update(b"abc");
        assert_eq!(hex(ctx.finalize()), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(ctx.to_bytes(), [0; CONTEXT_LEN]);
    }

    #[test]
    fn context_bytes_round_trip_in_the_md5_context_t_layout() {
        let raw: [u8; CONTEXT_LEN] = core::array::from_fn(|i| i as u8);
        assert_eq!(Md5Context::from_bytes(&raw).to_bytes(), raw);
    }
}
