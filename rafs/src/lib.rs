// Copyright 2020 Ant Financial. All rights reserved.
// Use of this source code is governed by a Apache 2.0 license that can be
// found in the LICENSE file.

//! A readonly filesystem with separated metadata and data, to support on-demand loading.

#[macro_use]
extern crate log;
extern crate serde;
#[macro_use]
extern crate serde_derive;

use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;

pub mod fs;
pub mod metadata;
pub mod storage;

#[macro_use]
extern crate lazy_static;
#[allow(dead_code)]
pub mod io_stats;

/// A helper trait for RafsIoReader.
pub trait RafsIoRead: Read + AsRawFd {}

/// A helper trait for RafsIoWriter.
pub trait RafsIoWrite: Write {}

impl RafsIoRead for File {}
impl RafsIoWrite for File {}

/// Handler to read file system metadata.
pub type RafsIoReader = Box<dyn RafsIoRead>;

/// Handler to write file system metadata.
pub type RafsIoWriter = Box<dyn RafsIoWrite>;

pub(crate) fn ebadf() -> std::io::Error {
    std::io::Error::from_raw_os_error(libc::EBADF)
}

pub(crate) fn enosys() -> std::io::Error {
    std::io::Error::from_raw_os_error(libc::ENOSYS)
}

pub(crate) fn einval() -> std::io::Error {
    std::io::Error::from_raw_os_error(libc::EINVAL)
}

pub(crate) fn enoent() -> std::io::Error {
    std::io::Error::from_raw_os_error(libc::ENOENT)
}

pub(crate) fn enoattr() -> std::io::Error {
    std::io::Error::from_raw_os_error(libc::ENODATA)
}

pub(crate) fn eaccess() -> std::io::Error {
    std::io::Error::from_raw_os_error(libc::EACCES)
}
