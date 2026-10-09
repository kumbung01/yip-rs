use crate::config::*;

fn hash(mut x: u32) -> u32 {
    x = x.wrapping_mul(0x9E37_79B1);
    x = (x ^ (x >> 16)).wrapping_mul(0x85EB_CA6B);
    x = (x ^ (x >> 13)).wrapping_mul(0xC2B2_AE35);
    x ^ (x >> 16)
}

pub fn get_channel(seq: u32) -> u8 {
    let index = hash(seq) % CHANNELS;
    (1 + index * 2) as u8
}
