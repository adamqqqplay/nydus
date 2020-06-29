// Copyright 2020 Ant Financial. All rights reserved.
//
// SPDX-License-Identifier: Apache-2.0

use std::borrow::Cow;
use std::convert::From;
use std::fmt;
use std::io::{Error, Result};
use std::str::FromStr;

mod lz4_standard;
use self::lz4_standard::*;

use nydus_error::einval;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Algorithm {
    None = 0,
    LZ4Block = 1,
}

impl fmt::Display for Algorithm {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl FromStr for Algorithm {
    type Err = Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "none" => Ok(Self::None),
            "lz4_block" => Ok(Self::LZ4Block),
            _ => Err(einval!("compression algorithm should be none or lz4_block")),
        }
    }
}

impl From<&u8> for Algorithm {
    fn from(src: &u8) -> Self {
        match *src {
            1 => Self::LZ4Block,
            _ => Self::None,
        }
    }
}

impl Algorithm {
    pub fn is_none(self) -> bool {
        self == Self::None
    }
}

// Algorithm::LZ4Block:
// 1. Default ratio
// 2. No prepend size

// For compatibility reason, we use liblz4 version to compress/decompress directly
// with data blocks so that we don't really care about lz4 header magic numbers like
// as being done with all these rust lz4 implementations
pub fn compress(src: &[u8], algorithm: Algorithm) -> Result<Cow<[u8]>> {
    match algorithm {
        Algorithm::None => Ok(Cow::Borrowed(src)),
        Algorithm::LZ4Block => lz4_compress(src).map(Cow::Owned),
    }
}

pub fn decompress(src: &[u8], dst: &mut [u8]) -> Result<usize> {
    lz4_decompress(src, dst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compress_algorithm_none() {
        let buf = [
            0x1u8, 0x2u8, 0x3u8, 0x4u8, 0x1u8, 0x2u8, 0x3u8, 0x4u8, 0x1u8, 0x2u8, 0x3u8, 0x4u8,
            0x1u8, 0x2u8, 0x3u8, 0x4u8,
        ];
        let compressed = compress(&buf, Algorithm::None).unwrap();

        assert_eq!(buf.to_vec(), compressed.to_vec());
    }

    #[test]
    fn test_lz4_compress_decompress_0_byte() {
        let buf = Vec::new();
        let compressed = compress(&buf, Algorithm::LZ4Block).unwrap();
        let mut decompressed = vec![0; buf.len()];
        let sz = decompress(&compressed, decompressed.as_mut_slice()).unwrap();

        assert_eq!(sz, 0);
        assert_eq!(buf, decompressed);
    }

    #[test]
    fn test_lz4_compress_decompress_1_byte() {
        let buf = vec![0x1u8];
        let compressed = compress(&buf, Algorithm::LZ4Block).unwrap();
        let mut decompressed = vec![0; buf.len()];
        let sz = decompress(&compressed, decompressed.as_mut_slice()).unwrap();

        assert_eq!(sz, 1);
        assert_eq!(buf, decompressed);
    }

    #[test]
    fn test_lz4_compress_decompress_2_bytes() {
        let buf = vec![0x2u8, 0x3u8];
        let compressed = compress(&buf, Algorithm::LZ4Block).unwrap();
        let mut decompressed = vec![0; buf.len()];
        let sz = decompress(&compressed, decompressed.as_mut_slice()).unwrap();

        assert_eq!(sz, 2);
        assert_eq!(buf, decompressed);
    }

    #[test]
    fn test_lz4_compress_decompress_16_bytes() {
        let buf = [
            0x1u8, 0x2u8, 0x3u8, 0x4u8, 0x1u8, 0x2u8, 0x3u8, 0x4u8, 0x1u8, 0x2u8, 0x3u8, 0x4u8,
            0x1u8, 0x2u8, 0x3u8, 0x4u8,
        ];
        let compressed = compress(&buf, Algorithm::LZ4Block).unwrap();
        let mut decompressed = vec![0; buf.len()];
        let sz = decompress(&compressed, decompressed.as_mut_slice()).unwrap();

        assert_eq!(sz, 16);
        assert_eq!(&buf, decompressed.as_slice());
    }

    #[test]
    fn test_lz4_compress_decompress_4095_bytes() {
        let buf = vec![0x2u8; 4095];
        let compressed = compress(&buf, Algorithm::LZ4Block).unwrap();
        let mut decompressed = vec![0; buf.len()];
        let sz = decompress(&compressed, decompressed.as_mut_slice()).unwrap();

        assert_eq!(sz, 4095);
        assert_eq!(buf, decompressed);
    }

    #[test]
    fn test_lz4_compress_decompress_4096_bytes() {
        let buf = vec![0x2u8; 4096];
        let compressed = compress(&buf, Algorithm::LZ4Block).unwrap();
        let mut decompressed = vec![0; buf.len()];
        let sz = decompress(&compressed, decompressed.as_mut_slice()).unwrap();

        assert_eq!(sz, 4096);
        assert_eq!(buf, decompressed);
    }

    #[test]
    fn test_lz4_compress_decompress_4097_bytes() {
        let buf = vec![0x2u8; 4097];
        let compressed = compress(&buf, Algorithm::LZ4Block).unwrap();
        let mut decompressed = vec![0; buf.len()];
        let sz = decompress(&compressed, decompressed.as_mut_slice()).unwrap();

        assert_eq!(sz, 4097);
        assert_eq!(buf, decompressed);
    }
}
