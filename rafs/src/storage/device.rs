// Copyright 2020 Ant Financial. All rights reserved.
// Use of this source code is governed by a Apache 2.0 license that can be
// found in the LICENSE file.

use std::cmp;
use std::io;
use std::io::Error;

use fuse_rs::api::filesystem::{ZeroCopyReader, ZeroCopyWriter};
use fuse_rs::transport::FileReadWriteVolatile;
use vm_memory::VolatileSlice;

use crate::metadata::{RafsChunkInfo, RafsDigest};
use crate::storage::backend::*;
use crate::storage::factory;

static ZEROS: &[u8] = &[0u8; 4096]; // why 4096? volatile slice default size, unfortunately

// A rafs storage device
pub struct RafsDevice {
    c: factory::Config,
    b: Box<dyn BlobBackend + Send + Sync>,
}

impl RafsDevice {
    pub fn new(c: factory::Config) -> Self {
        let backend = factory::new_backend(&c.backend).unwrap();
        RafsDevice { c, b: backend }
    }

    pub fn init(&mut self) -> io::Result<()> {
        Ok(())
    }

    pub fn close(&mut self) {
        self.b.close();
    }

    /// Read a range of data from blob into the provided writer
    pub fn read_to(&self, w: &mut dyn ZeroCopyWriter, desc: RafsBioDesc) -> io::Result<usize> {
        let mut count: usize = 0;
        for bio in desc.bi_vec.iter() {
            let mut f = RafsBioDevice::new(bio, &self)?;
            count += w.write_from(&mut f, bio.size, bio.offset as u64)?;
        }
        Ok(count)
    }

    /// Write a range of data to blob from the provided reader
    pub fn write_from(&self, r: &mut dyn ZeroCopyReader, desc: RafsBioDesc) -> io::Result<usize> {
        let mut count: usize = 0;
        for bio in desc.bi_vec.iter() {
            let mut f = RafsBioDevice::new(bio, &self)?;
            let offset = bio.chunkinfo.blob_offset() + bio.offset as u64;
            count += r.read_to(&mut f, bio.size, offset)?;
        }
        Ok(count)
    }
}

pub struct RafsBuffer {
    buf: Vec<u8>,
    compressed: bool,
}

impl RafsBuffer {
    pub fn new_compressed(buf: Vec<u8>) -> RafsBuffer {
        RafsBuffer {
            buf,
            compressed: true,
        }
    }

    pub fn new_decompressed(buf: Vec<u8>) -> RafsBuffer {
        RafsBuffer {
            buf,
            compressed: false,
        }
    }

    pub fn decompressed(self, f: &dyn Fn(&[u8]) -> io::Result<Vec<u8>>) -> io::Result<Vec<u8>> {
        if self.compressed {
            f(self.buf.as_slice())
        } else {
            Ok(self.buf)
        }
    }
}

struct RafsBioDevice<'a> {
    bio: &'a RafsBio<'a>,
    dev: &'a RafsDevice,
    buf: Vec<u8>,
}

impl<'a> RafsBioDevice<'a> {
    fn new(bio: &'a RafsBio<'a>, b: &'a RafsDevice) -> io::Result<Self> {
        // FIXME: make sure bio is valid
        Ok(RafsBioDevice {
            bio,
            dev: b,
            buf: Vec::new(),
        })
    }

    fn blob_offset(&self) -> u64 {
        self.bio.chunkinfo.blob_offset() + self.bio.offset as u64
    }
}

impl FileReadWriteVolatile for RafsBioDevice<'_> {
    fn read_volatile(&mut self, slice: VolatileSlice) -> Result<usize, Error> {
        // Skip because we don't really use it
        Ok(slice.len())
    }

    fn write_volatile(&mut self, slice: VolatileSlice) -> Result<usize, Error> {
        // Skip because we don't really use it
        Ok(slice.len())
    }

    fn read_at_volatile(&mut self, slice: VolatileSlice, offset: u64) -> Result<usize, Error> {
        if self.buf.is_empty() {
            let mut buf = Vec::new();
            let len = self.dev.b.read(
                &self.bio.blob_id.as_str()?,
                &mut buf,
                self.bio.chunkinfo.blob_offset(),
                self.bio.chunkinfo.compress_size() as usize,
            )?;
            debug_assert_eq!(len, buf.len());
            self.buf = utils::decompress(&buf, self.bio.blksize)?;
        }

        let count = cmp::min(
            cmp::min(
                self.bio.offset as usize + self.bio.size - offset as usize,
                slice.len(),
            ),
            self.buf.len() - offset as usize,
        );
        slice.copy_from(&self.buf[offset as usize..offset as usize + count]);
        Ok(count)
    }

    // The default read_vectored_at_volatile only read to the first slice, so we have to overload it.
    fn read_vectored_at_volatile(
        &mut self,
        bufs: &[VolatileSlice],
        offset: u64,
    ) -> Result<usize, Error> {
        let mut f_offset: u64 = offset;
        let mut count: usize = 0;
        if self.bio.chunkinfo.compress_size() == 0 {
            return self.fill_hole(bufs);
        }
        for buf in bufs.iter() {
            let res = self.read_at_volatile(*buf, f_offset)?;
            count += res;
            f_offset += res as u64;
            if res == 0
                || count >= self.bio.size
                || f_offset >= self.bio.offset as u64 + self.bio.size as u64
            {
                break;
            }
        }
        Ok(count)
    }

    fn write_at_volatile(&mut self, slice: VolatileSlice, offset: u64) -> Result<usize, Error> {
        let mut buf = vec![0u8; slice.len()];
        slice.copy_to(&mut buf);
        let compressed = utils::compress(&buf)?;
        self.dev
            .b
            .write(&self.bio.blob_id.as_str()?, &compressed, offset)?;
        // Need to return slice length because that's what upper layer asks to write
        Ok(slice.len())
    }
}

impl RafsBioDevice<'_> {
    fn fill_hole(&self, bufs: &[VolatileSlice]) -> Result<usize, Error> {
        let mut count: usize = 0;
        let mut remain: usize = self.bio.size;
        for &buf in bufs.iter() {
            let mut total = cmp::min(remain, buf.len());
            while total > 0 {
                let cnt = cmp::min(total, ZEROS.len());
                buf.copy_from(&ZEROS[ZEROS.len() - cnt..]);
                count += cnt;
                remain -= cnt;
                total -= cnt;
            }
        }
        Ok(count)
    }
}

// Rafs device blob IO descriptor
#[derive(Clone, Default)]
pub struct RafsBioDesc<'a> {
    // Blob IO flags
    pub bi_flags: u32,
    // Totol IO size to be performed
    pub bi_size: usize,
    // Array of blob IO info. Corresponding data should be read from/write to IO stream sequentially
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
#[derive(Copy, Clone)]
pub struct RafsBio<'a> {
    /// Reference to the chunk.
    pub chunkinfo: &'a dyn RafsChunkInfo,
    /// blob id of chunk
    pub blob_id: &'a dyn RafsDigest,
    /// offset within the block
    pub offset: u32,
    /// size of data to transfer
    pub size: usize,
    /// block size to read in one shot
    pub blksize: u32,
}

impl<'a> RafsBio<'a> {
    pub fn new(
        chunkinfo: &'a dyn RafsChunkInfo,
        blob_id: &'a dyn RafsDigest,
        offset: u32,
        size: usize,
        blksize: u32,
    ) -> Self {
        RafsBio {
            chunkinfo,
            blob_id,
            offset,
            size,
            blksize,
        }
    }
}
