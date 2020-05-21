// Copyright (C) 2020 Alibaba Cloud. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use std::fs::File;
use std::io::{Error, Result};
use std::mem::size_of;
use std::os::unix::io::{FromRawFd, RawFd};
use std::sync::Arc;

use crate::metadata::layout::*;
use crate::metadata::*;

struct DirectMapping {
    base: *const u8,
    end: *const u8,
    size: usize,
}

impl DirectMapping {
    fn new() -> Self {
        DirectMapping {
            base: std::ptr::null(),
            end: std::ptr::null(),
            size: 0,
        }
    }

    fn from_raw_fd(fd: RawFd) -> Result<Self> {
        let file = unsafe { File::from_raw_fd(fd) };
        let md = file.metadata()?;
        let len = md.len();
        if len < RAFS_SUPERBLOCK_SIZE as u64
            || len > RAFS_MAX_METADATA_SIZE as u64
            || len & (RAFS_ALIGNMENT as u64 - 1) != 0
        {
            return Err(ebadf());
        }
        let size = len as usize;
        let base = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                size,
                libc::PROT_READ,
                libc::MAP_NORESERVE | libc::MAP_PRIVATE,
                fd,
                0,
            )
        } as *const u8;
        // Safe because the mmap area should covered the range [start, end)
        let end = unsafe { base.add(size) };

        Ok(DirectMapping { base, end, size })
    }

    fn cast_to_ref<'a, 'b, T>(&'a self, base: *const u8, offset: usize) -> Result<&'b T> {
        let start = base.wrapping_add(offset);
        let end = start.wrapping_add(size_of::<T>());

        if start < self.base || end < self.base || end > self.end
        // || start as usize & (std::mem::align_of::<T>() - 1) != 0
        {
            return Err(einval());
        }

        Ok(unsafe { &*(start as *const T) })
    }
}

impl Drop for DirectMapping {
    fn drop(&mut self) {
        if self.base != std::ptr::null() {
            unsafe { libc::munmap(self.base as *mut u8 as *mut libc::c_void, self.size) };
            self.base = std::ptr::null();
            self.end = std::ptr::null();
            self.size = 0;
        }
    }
}

pub struct DirectMapInodes {
    // TODO: use ArcSwap here to support swapping underlying metadata file.
    mapping: Arc<DirectMapping>,
    inode_table: Arc<OndiskInodeTable>,
    blob_table: Arc<OndiskBlobTable>,
}

impl DirectMapInodes {
    pub fn new(inode_table: Arc<OndiskInodeTable>, blob_table: Arc<OndiskBlobTable>) -> Self {
        DirectMapInodes {
            mapping: Arc::new(DirectMapping::new()),
            inode_table,
            blob_table,
        }
    }
}

impl RafsSuperInodes for DirectMapInodes {
    fn load(&mut self, _sb: &mut RafsSuperMeta, r: &mut RafsIoReader) -> Result<()> {
        let fd = unsafe { libc::dup(r.as_raw_fd()) };
        if fd < 0 {
            return Err(Error::last_os_error());
        }

        let mapping = DirectMapping::from_raw_fd(fd)?;
        self.mapping = Arc::new(mapping);

        Ok(())
    }

    fn destroy(&mut self) {
        self.mapping = Arc::new(DirectMapping::new());
    }

    fn get_inode(&self, ino: u64) -> Result<&dyn RafsInode> {
        let offset = self.inode_table.get(ino)?;
        let inode = self
            .mapping
            .cast_to_ref::<OndiskInode>(self.mapping.base, offset as usize)?;
        Ok(inode as &dyn RafsInode)
    }

    fn get_blob_id<'a>(&'a self, idx: u32) -> Result<&'a OndiskDigest> {
        self.blob_table.get(idx)
    }

    fn get_chunk_info(&self, inode: &dyn RafsInode, idx: u64) -> Result<&OndiskChunkInfo> {
        let ptr = inode as *const dyn RafsInode as *const u8;
        let chunk = ptr
            .wrapping_add(size_of::<OndiskInode>() + size_of::<OndiskChunkInfo>() * idx as usize);

        self.mapping.cast_to_ref::<OndiskChunkInfo>(chunk, 0)
    }

    fn get_symlink(&self, inode: &dyn RafsInode) -> Result<OndiskSymlinkInfo> {
        let sz = inode.chunk_cnt() as usize * RAFS_ALIGNMENT;
        if sz == 0 || sz > (libc::PATH_MAX as usize) + RAFS_ALIGNMENT - 1 {
            return Err(ebadf());
        }

        let start = (inode as *const dyn RafsInode as *const u8).wrapping_add(RAFS_INODE_INFO_SIZE);
        let input = unsafe { std::slice::from_raw_parts(start, sz) };

        Ok(OndiskSymlinkInfo {
            data: input.to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::layout::{
        save_symlink_ondisk, OndiskInodeTable, OndiskSuperBlock, INO_FLAG_SYMLINK,
    };
    use crate::metadata::CachedIoBuf;
    use crate::metadata::{calc_symlink_size, RafsSuper, RAFS_INODE_BLOCKSIZE};
    use fuse_rs::api::filesystem::ROOT_ID;

    #[test]
    fn test_rafs_directmap_load_v5() {
        let mut buf = CachedIoBuf::new();

        let mut sb = OndiskSuperBlock::new();
        sb.set_inode_size(4);
        sb.set_inode_table_offset(RAFS_SUPERBLOCK_SIZE as u64);
        buf.write_all(sb.as_ref()).unwrap();

        let mut table = vec![0u8; 32];
        table[4] = 0x4;
        table[5] = 0x4;
        table[8] = 0x44;
        table[9] = 0x4;
        table[12] = 0xa4;
        table[13] = 0x4;
        table[16] = 0xe4;
        table[17] = 0x4;
        buf.write_all(table.as_ref()).unwrap();

        let mut ondisk = OndiskInode::new();
        ondisk.set_name("root").unwrap();
        ondisk.set_parent(ROOT_ID);
        ondisk.set_ino(ROOT_ID);
        ondisk.set_mode(libc::S_IFDIR);
        buf.append_buf(ondisk.as_ref());

        let mut ondisk = OndiskInode::new();
        ondisk.set_name("a").unwrap();
        ondisk.set_parent(ROOT_ID);
        ondisk.set_ino(ROOT_ID + 1);
        ondisk.set_chunk_cnt(2);
        ondisk.set_mode(libc::S_IFREG);
        ondisk.set_size(RAFS_INODE_BLOCKSIZE as u64 * 2);
        buf.append_buf(ondisk.as_ref());
        let mut ondisk = OndiskChunkInfo::new();
        ondisk.set_blob_offset(0);
        ondisk.set_compress_size(5);
        buf.append_buf(ondisk.as_ref());
        let mut ondisk = OndiskChunkInfo::new();
        ondisk.set_blob_offset(10);
        ondisk.set_compress_size(5);
        buf.append_buf(ondisk.as_ref());

        let mut ondisk = OndiskInode::new();
        ondisk.set_name("b").unwrap();
        ondisk.set_parent(ROOT_ID);
        ondisk.set_ino(ROOT_ID + 2);
        ondisk.set_mode(libc::S_IFDIR);
        buf.append_buf(ondisk.as_ref());

        let mut ondisk = OndiskInode::new();
        ondisk.set_name("c").unwrap();
        ondisk.set_parent(ROOT_ID + 2);
        ondisk.set_ino(ROOT_ID + 3);
        ondisk.set_mode(libc::S_IFLNK);
        let (_, chunks) = calc_symlink_size("/a/b/d".len()).unwrap();
        ondisk.set_chunk_cnt(chunks as u64);
        ondisk.set_flags(INO_FLAG_SYMLINK);
        buf.append_buf(ondisk.as_ref());
        let mut buf1: Box<dyn RafsIoWrite> = Box::new(buf.clone());
        save_symlink_ondisk("/a/b/d".as_bytes(), &mut buf1).unwrap();

        let (base, size) = buf.as_buf();
        let end = unsafe { base.add(size) };
        let mut mapping_table = OndiskInodeTable::new(4);
        mapping_table.set(ROOT_ID, 0x404).unwrap();
        mapping_table.set(ROOT_ID + 1, 0x444).unwrap();
        mapping_table.set(ROOT_ID + 2, 0x4a4).unwrap();
        mapping_table.set(ROOT_ID + 3, 0x4e4).unwrap();
        let mut inodes = DirectMapInodes::new(Arc::new(mapping_table));
        inodes.mapping = Arc::new(DirectMapping { base, end, size });

        let mut sb2 = RafsSuper::new();
        sb2.s_inodes = Box::new(inodes);
        sb2.s_meta.s_magic = sb.magic();
        sb2.s_meta.s_version = sb.version();
        sb2.s_meta.s_sb_size = sb.sb_size();
        sb2.s_meta.s_inode_size = sb.inode_size();
        sb2.s_meta.s_block_size = sb.block_size();
        sb2.s_meta.s_chunkinfo_size = sb.chunkinfo_size();
        sb2.s_meta.s_flags = sb.flags();
        sb2.s_meta.s_blocks_count = 0;
        sb2.s_meta.s_inodes_count = sb.inodes_count();
        sb2.s_meta.s_inode_table_entries = sb.inode_table_entries();
        sb2.s_meta.s_inode_table_offset = sb.inode_table_offset();

        let inode = sb2.s_inodes.get_inode(ROOT_ID).unwrap();
        assert_eq!(inode.ino(), ROOT_ID);
        assert_eq!(inode.parent(), ROOT_ID);
        assert_eq!(inode.is_dir(), true);

        let inode = sb2.s_inodes.get_inode(ROOT_ID + 1).unwrap();
        assert_eq!(inode.ino(), ROOT_ID + 1);
        assert_eq!(inode.parent(), ROOT_ID);
        assert_eq!(inode.is_reg(), true);
        assert_eq!(inode.chunk_cnt(), 2);
        // TODO: chunk

        let inode = sb2.s_inodes.get_inode(ROOT_ID + 2).unwrap();
        assert_eq!(inode.ino(), ROOT_ID + 2);
        assert_eq!(inode.parent(), ROOT_ID);
        assert_eq!(inode.is_dir(), true);

        let inode = sb2.s_inodes.get_inode(ROOT_ID + 3).unwrap();
        assert_eq!(inode.name(), "c");
        assert_eq!(inode.ino(), ROOT_ID + 3);
        assert_eq!(inode.parent(), ROOT_ID + 2);
        assert_eq!(inode.is_symlink(), true);
        assert_eq!(inode.chunk_cnt(), 1);
        assert_eq!(inode.get_symlink(&sb2).unwrap(), "/a/b/d".as_bytes());
    }
}
