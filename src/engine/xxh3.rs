// Copyright (C) 2012-2023 Yann Collet.
// SPDX-License-Identifier: BSD-2-Clause
//
// This safe scalar implementation follows the XXH3-64 algorithm specification
// and the reference implementation in https://github.com/Cyan4973/xxHash.
// See container/notices/xxHash-BSD-2-Clause.txt for the required license.

const SECRET: [u8; 192] = [
    0xb8, 0xfe, 0x6c, 0x39, 0x23, 0xa4, 0x4b, 0xbe, 0x7c, 0x01, 0x81, 0x2c, 0xf7, 0x21, 0xad, 0x1c,
    0xde, 0xd4, 0x6d, 0xe9, 0x83, 0x90, 0x97, 0xdb, 0x72, 0x40, 0xa4, 0xa4, 0xb7, 0xb3, 0x67, 0x1f,
    0xcb, 0x79, 0xe6, 0x4e, 0xcc, 0xc0, 0xe5, 0x78, 0x82, 0x5a, 0xd0, 0x7d, 0xcc, 0xff, 0x72, 0x21,
    0xb8, 0x08, 0x46, 0x74, 0xf7, 0x43, 0x24, 0x8e, 0xe0, 0x35, 0x90, 0xe6, 0x81, 0x3a, 0x26, 0x4c,
    0x3c, 0x28, 0x52, 0xbb, 0x91, 0xc3, 0x00, 0xcb, 0x88, 0xd0, 0x65, 0x8b, 0x1b, 0x53, 0x2e, 0xa3,
    0x71, 0x64, 0x48, 0x97, 0xa2, 0x0d, 0xf9, 0x4e, 0x38, 0x19, 0xef, 0x46, 0xa9, 0xde, 0xac, 0xd8,
    0xa8, 0xfa, 0x76, 0x3f, 0xe3, 0x9c, 0x34, 0x3f, 0xf9, 0xdc, 0xbb, 0xc7, 0xc7, 0x0b, 0x4f, 0x1d,
    0x8a, 0x51, 0xe0, 0x4b, 0xcd, 0xb4, 0x59, 0x31, 0xc8, 0x9f, 0x7e, 0xc9, 0xd9, 0x78, 0x73, 0x64,
    0xea, 0xc5, 0xac, 0x83, 0x34, 0xd3, 0xeb, 0xc3, 0xc5, 0x81, 0xa0, 0xff, 0xfa, 0x13, 0x63, 0xeb,
    0x17, 0x0d, 0xdd, 0x51, 0xb7, 0xf0, 0xda, 0x49, 0xd3, 0x16, 0x55, 0x26, 0x29, 0xd4, 0x68, 0x9e,
    0x2b, 0x16, 0xbe, 0x58, 0x7d, 0x47, 0xa1, 0xfc, 0x8f, 0xf8, 0xb8, 0xd1, 0x7a, 0xd0, 0x31, 0xce,
    0x45, 0xcb, 0x3a, 0x8f, 0x95, 0x16, 0x04, 0x28, 0xaf, 0xd7, 0xfb, 0xca, 0xbb, 0x4b, 0x40, 0x7e,
];

const PRIME32_1: u64 = 0x9e37_79b1;
const PRIME32_2: u64 = 0x85eb_ca77;
const PRIME32_3: u64 = 0xc2b2_ae3d;
const PRIME64_1: u64 = 0x9e37_79b1_85eb_ca87;
const PRIME64_2: u64 = 0xc2b2_ae3d_27d4_eb4f;
const PRIME64_3: u64 = 0x1656_67b1_9e37_79f9;
const PRIME64_4: u64 = 0x85eb_ca77_c2b2_ae63;
const PRIME64_5: u64 = 0x27d4_eb2f_1656_67c5;
const PRIME_MX1: u64 = 0x1656_6791_9e37_79f9;
const PRIME_MX2: u64 = 0x9fb2_1c65_1e98_df25;

#[inline]
fn read32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("four-byte slice"),
    )
}

#[inline]
fn read64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("eight-byte slice"),
    )
}

#[inline]
fn avalanche(value: u64) -> u64 {
    let value = (value ^ (value >> 37)).wrapping_mul(PRIME_MX1);
    value ^ (value >> 32)
}

#[inline]
fn avalanche64(mut value: u64) -> u64 {
    value ^= value >> 33;
    value = value.wrapping_mul(PRIME64_2);
    value ^= value >> 29;
    value = value.wrapping_mul(PRIME64_3);
    value ^ (value >> 32)
}

#[inline]
fn rrmxmx(mut value: u64, length: usize) -> u64 {
    value ^= value.rotate_left(49) ^ value.rotate_left(24);
    value = value.wrapping_mul(PRIME_MX2);
    value ^= (value >> 35).wrapping_add(length as u64);
    value = value.wrapping_mul(PRIME_MX2);
    value ^ (value >> 28)
}

#[inline]
fn fold_multiply(lhs: u64, rhs: u64) -> u64 {
    let product = (lhs as u128) * (rhs as u128);
    product as u64 ^ (product >> 64) as u64
}

#[inline]
fn mix16(input: &[u8], input_offset: usize, secret_offset: usize) -> u64 {
    fold_multiply(
        read64(input, input_offset) ^ read64(&SECRET, secret_offset),
        read64(input, input_offset + 8) ^ read64(&SECRET, secret_offset + 8),
    )
}

fn hash_short(input: &[u8]) -> u64 {
    let len = input.len();
    match len {
        0 => avalanche64(read64(&SECRET, 56) ^ read64(&SECRET, 64)),
        1..=3 => {
            let combined = u32::from(input[0])
                | (u32::from(input[len >> 1]) << 8)
                | (u32::from(input[len - 1]) << 16)
                | ((len as u32) << 24);
            let bitflip = (read32(&SECRET, 0) ^ read32(&SECRET, 4)) as u64;
            avalanche64(u64::from(combined) ^ bitflip)
        }
        4..=8 => {
            let input_first = u64::from(read32(input, 0));
            let input_last = u64::from(read32(input, len - 4));
            // Redis uses the default XXH3 seed (zero).
            let bitflip = read64(&SECRET, 8) ^ read64(&SECRET, 16);
            rrmxmx((input_last | (input_first << 32)) ^ bitflip, len)
        }
        9..=16 => {
            let low = read64(input, 0) ^ (read64(&SECRET, 24) ^ read64(&SECRET, 32));
            let high = read64(input, len - 8) ^ (read64(&SECRET, 40) ^ read64(&SECRET, 48));
            avalanche(
                (len as u64)
                    .wrapping_add(low.swap_bytes())
                    .wrapping_add(high)
                    .wrapping_add(fold_multiply(low, high)),
            )
        }
        _ => unreachable!(),
    }
}

fn hash_medium(input: &[u8]) -> u64 {
    let len = input.len();
    let mut acc = (len as u64).wrapping_mul(PRIME64_1);
    if len <= 128 {
        let rounds = (len - 1) / 32 + 1;
        for i in (0..rounds).rev() {
            acc = acc.wrapping_add(mix16(input, i * 16, i * 32));
            acc = acc.wrapping_add(mix16(input, len - i * 16 - 16, i * 32 + 16));
        }
        return avalanche(acc);
    }

    let rounds = len / 16;
    for i in 0..8 {
        acc = acc.wrapping_add(mix16(input, i * 16, i * 16));
    }
    let mut end = mix16(input, len - 16, 119);
    acc = avalanche(acc);
    for i in 8..rounds {
        end = end.wrapping_add(mix16(input, i * 16, (i - 8) * 16 + 3));
    }
    avalanche(acc.wrapping_add(end))
}

fn accumulate(acc: &mut [u64; 8], input: &[u8], input_offset: usize, secret_offset: usize) {
    for lane in 0..8 {
        let data = read64(input, input_offset + lane * 8);
        let keyed = data ^ read64(&SECRET, secret_offset + lane * 8);
        let product = u64::from(keyed as u32).wrapping_mul(u64::from((keyed >> 32) as u32));
        acc[lane ^ 1] = acc[lane ^ 1].wrapping_add(data);
        acc[lane] = acc[lane].wrapping_add(product);
    }
}

fn accumulate_stripes(acc: &mut [u64; 8], input: &[u8], start: usize, stripes: usize) {
    for stripe in 0..stripes {
        let input_offset = start + stripe * 64;
        let secret_offset = stripe * 8;
        for lane in 0..8 {
            let data = read64(input, input_offset + lane * 8);
            let keyed = data ^ read64(&SECRET, secret_offset + lane * 8);
            acc[lane ^ 1] = acc[lane ^ 1].wrapping_add(data);
            let product = u64::from(keyed as u32).wrapping_mul(u64::from((keyed >> 32) as u32));
            acc[lane] = acc[lane].wrapping_add(product);
        }
    }
}

fn hash_long(input: &[u8]) -> u64 {
    let mut acc = [
        PRIME32_3, PRIME64_1, PRIME64_2, PRIME64_3, PRIME64_4, PRIME32_2, PRIME64_5, PRIME32_1,
    ];
    let block_len = 1024;
    let blocks = (input.len() - 1) / block_len;
    for block in 0..blocks {
        accumulate_stripes(&mut acc, input, block * block_len, 16);
        for (lane, value) in acc.iter_mut().enumerate() {
            *value ^= *value >> 47;
            *value ^= read64(&SECRET, 128 + lane * 8);
            *value = value.wrapping_mul(PRIME32_1);
        }
    }
    let start = blocks * block_len;
    let stripes = ((input.len() - 1) - start) / 64;
    accumulate_stripes(&mut acc, input, start, stripes);
    accumulate(&mut acc, input, input.len() - 64, SECRET.len() - 64 - 7);
    let mut result = (input.len() as u64).wrapping_mul(PRIME64_1);
    for pair in 0..4 {
        result = result.wrapping_add(fold_multiply(
            acc[pair * 2] ^ read64(&SECRET, 11 + pair * 16),
            acc[pair * 2 + 1] ^ read64(&SECRET, 19 + pair * 16),
        ));
    }
    avalanche(result)
}

pub(super) fn xxh3_64(input: &[u8]) -> u64 {
    match input.len() {
        0..=16 => hash_short(input),
        17..=240 => hash_medium(input),
        _ => hash_long(input),
    }
}

#[cfg(test)]
mod tests {
    use super::xxh3_64;

    #[test]
    fn official_xxh3_64_known_vectors_cover_all_length_paths() {
        assert_eq!(xxh3_64(b""), 0x2d06800538d394c2);
        assert_eq!(xxh3_64(b"a"), 0xc44bdff4074eecdb);
        assert_eq!(xxh3_64(b"abc"), 0xeb5d658bb22f286b);
        assert_eq!(xxh3_64(b"foo"), 0xab6e5f64077e7d8a);
        assert_eq!(xxh3_64(&[0; 17]), 0xc2915ca0df7ad4c1);
        assert_eq!(xxh3_64(&[0; 129]), 0x37f7943eb2f51359);
        assert_eq!(xxh3_64(&[0; 241]), 0x5c5b5d5d40c59ce3);
        assert_eq!(xxh3_64(&[0; 1025]), 0x64ace6654784c523);
        let redis_vector = b"v8lf0c11xh8ymlqztfd3eeq16kfn4sspw7fqmnuuq3k3t75em5wdizgcdw7uc26nnf961u2jkfzkjytls2kwlj7626sd";
        assert_eq!(xxh3_64(redis_vector), 0x00006c38adf31777);
    }
}
