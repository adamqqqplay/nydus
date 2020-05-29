use std::fs::{self, File};
use std::io::{Error, ErrorKind, Result, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread::*;
use std::time;

const NYDUSD: &str = "./target-fusedev/debug/nydusd";

pub fn exec(cmd: &str) -> Result<()> {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()?;
    let status = child.wait()?;

    let status = status
        .code()
        .ok_or(Error::new(ErrorKind::Other, "exited with unknown status"))?;

    if status != 0 {
        return Err(Error::new(ErrorKind::Other, "exited with non-zero"));
    }

    Ok(())
}

pub struct Nydusd {
    work_dir: PathBuf,
    mount_path: PathBuf,
}

pub fn new(work_dir: &PathBuf) -> Result<Nydusd> {
    let mount_path = work_dir.join("mnt");
    fs::create_dir_all(mount_path.clone())?;

    let config = format!(
        r###"
        {{
            "device": {{
                "backend": {{
                    "type": "localfs",
                    "config": {{
                        "dir": {:?}
                    }}
                }},
                "rafs": {{
                    "mode": "direct"
                }}
            }},
            "mode": "direct"
        }}
        "###,
        work_dir.join("blobs"),
    );

    File::create(work_dir.join("config.json"))?.write_all(config.as_bytes())?;

    Ok(Nydusd {
        work_dir: work_dir.clone(),
        mount_path,
    })
}

impl Drop for Nydusd {
    fn drop(&mut self) {
        exec(format!("sudo pkill nydusd").as_str()).unwrap();
        exec(format!("sudo umount -l {:?}", self.mount_path).as_str()).unwrap();
    }
}

impl Nydusd {
    pub fn start(&self) -> Result<()> {
        let work_dir = self.work_dir.clone();
        let mount_path = self.mount_path.clone();

        spawn(move || {
            exec(
                format!(
                    "sudo {} --config {:?} --apisock {:?} --mountpoint {:?} --metadata {:?} --log-level error",
                    NYDUSD,
                    work_dir.join("config.json"),
                    work_dir.join("api.sock"),
                    mount_path,
                    work_dir.join("parent-bootstrap"),
                )
                .as_str(),
            ).unwrap_or(());
        });

        sleep(time::Duration::from_secs(1));

        Ok(())
    }
}
