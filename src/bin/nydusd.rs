// Copyright 2020 Ant Financial. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.
//
// Copyright 2019 Intel Corporation. All Rights Reserved.
//
// SPDX-License-Identifier: (Apache-2.0 AND BSD-3-Clause)

#[macro_use(crate_version, crate_authors)]
extern crate clap;
#[macro_use]
extern crate log;
extern crate config;
extern crate stderrlog;

use std::fs::File;
use std::io::Result;
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::{convert, error, fmt, io, process};

use libc::EFD_NONBLOCK;

use clap::{App, Arg};
use vm_memory::GuestMemoryMmap;
use vmm_sys_util::eventfd::EventFd;

use fuse::filesystem::FileSystem;
use fuse::server::Server;
use fuse::Error as VhostUserFsError;

use rafs::fs::{Rafs, RafsConfig};
use rafs::storage::oss_backend;

use vhost_rs::descriptor_utils::{Reader, Writer};
use vhost_rs::vhost_user::message::*;
use vhost_rs::vring::{VhostUserBackend, VhostUserDaemon, Vring};

const VIRTIO_F_VERSION_1: u32 = 32;

const QUEUE_SIZE: usize = 1024;
const NUM_QUEUES: usize = 2;

// The guest queued an available buffer for the high priority queue.
const HIPRIO_QUEUE_EVENT: u16 = 0;
// The guest queued an available buffer for the request queue.
const REQ_QUEUE_EVENT: u16 = 1;
// The device has been dropped.
const KILL_EVENT: u16 = 2;

type VhostUserBackendResult<T> = std::result::Result<T, std::io::Error>;

#[derive(Debug)]
enum Error {
    /// Failed to create kill eventfd.
    CreateKillEventFd(io::Error),
    /// Failed to handle event other than input event.
    HandleEventNotEpollIn,
    /// Failed to handle unknown event.
    HandleEventUnknownEvent,
    /// No memory configured.
    NoMemoryConfigured,
    /// Processing queue failed.
    ProcessQueue(VhostUserFsError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "vhost_user_fs_error: {:?}", self)
    }
}

impl error::Error for Error {}

impl convert::From<Error> for io::Error {
    fn from(e: Error) -> Self {
        io::Error::new(io::ErrorKind::Other, e)
    }
}

struct VhostUserFsBackend<F: FileSystem + Send + Sync + 'static> {
    mem: Option<GuestMemoryMmap>,
    kill_evt: EventFd,
    server: Arc<Server<F>>,
}

impl<F: FileSystem + Send + Sync + 'static> VhostUserFsBackend<F> {
    fn new(fs: F) -> Result<Self> {
        Ok(VhostUserFsBackend {
            mem: None,
            kill_evt: EventFd::new(EFD_NONBLOCK).map_err(Error::CreateKillEventFd)?,
            server: Arc::new(Server::new(fs)),
        })
    }
    fn process_queue(&mut self, vring: &mut Vring) -> Result<()> {
        let mem = self.mem.as_ref().ok_or(Error::NoMemoryConfigured)?;

        let mut used_desc_heads = [(0, 0); QUEUE_SIZE];
        let mut used_count = 0;
        while let Some(avail_desc) = vring.mut_queue().iter(&mem).next() {
            let head_index = avail_desc.index;
            let reader = Reader::new(&mem, avail_desc.clone()).unwrap();
            let writer = Writer::new(&mem, avail_desc.clone()).unwrap();

            let total = self
                .server
                .handle_message(reader, writer)
                .map_err(Error::ProcessQueue)?;

            used_desc_heads[used_count] = (head_index, total);
            used_count += 1;
        }

        if used_count > 0 {
            for &(desc_index, _) in &used_desc_heads[..used_count] {
                vring.mut_queue().add_used(&mem, desc_index, 0);
            }
            vring.signal_used_queue().unwrap();
        }

        Ok(())
    }
}

impl<F: FileSystem + Send + Sync + 'static> VhostUserBackend for VhostUserFsBackend<F> {
    fn num_queues(&self) -> usize {
        NUM_QUEUES
    }

    fn max_queue_size(&self) -> usize {
        QUEUE_SIZE
    }

    fn features(&self) -> u64 {
        1 << VIRTIO_F_VERSION_1 | VhostUserVirtioFeatures::PROTOCOL_FEATURES.bits()
    }

    fn protocol_features(&self) -> VhostUserProtocolFeatures {
        // liubo: we haven't supported slave req in rafs.
        VhostUserProtocolFeatures::MQ
    }

    fn update_memory(&mut self, mem: GuestMemoryMmap) -> VhostUserBackendResult<()> {
        self.mem = Some(mem);
        Ok(())
    }

    fn handle_event(
        &mut self,
        index: u16,
        evset: epoll::Events,
        vrings: &[Arc<RwLock<Vring>>],
    ) -> VhostUserBackendResult<bool> {
        if evset != epoll::Events::EPOLLIN {
            return Err(Error::HandleEventNotEpollIn.into());
        }

        match index {
            HIPRIO_QUEUE_EVENT => {
                let mut vring = vrings[HIPRIO_QUEUE_EVENT as usize].write().unwrap();
                // high priority requests are also just plain fuse requests, just in a
                // different queue
                self.process_queue(&mut vring)?;
            }
            x if x >= REQ_QUEUE_EVENT && x < vrings.len() as u16 => {
                let mut vring = vrings[x as usize].write().unwrap();
                self.process_queue(&mut vring)?;
            }
            _ => return Err(Error::HandleEventUnknownEvent.into()),
        }

        Ok(false)
    }

    fn exit_event(&self) -> Option<(EventFd, Option<u16>)> {
        Some((self.kill_evt.try_clone().unwrap(), Some(KILL_EVENT)))
    }
}

fn main() -> Result<()> {
    let cmd_arguments = App::new("vhost-user-fs backend")
        .version(crate_version!())
        .author(crate_authors!())
        .about("Launch a vhost-user-fs backend.")
        .arg(
            Arg::with_name("metadata")
                .long("metadata")
                .help("rafs metadata file")
                .takes_value(true)
                .min_values(1),
        )
        .arg(
            Arg::with_name("sock")
                .long("sock")
                .help("vhost-user socket path")
                .takes_value(true)
                .min_values(1),
        )
        .arg(
            Arg::with_name("config")
                .long("config")
                .help("config file")
                .takes_value(true)
                .min_values(1),
        )
        .get_matches();

    // Retrieve arguments
    let config_file = cmd_arguments
        .value_of("config")
        .expect("config file must be provided");
    let sock = cmd_arguments
        .value_of("sock")
        .expect("Failed to retrieve vhost-user socket path");
    let metadata = cmd_arguments
        .value_of("metadata")
        .expect("Rafs metatada file must be set");

    stderrlog::new()
        .quiet(false)
        .verbosity(log::LevelFilter::Trace as usize)
        .timestamp(stderrlog::Timestamp::Second)
        .init()
        .unwrap();

    let mut settings = config::Config::new();
    settings
        .merge(config::File::from(Path::new(config_file)))
        .expect("failed to open config file");
    let rafs_conf: RafsConfig = settings.try_into().expect("Invalid config");

    let backend = oss_backend::new();
    let mut rafs = Rafs::new(rafs_conf, backend);

    /* example code to call pseudofs
    let rafs2 = Rafs::new(RafsConfig::new(), oss_backend::new());
    let vfs = PseudoFs::new();
    vfs.mount(rafs2, "/etc")?;
    */

    let mut file = File::open(metadata)?;
    rafs.mount(&mut file, "/")?;
    info!("rafs mounted");

    let fs_backend = Arc::new(RwLock::new(VhostUserFsBackend::new(rafs).unwrap()));

    let mut daemon = VhostUserDaemon::new(
        String::from("vhost-user-fs-backend"),
        String::from(sock),
        fs_backend.clone(),
    )
    .unwrap();

    info!("starting fuse daemon");
    if let Err(e) = daemon.start() {
        error!("Failed to start daemon: {:?}", e);
        process::exit(1);
    }

    if let Err(e) = daemon.wait() {
        error!("Waiting for daemon failed: {:?}", e);
    }

    let kill_evt = &fs_backend.read().unwrap().kill_evt;
    if let Err(e) = kill_evt.write(1) {
        error!("Error shutting down worker thread: {:?}", e)
    }

    info!("nydusd quits");
    Ok(())
}
