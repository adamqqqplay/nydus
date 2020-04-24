// Copyright 2020 Ant Financial. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use crate::fs::RafsBlk;
use crate::storage::backend::BlobBackend;
use crate::storage::cache::RafsCache;
use std::io::{Error, Result};

pub struct DummyCache {
    pub backend: Box<dyn BlobBackend + Sync + Send>,
}

impl DummyCache {
    pub fn new(backend: Box<dyn BlobBackend + Sync + Send>) -> DummyCache {
        DummyCache { backend }
    }
}

impl RafsCache for DummyCache {
    fn has(&self, _blk: &RafsBlk) -> bool {
        true
    }

    fn evict(&self, _blk: &RafsBlk) -> Result<()> {
        Ok(())
    }

    fn flush(&self) -> Result<()> {
        Ok(())
    }

    fn read(&self, blk: &RafsBlk) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; blk.compr_size];
        let len = self
            .backend
            .read(&blk.blob_id, &mut buf, blk.blob_offset, blk.compr_size)?;
        if len != blk.compr_size {
            return Err(Error::from_raw_os_error(libc::EIO));
        }
        Ok(buf)
    }

    fn write(&self, blk: &RafsBlk, buf: &[u8]) -> Result<usize> {
        self.backend.write(&blk.blob_id, buf, blk.blob_offset)
    }

    fn compressed(&self) -> bool {
        true
    }

    fn release(&mut self) {
        self.backend.close();
    }
}

pub fn new(backend: Box<dyn BlobBackend + Sync + Send>) -> Result<DummyCache> {
    Ok(DummyCache { backend })
}
