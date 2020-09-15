// Copyright 2020 Ant Financial. All rights reserved.
// Copyright (C) 2020 Alibaba Cloud. All rights reserved.
//
// SPDX-License-Identifier: (Apache-2.0 AND BSD-3-Clause)

use nydus_utils::last_error;
use sendfd::{RecvWithFd, SendWithFd};
use std::any::Any;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io;
use std::io::Result;
use std::net::Shutdown;
use std::ops::Deref;
use std::os::unix::io::{FromRawFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::{
    atomic::{AtomicI32, Ordering},
    Arc, Mutex,
};
use std::thread;

use serde::{Deserialize, Serialize};

use fuse_rs::api::{server::Server, Vfs};
use nydus_utils::{einval, eio, FuseChannel, FuseSession};
use vmm_sys_util::eventfd::EventFd;

use crate::daemon;
use daemon::{Error, NydusDaemon};

use crate::upgrade_manager::{Resource, ResourceType, UPGRADE_MRG};
use crate::EVENT_MANAGER_RUN;

struct FuseServer {
    server: Arc<Server<Arc<Vfs>>>,
    ch: FuseChannel,
    // read buffer for fuse requests
    buf: Vec<u8>,
    evtfd: EventFd,
}

impl FuseServer {
    fn new(server: Arc<Server<Arc<Vfs>>>, se: &FuseSession, evtfd: EventFd) -> Result<FuseServer> {
        Ok(FuseServer {
            server,
            ch: se.new_channel(evtfd.try_clone().unwrap())?,
            buf: Vec::with_capacity(se.bufsize()),
            evtfd,
        })
    }

    fn svc_loop(&mut self) -> Result<()> {
        // Safe because we have already reserved the capacity
        unsafe {
            self.buf.set_len(self.buf.capacity());
        }

        // Given error EBADF, it means kernel has shut down this session.
        let _ebadf = std::io::Error::from_raw_os_error(libc::EBADF);
        loop {
            if let Some(reader) = self.ch.get_reader(&mut self.buf)? {
                let writer = self.ch.get_writer()?;
                if let Err(e) = self.server.handle_message(reader, writer, None) {
                    match e {
                        fuse_rs::Error::EncodeMessage(_ebadf) => {
                            return Err(eio!("fuse session has been shut down: {:?}"));
                        }
                        _ => {
                            error!("Handling fuse message, {}", Error::ProcessQueue(e));
                            continue;
                        }
                    }
                }
            } else {
                info!("fuse server exits");
                break;
            }
        }
        Ok(())
    }
}
#[allow(dead_code)]
#[derive(Hash, PartialEq, Eq)]
enum DaemonStatus {
    INIT,
    RUNNING,
    STOP,
    UPGRADE,
}

struct FusedevDaemon {
    server: Arc<Server<Arc<Vfs>>>,
    pub session: FuseSession,
    threads: Vec<Option<thread::JoinHandle<Result<()>>>>,
    event_fd: EventFd,
}

impl FusedevDaemon {
    fn kick_one_server(&mut self) -> Result<()> {
        let mut s = FuseServer::new(
            self.server.clone(),
            &self.session,
            // Clone event fd must succeed, otherwise fusedev daemon should not work.
            self.event_fd.try_clone().unwrap(),
        )?;

        let thread = thread::Builder::new()
            .name("fuse_server".to_string())
            .spawn(move || {
                let _ = s.svc_loop();
                EVENT_MANAGER_RUN.store(false, Ordering::Relaxed);
                s.evtfd.write(1)
            })
            .map_err(Error::ThreadSpawn)?;
        self.threads.push(Some(thread));
        Ok(())
    }
}

impl NydusDaemon for FusedevDaemon {
    fn start(&mut self, cnt: u32) -> Result<()> {
        for _ in 0..cnt {
            self.kick_one_server()?;
        }
        Ok(())
    }

    fn wait(&mut self) -> Result<()> {
        for t in &mut self.threads {
            if let Some(handle) = t.take() {
                handle.join().map_err(|_| Error::WaitDaemon)??;
            }
        }
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        self.event_fd.write(1).expect("Stop fuse service loop");
        self.session.umount()
    }

    fn as_any(&mut self) -> &mut dyn Any {
        self
    }
}

pub fn create_nydus_daemon(
    mountpoint: &str,
    fs: Arc<Vfs>,
    evtfd: EventFd,
    supervisor: Option<OsString>,
    id: Option<String>,
    upgrade: bool,
) -> Result<Arc<Mutex<dyn NydusDaemon + Send>>> {
    let mut se = FuseSession::new(Path::new(mountpoint), "rafs", "")?;

    let mut fuse_fd = None;
    if !upgrade {
        se.mount()?;
        fuse_fd = Some(se.expose_fuse_fd());
    }

    let daemon = Arc::new(Mutex::new(FusedevDaemon {
        session: se,
        server: Arc::new(Server::new(fs)),
        threads: Vec::new(),
        event_fd: evtfd,
    }));

    if let Some(id) = id {
        if let Some(supervisor) = supervisor {
            let res = FuseDevFdRes::new(fuse_fd, supervisor.as_ref(), id, daemon.clone());

            UPGRADE_MRG
                .lock()
                .expect("Not expect a poisoned Upgrade Manger lock!")
                .add_resource(res, ResourceType::Fd);
        }
    }

    Ok(daemon)
}

#[derive(Default, Debug, Serialize, Deserialize)]
struct ResOpaque {
    version: u32,
    daemon_id: String,
    opaque: String,
}

pub struct FuseDevFdRes {
    fuse_fd: AtomicI32,
    uds_path: OsString,
    stream: Arc<Mutex<Option<UnixStream>>>,
    daemon_id: String,
    daemon: Arc<Mutex<dyn NydusDaemon + Send>>,
}

impl FuseDevFdRes {
    fn new(
        fd: Option<RawFd>,
        uds: &OsStr,
        daemon_id: String,
        daemon: Arc<Mutex<dyn NydusDaemon + Send>>,
    ) -> Self {
        FuseDevFdRes {
            fuse_fd: fd.map(AtomicI32::new).unwrap_or_else(|| AtomicI32::new(-1)),
            uds_path: uds.to_os_string(),
            stream: Default::default(),
            daemon_id,
            daemon,
        }
    }

    // TODO: unlink unix domain socket when drop such resource.
    pub fn connect(&self) -> Result<()> {
        let stream = UnixStream::connect(&self.uds_path).map_err(|e| {
            error!("Connect to {:?} failed, {:?}", &self.uds_path, e);
            e
        })?;
        *self.stream.lock().unwrap() = Some(stream);
        Ok(())
    }

    #[allow(dead_code)]
    fn listen(&self, path: &OsStr) -> Result<UnixListener> {
        std::fs::remove_file(path).unwrap_or_default();
        UnixListener::bind(path).map_err(|e| last_error!(e))
    }

    fn send_fd(&self) -> Result<usize> {
        if let Some(ref sock) = self.stream.lock().unwrap().deref() {
            let opaque = ResOpaque {
                version: 1,
                daemon_id: self.daemon_id.clone(),
                ..Default::default()
            };

            let opaque_buf = serde_json::to_string(&opaque).unwrap().into_bytes();
            let mut fds: [RawFd; 8] = Default::default();
            fds[0] = self.fuse_fd.load(Ordering::Acquire);
            sock.send_with_fd(&opaque_buf, &fds)
                .map_err(|_| last_error!())
        } else {
            error!("Send fd error!");
            Err(io::Error::from_raw_os_error(libc::ENOTCONN))
        }
    }

    fn recv_fd(&self) -> Result<ResOpaque> {
        // TODO: Is 8K buffer large enough?
        let mut opaque = vec![0u8; 8192];
        let mut fds: [RawFd; 8] = Default::default();
        let (opaque_size, fds_count) = self
            .stream
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .recv_with_fd(&mut opaque, &mut fds)
            .map_err(|e| {
                error!("Failed in receiving fd");
                e
            })?;

        if fds_count != 1 {
            warn!("There should be only one fd sent, but {} comes", fds_count);
        }

        debug!("daemon id is {}", self.daemon_id);

        self.fuse_fd.store(fds[1], Ordering::Release);

        serde_json::from_str::<ResOpaque>(
            std::str::from_utf8(&opaque[..opaque_size]).map_err(|e| einval!(e))?,
        )
        .map_err(|e| {
            error!(" Opaque can't ba parsed, {} ", e);
            einval!(e)
        })
    }
}

impl Resource for FuseDevFdRes {
    fn store(&self) -> Result<()> {
        self.connect()?;
        self.send_fd()?;
        self.stream
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .shutdown(Shutdown::Both)?;

        Ok(())
    }

    fn load(&self) -> Result<()> {
        self.connect()?;
        let _opaque = self.recv_fd()?;
        // TODO: Read config file again? or store config as opaque into backend?
        // FIXME:
        let mut d_guard = self.daemon.lock().unwrap();
        let d = d_guard.as_any().downcast_mut::<FusedevDaemon>().unwrap();

        d.session.file = unsafe { Some(File::from_raw_fd(self.fuse_fd.load(Ordering::Acquire))) };

        self.daemon.lock().unwrap().start(4)?;

        Ok(())
    }
}
