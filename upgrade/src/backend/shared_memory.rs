// Copyright 2020 Ant Group. All rights reserved.
// Copyright (C) 2020 Alibaba Cloud. All rights reserved.
//
// SPDX-License-Identifier: (Apache-2.0 AND BSD-3-Clause)

use std::fs::File;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::{Read, Write};
use std::os::unix::io::{FromRawFd, RawFd};

use nix::errno::Errno;
use nix::fcntl::OFlag;
use nix::sys::mman::{shm_open, shm_unlink};
use nix::sys::stat::Mode;

use super::{Backend, BackendError, Result};

pub struct SharedMemoryBackend {
    name: String,
    file: File,
}

use BackendError::*;

// SharedMemoryBackend is responsible for writing/reading binary data to/from shared memory file.
impl SharedMemoryBackend {
    pub fn new(name: &str) -> Result<Self> {
        let fd = shm_open(
            name,
            OFlag::O_CREAT | OFlag::O_RDWR,
            Mode::S_IRUSR | Mode::S_IWUSR,
        )
        .map_err(SharedMem)?;

        let file = unsafe { File::from_raw_fd(fd) };

        Ok(Self {
            name: String::from(name),
            file,
        })
    }

    fn reset(&mut self) -> Result<()> {
        self.file.seek(SeekFrom::Start(0)).map_err(File)?;
        Ok(())
    }
}

impl Backend for SharedMemoryBackend {
    fn save(&mut self, _fds: &[RawFd], opaque: &[u8]) -> Result<usize> {
        self.reset()?;
        self.file.write_all(opaque).map_err(File)?;
        Ok(opaque.len())
    }

    fn restore(
        &mut self,
        mut _fds: &mut Vec<RawFd>,
        mut opaque: &mut Vec<u8>,
    ) -> Result<(usize, usize)> {
        self.reset()?;
        let size = self.file.read_to_end(&mut opaque).map_err(File)?;
        Ok((size, 0))
    }

    fn destroy(&mut self) -> Result<()> {
        shm_unlink(self.name.as_str())
            .or_else(|err| {
                if err.as_errno() == Some(Errno::ENOENT) {
                    Ok(())
                } else {
                    Err(err)
                }
            })
            .map_err(SharedMem)
    }
}
