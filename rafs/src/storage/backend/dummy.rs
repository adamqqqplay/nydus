// Copyright 2020 Ant Financial. All rights reserved.
//
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::io::Result;
use std::sync::{Arc, Mutex, RwLock};
use vm_memory::VolatileSlice;

use crate::storage::backend::BlobBackend;

#[derive(Default, Clone)]
struct DummyTarget {
    path: String,
}

impl DummyTarget {
    fn new(blobid: &str) -> DummyTarget {
        DummyTarget {
            path: blobid.to_owned(),
        }
    }
}

pub struct Dummy {
    targets: RwLock<HashMap<String, Arc<Mutex<DummyTarget>>>>,
}

pub fn new() -> Dummy {
    Dummy {
        targets: RwLock::new(HashMap::new()),
    }
}

impl BlobBackend for Dummy {
    // Read a range of data from blob into the provided destination
    fn read(&self, _blobid: &str, buf: &mut [u8], _offset: u64) -> Result<usize> {
        Ok(buf.len())
    }

    fn readv(&self, _blobid: &str, _bufs: &[VolatileSlice], _offset: u64) -> Result<usize> {
        Ok(0)
    }

    // Write a range of data to blob from the provided source
    fn write(&self, _blobid: &str, buf: &[u8], _offset: u64) -> Result<usize> {
        Ok(buf.len())
    }

    fn close(&mut self) {
        self.targets.write().unwrap().clear()
    }
}
