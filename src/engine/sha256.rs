//! Dependency-free SHA-256 for credential storage.
//!
//! This is the standard FIPS 180-4 construction, used only to compare supplied
//! passwords with a stored digest. It is not a general crypto library and makes
//! no claim about transport security or password strength.

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

const INITIAL_STATE: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

/// Incremental SHA-256 so journal authentication can hash large files without
/// materializing them in memory.
pub(super) struct Sha256 {
    state: [u32; 8],
    buffer: [u8; 64],
    buffered: usize,
    total: u64,
}

impl Sha256 {
    pub(super) fn new() -> Self {
        Self {
            state: INITIAL_STATE,
            buffer: [0; 64],
            buffered: 0,
            total: 0,
        }
    }

    pub(super) fn update(&mut self, mut input: &[u8]) {
        self.total = self.total.wrapping_add(input.len() as u64);
        if self.buffered > 0 {
            let take = (64 - self.buffered).min(input.len());
            self.buffer[self.buffered..self.buffered + take].copy_from_slice(&input[..take]);
            self.buffered += take;
            input = &input[take..];
            if self.buffered == 64 {
                let block = self.buffer;
                self.compress(&block);
                self.buffered = 0;
            }
        }
        while input.len() >= 64 {
            let (block, rest) = input.split_at(64);
            self.compress(block.try_into().expect("64-byte SHA-256 block"));
            input = rest;
        }
        if !input.is_empty() {
            self.buffer[..input.len()].copy_from_slice(input);
            self.buffered = input.len();
        }
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut words = [0u32; 64];
        for (word, bytes) in words.iter_mut().zip(block.as_chunks::<4>().0) {
            *word = u32::from_be_bytes(*bytes);
        }
        for index in 16..64 {
            let s0 = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let s1 = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] = words[index - 16]
                .wrapping_add(s0)
                .wrapping_add(words[index - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ (!e & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(choice)
                .wrapping_add(K[index])
                .wrapping_add(words[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        for (cell, value) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *cell = cell.wrapping_add(value);
        }
    }

    pub(super) fn finish(mut self) -> [u8; 32] {
        let bits = self.total.wrapping_mul(8);
        let mut tail = [0u8; 128];
        tail[..self.buffered].copy_from_slice(&self.buffer[..self.buffered]);
        tail[self.buffered] = 0x80;
        let end = if self.buffered < 56 { 64 } else { 128 };
        tail[end - 8..end].copy_from_slice(&bits.to_be_bytes());
        for block in tail[..end].as_chunks::<64>().0 {
            self.compress(block);
        }
        let mut output = [0u8; 32];
        for (bytes, word) in output.as_chunks_mut::<4>().0.iter_mut().zip(self.state) {
            *bytes = word.to_be_bytes();
        }
        output
    }
}

pub(super) fn digest(input: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(input);
    hasher.finish()
}

pub(super) fn hex32(bytes: &[u8; 32]) -> String {
    let mut text = String::with_capacity(64);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(text, "{byte:02x}");
    }
    text
}

pub(super) fn parse_hex32(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 {
        return None;
    }
    let mut output = [0u8; 32];
    for (chunk, byte) in text
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .zip(output.iter_mut())
    {
        let high = (chunk[0] as char).to_digit(16)?;
        let low = (chunk[1] as char).to_digit(16)?;
        *byte = ((high << 4) | low) as u8;
    }
    Some(output)
}

/// Lowercase hexadecimal digest for tests and future ACL hash interoperability.
#[cfg(test)]
pub(super) fn hex(input: &[u8]) -> String {
    let mut text = String::with_capacity(64);
    for byte in digest(input) {
        use std::fmt::Write as _;
        let _ = write!(text, "{byte:02x}");
    }
    text
}

/// Incremental RFC 2104 HMAC-SHA256. The journal uses one instance per hash
/// link so a multi-gigabyte rewrite never has to be buffered in memory.
pub(super) struct HmacSha256 {
    inner: Sha256,
    outer_pad: [u8; 64],
}

impl HmacSha256 {
    pub(super) fn new(key: &[u8]) -> Self {
        let mut normalized = [0u8; 64];
        if key.len() > normalized.len() {
            normalized[..32].copy_from_slice(&digest(key));
        } else {
            normalized[..key.len()].copy_from_slice(key);
        }
        let mut inner = Sha256::new();
        inner.update(&std::array::from_fn::<u8, 64, _>(|index| {
            normalized[index] ^ 0x36
        }));
        Self {
            inner,
            outer_pad: std::array::from_fn(|index| normalized[index] ^ 0x5c),
        }
    }

    pub(super) fn update(&mut self, message: &[u8]) {
        self.inner.update(message);
    }

    pub(super) fn finish(self) -> [u8; 32] {
        let inner = self.inner.finish();
        let mut outer = Sha256::new();
        outer.update(&self.outer_pad);
        outer.update(&inner);
        outer.finish()
    }
}

/// RFC 2104 HMAC-SHA256 for optional audit-line and journal authentication.
pub(super) fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut hmac = HmacSha256::new(key);
    hmac.update(message);
    hmac.finish()
}

pub(super) fn constant_time_eq(left: &[u8; 32], right: &[u8; 32]) -> bool {
    let mut difference = 0u8;
    for (a, b) in left.iter().zip(right) {
        difference |= a ^ b;
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors_and_length_boundaries_match_fips_180_4() {
        for (input, expected) in [
            (
                b"".as_slice(),
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            ),
            (
                b"abc",
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            ),
            (
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
                "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
            ),
            (
                b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb",
            ),
        ] {
            assert_eq!(hex(input), expected, "{input:?}");
        }
        let million = vec![b'a'; 1_000_000];
        assert_eq!(
            hex(&million),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    fn test_hex32(bytes: [u8; 32]) -> String {
        let mut text = String::with_capacity(64);
        for byte in bytes {
            use std::fmt::Write as _;
            let _ = write!(text, "{byte:02x}");
        }
        text
    }

    #[test]
    fn hmac_sha256_matches_rfc_4231_vectors() {
        assert_eq!(
            test_hex32(hmac_sha256(&[0x0b; 20], b"Hi There")),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        assert_eq!(
            test_hex32(hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        assert_eq!(
            test_hex32(hmac_sha256(
                &[0xaa; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            )),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn streaming_sha256_and_hmac_match_one_shot_construction_at_every_chunk_size() {
        let message: Vec<u8> = (0..1000).map(|index| (index % 251) as u8).collect();
        let expected_digest = digest(&message);
        let expected_hmac = hmac_sha256(b"a 16-byte key!!!!", &message);
        for chunk in [1, 3, 7, 63, 64, 65, 127, 128, 129, 511] {
            let mut hasher = Sha256::new();
            let mut hmac = HmacSha256::new(b"a 16-byte key!!!!");
            for piece in message.chunks(chunk) {
                hasher.update(piece);
                hmac.update(piece);
            }
            assert_eq!(hasher.finish(), expected_digest, "chunk {chunk}");
            assert_eq!(hmac.finish(), expected_hmac, "chunk {chunk}");
        }
    }

    #[test]
    fn hex_round_trip_rejects_malformed_or_wrong_length_digests() {
        let digest = digest(b"journal authentication");
        let encoded = hex32(&digest);
        assert_eq!(encoded.len(), 64);
        assert_eq!(parse_hex32(&encoded), Some(digest));
        assert_eq!(parse_hex32(&encoded[..63]), None);
        assert_eq!(parse_hex32(&format!("{}z", &encoded[..63])), None);
        assert_eq!(parse_hex32(&format!("{}G", &encoded[..63])), None);
    }

    #[test]
    fn constant_time_comparison_only_matches_equal_digests() {
        let left = digest(b"secret");
        assert!(constant_time_eq(&left, &digest(b"secret")));
        assert!(!constant_time_eq(&left, &digest(b"secreu")));
        assert!(!constant_time_eq(&left, &[0; 32]));
    }
}
