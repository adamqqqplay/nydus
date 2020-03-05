// Copyright 2020 Ant Financial. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io;
use std::io::{Error, Read, Write};

use vm_memory::VolatileSlice;

use fuse::filesystem::{ZeroCopyReader, ZeroCopyWriter};
use vhost_rs::descriptor_utils::FileReadWriteVolatile;

use crate::fs::RafsBlk;
use crate::storage::backend::*;

use utils;

// A rafs storage device config
#[derive(Default, Clone, Serialize, Deserialize)]
pub struct Config {
    // backend type
    pub backend_type: String,
    // Storage path, can be a directory or a URL to some remote storage
    pub endpoint: String,
    // OSS bucket name
    pub bucket_name: String,
    // optional auth info used to access the storage
    pub access_key_id: String,
    pub access_key_secret: String,
}

impl Config {
    pub fn new() -> Config {
        Config {
            ..Default::default()
        }
    }

    pub fn hashmap(&self) -> HashMap<&str, &str> {
        let mut hmap: HashMap<&str, &str> = HashMap::new();
        hmap.insert("endpoint", &self.endpoint);
        hmap.insert("access_key_id", &self.access_key_id);
        hmap.insert("access_key_secret", &self.access_key_secret);
        hmap.insert("bucket_name", &self.bucket_name);
        hmap
    }
}

// A rafs storage device
pub struct RafsDevice<B: BlobBackend> {
    c: Config,
    b: B,
}

impl<B: BlobBackend> RafsDevice<B> {
    pub fn new(c: Config, b: B) -> Self {
        match c.backend_type {
            _ => RafsDevice { c: c, b: b },
        }
    }
}

impl<B: BlobBackend> RafsDevice<B> {
    pub fn init(&mut self) -> io::Result<()> {
        self.b.init(self.c.hashmap())
    }

    pub fn close(&mut self) -> io::Result<()> {
        self.b.close();
        Ok(())
    }

    // Read a range of data from blob into the provided writer
    pub fn read_to<W: Write + ZeroCopyWriter>(
        &self,
        mut w: W,
        desc: RafsBioDesc,
    ) -> io::Result<usize> {
        let mut count: usize = 0;
        for bio in desc.bi_vec.iter() {
            let mut f = RafsBioDevice::new(bio, &self)?;
            let offset = bio.blkinfo.blob_offset + bio.offset as u64;
            debug!("reading bio desc {:?}", bio);
            count += w.write_from(&mut f, bio.size, offset)?;
        }
        Ok(count)
    }

    // Write a range of data to blob from the provided reader
    pub fn write_from<R: Read + ZeroCopyReader>(
        &self,
        mut r: R,
        desc: RafsBioDesc,
    ) -> io::Result<usize> {
        let mut count: usize = 0;
        for bio in desc.bi_vec.iter() {
            let mut f = RafsBioDevice::new(bio, &self)?;
            let offset = bio.blkinfo.blob_offset + bio.offset as u64;
            count += r.read_to(&mut f, bio.size, offset)?;
        }
        Ok(count)
    }
}

struct RafsBioDevice<'a, B: BlobBackend> {
    bio: &'a RafsBio<'a>,
    dev: &'a RafsDevice<B>,
}

impl<'a, B: BlobBackend> RafsBioDevice<'a, B> {
    fn new(bio: &'a RafsBio<'a>, b: &'a RafsDevice<B>) -> io::Result<Self> {
        // FIXME: make sure bio is valid
        Ok(RafsBioDevice { bio: bio, dev: b })
    }

    fn blob_offset(&self) -> u64 {
        let blkinfo = &self.bio.blkinfo;
        blkinfo.blob_offset + self.bio.offset as u64
    }
}

impl<B: BlobBackend> FileReadWriteVolatile for RafsBioDevice<'_, B> {
    fn read_volatile(&mut self, slice: VolatileSlice) -> Result<usize, Error> {
        // Skip because we don't really use it
        Ok(slice.len())
    }

    fn write_volatile(&mut self, slice: VolatileSlice) -> Result<usize, Error> {
        // Skip because we don't really use it
        Ok(slice.len())
    }

    fn read_at_volatile(&mut self, slice: VolatileSlice, offset: u64) -> Result<usize, Error> {
        let mut buf: Vec<u8> = Vec::new();
        let len = self.dev.b.read(
            &self.bio.blkinfo.blob_id,
            &mut buf,
            offset,
            self.bio.blkinfo.compr_size,
        )?;
        debug_assert_eq!(len, buf.len());
        let decompressed = utils::decompress_with_lz4(&buf)?;
        let mut count = self.bio.size;
        if slice.len() < count {
            count = slice.len()
        }
        slice.copy_from(&decompressed[self.bio.offset as usize..self.bio.offset as usize + count]);

        Ok(count)
    }

    fn write_at_volatile(&mut self, slice: VolatileSlice, offset: u64) -> Result<usize, Error> {
        let mut buf = vec![0u8; slice.len()];
        slice.copy_to(&mut buf);
        let compressed = utils::compress_with_lz4(&buf)?;
        self.dev
            .b
            .write(&self.bio.blkinfo.blob_id, &compressed, offset)?;
        // Need to return slice length because that's what upper layer asks to write
        Ok(slice.len())
    }
}

// Rafs device blob IO descriptor
#[derive(Default, Debug)]
pub struct RafsBioDesc<'a> {
    // Blob IO flags
    pub bi_flags: u32,
    // Totol IO size to be performed
    pub bi_size: usize,
    // Array of blob IO info. Corresponding data should
    // be read from (written to) IO stream sequencially
    pub bi_vec: Vec<RafsBio<'a>>,
}

impl RafsBioDesc<'_> {
    pub fn new() -> Self {
        RafsBioDesc {
            ..Default::default()
        }
    }
}

// Rafs blob IO info
#[derive(Copy, Clone, Debug)]
pub struct RafsBio<'a> {
    pub blkinfo: &'a RafsBlk,
    // offset within the block
    pub offset: u32,
    // size of data to transfer
    pub size: usize,
}

impl<'a> RafsBio<'a> {
    pub fn new(b: &'a RafsBlk, offset: u32, size: usize) -> Self {
        RafsBio {
            blkinfo: b,
            offset: offset,
            size: size,
        }
    }
}
