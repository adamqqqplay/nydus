// Copyright 2020 Ant Financial. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.
//
// A container image Registry Accerlation File System.

use std::collections::BTreeMap;
use std::ffi::CStr;
use std::io;
use std::mem;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use crate::dag::Dag;
use crate::storage::backend;
use crate::storage::device;
use crate::storage::device::*;

use fuse::filesystem::*;

type Inode = u64;
type Handle = u64;

struct RafsInode {
    i_no: Inode,
}

struct RafsSuper {
    s_magic: u32,
    s_version: u32,
    s_root: Dag,
    s_inodes: RwLock<BTreeMap<Inode, Arc<RafsInode>>>,
}

#[derive(Clone, Default)]
pub struct RafsConfig {
    source: String,
    device_config: device::Config,
}

impl RafsConfig {
    pub fn new() -> RafsConfig {
        RafsConfig {
            ..Default::default()
        }
    }

    fn dev_config(&self) -> device::Config {
        let mut c = device::Config::new();
        c.backend_type = self.device_config.backend_type;
        c.id = String::from(&self.device_config.id);
        c.path = String::from(&self.device_config.path);
        c.secret = String::from(&self.device_config.secret);
        c
    }
}

pub struct Rafs<B: backend::BlobBackend + 'static> {
    conf: RafsConfig,

    sb: RafsSuper,
    device: device::RafsDevice<B>,
}

impl<B: backend::BlobBackend + 'static> Rafs<B> {
    pub fn new(conf: RafsConfig, b: B) -> Self {
        Rafs {
            sb: RafsSuper {
                s_magic: 100,
                s_version: 1,
                s_root: Dag::new(),
                s_inodes: RwLock::new(BTreeMap::new()),
            },
            device: device::RafsDevice::new(conf.dev_config(), b),
            conf: conf,
        }
    }

    fn mount(&self) -> io::Result<()> {
        Ok(())
    }

    fn umount(&self) -> io::Result<()> {
        Ok(())
    }
}

impl<B: backend::BlobBackend + 'static> FileSystem for Rafs<B> {
    type Inode = Inode;
    type Handle = Handle;

    fn init(&self, _: FsOptions) -> io::Result<FsOptions> {
        let data = RafsInode { i_no: ROOT_ID };
        self.sb
            .s_inodes
            .write()
            .unwrap()
            .insert(ROOT_ID, Arc::new(data));

        Ok(
            // These fuse features are supported by rafs by default.
            FsOptions::ASYNC_READ
                | FsOptions::PARALLEL_DIROPS
                | FsOptions::BIG_WRITES
                | FsOptions::HANDLE_KILLPRIV
                | FsOptions::ASYNC_DIO
                | FsOptions::HAS_IOCTL_DIR
                | FsOptions::WRITEBACK_CACHE
                | FsOptions::ZERO_MESSAGE_OPEN
                | FsOptions::ATOMIC_O_TRUNC,
        )
    }

    fn destroy(&self) {
        self.sb.s_inodes.write().unwrap().clear();
    }

    fn lookup(&self, ctx: Context, parent: Self::Inode, name: &CStr) -> io::Result<Entry> {
        Err(io::Error::from_raw_os_error(libc::ENOSYS))
    }

    fn forget(&self, ctx: Context, inode: Self::Inode, count: u64) {}

    fn batch_forget(&self, ctx: Context, requests: Vec<(Self::Inode, u64)>) {
        for (inode, count) in requests {
            self.forget(ctx, inode, count)
        }
    }

    fn getattr(
        &self,
        ctx: Context,
        inode: Self::Inode,
        handle: Option<Self::Handle>,
    ) -> io::Result<(libc::stat64, Duration)> {
        Err(io::Error::from_raw_os_error(libc::ENOSYS))
    }

    fn readlink(&self, ctx: Context, inode: Self::Inode) -> io::Result<Vec<u8>> {
        Err(io::Error::from_raw_os_error(libc::ENOSYS))
    }

    fn open(
        &self,
        ctx: Context,
        inode: Self::Inode,
        flags: u32,
    ) -> io::Result<(Option<Self::Handle>, OpenOptions)> {
        // Matches the behavior of libfuse.
        Ok((None, OpenOptions::empty()))
    }

    #[allow(clippy::too_many_arguments)]
    fn read<W: io::Write + ZeroCopyWriter>(
        &self,
        ctx: Context,
        inode: Self::Inode,
        handle: Self::Handle,
        w: W,
        size: u32,
        offset: u64,
        lock_owner: Option<u64>,
        flags: u32,
    ) -> io::Result<usize> {
        //TODO: fill in properly
        let bio = RafsBio {
            ..Default::default()
        };
        let mut desc = RafsBioDesc {
            ..Default::default()
        };
        desc.bi_vec.push(bio);
        self.device.read_to(w, desc)
    }

    #[allow(clippy::too_many_arguments)]
    fn write<R: io::Read + ZeroCopyReader>(
        &self,
        ctx: Context,
        inode: Self::Inode,
        handle: Self::Handle,
        r: R,
        size: u32,
        offset: u64,
        lock_owner: Option<u64>,
        delayed_write: bool,
        flags: u32,
    ) -> io::Result<usize> {
        //TODO: fill in properly
        let bio = RafsBio {
            ..Default::default()
        };
        let mut desc = RafsBioDesc {
            ..Default::default()
        };
        desc.bi_vec.push(bio);
        self.device.write_from(r, desc)
    }

    fn release(
        &self,
        ctx: Context,
        inode: Self::Inode,
        flags: u32,
        handle: Self::Handle,
        flush: bool,
        flock_release: bool,
        lock_owner: Option<u64>,
    ) -> io::Result<()> {
        Ok(())
    }

    fn statfs(&self, ctx: Context, inode: Self::Inode) -> io::Result<libc::statvfs64> {
        // Safe because we are zero-initializing a struct with only POD fields.
        let mut st: libc::statvfs64 = unsafe { mem::zeroed() };

        // This matches the behavior of libfuse as it returns these values if the
        // filesystem doesn't implement this method.
        st.f_namemax = 255;
        st.f_bsize = 512;

        Ok(st)
    }

    fn getxattr(
        &self,
        ctx: Context,
        inode: Self::Inode,
        name: &CStr,
        size: u32,
    ) -> io::Result<GetxattrReply> {
        Err(io::Error::from_raw_os_error(libc::ENOSYS))
    }

    fn listxattr(&self, ctx: Context, inode: Self::Inode, size: u32) -> io::Result<ListxattrReply> {
        Err(io::Error::from_raw_os_error(libc::ENOSYS))
    }

    fn opendir(
        &self,
        ctx: Context,
        inode: Self::Inode,
        flags: u32,
    ) -> io::Result<(Option<Self::Handle>, OpenOptions)> {
        Err(io::Error::from_raw_os_error(libc::ENOSYS))
    }

    fn readdir<F>(
        &self,
        ctx: Context,
        inode: Self::Inode,
        handle: Self::Handle,
        size: u32,
        offset: u64,
        add_entry: F,
    ) -> io::Result<()>
    where
        F: FnMut(DirEntry) -> io::Result<usize>,
    {
        Err(io::Error::from_raw_os_error(libc::ENOSYS))
    }

    fn readdirplus<F>(
        &self,
        ctx: Context,
        inode: Self::Inode,
        handle: Self::Handle,
        size: u32,
        offset: u64,
        add_entry: F,
    ) -> io::Result<()>
    where
        F: FnMut(DirEntry, Entry) -> io::Result<usize>,
    {
        Err(io::Error::from_raw_os_error(libc::ENOSYS))
    }

    fn releasedir(
        &self,
        ctx: Context,
        inode: Self::Inode,
        flags: u32,
        handle: Self::Handle,
    ) -> io::Result<()> {
        Ok(())
    }

    fn access(&self, ctx: Context, inode: Self::Inode, mask: u32) -> io::Result<()> {
        Err(io::Error::from_raw_os_error(libc::ENOSYS))
    }
}
