// Copyright (c) 2020 Ant Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

use nix::sys::eventfd::{EfdFlags, EventFd};
use std::fs::{self, File};
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use crate::fs::error::ErrorKind::*;
use crate::fs::error::*;

// notify_on_oom returns channel on which you can expect event about OOM,
// if process died without OOM this channel will be closed.
pub fn notify_on_oom_v2(key: &str, dir: &Path) -> Result<Receiver<String>> {
    register_memory_event(key, dir, "memory.oom_control", "")
}

// notify_on_oom returns channel on which you can expect event about OOM,
// if process died without OOM this channel will be closed.
pub fn notify_on_oom_v1(key: &str, dir: &Path) -> Result<Receiver<String>> {
    register_memory_event(key, dir, "memory.oom_control", "")
}

// level is one of "low", "medium", or "critical"
pub fn notify_memory_pressure(key: &str, dir: &Path, level: &str) -> Result<Receiver<String>> {
    if level != "low" && level != "medium" && level != "critical" {
        return Err(Error::from_string(format!(
            "invalid pressure level {}",
            level
        )));
    }

    register_memory_event(key, dir, "memory.pressure_level", level)
}

fn register_memory_event(
    key: &str,
    cg_dir: &Path,
    event_name: &str,
    arg: &str,
) -> Result<Receiver<String>> {
    let path = cg_dir.join(event_name);
    let event_file = File::open(path.clone())
        .map_err(|e| Error::with_cause(ReadFailed(path.display().to_string()), e))?;

    let eventfd = EventFd::from_flags(EfdFlags::EFD_CLOEXEC)
        .map_err(|e| Error::with_cause(ReadFailed("eventfd".to_string()), e))?;

    let event_control_path = cg_dir.join("cgroup.event_control");
    let data = if arg.is_empty() {
        format!("{} {}", eventfd.as_raw_fd(), event_file.as_raw_fd())
    } else {
        format!("{} {} {}", eventfd.as_raw_fd(), event_file.as_raw_fd(), arg)
    };

    // write to file and set mode to 0700(FIXME)
    fs::write(&event_control_path, data.clone()).map_err(|e| {
        Error::with_cause(
            WriteFailed(event_control_path.display().to_string(), data),
            e,
        )
    })?;

    let (sender, receiver) = mpsc::channel();
    let key = key.to_string();

    thread::spawn(move || {
        loop {
            if eventfd.read().is_err() {
                return;
            }

            // When a cgroup is destroyed, an event is sent to eventfd.
            // So if the control path is gone, return instead of notifying.
            if !Path::new(&event_control_path).exists() {
                return;
            }
            sender.send(key.clone()).unwrap();
        }
    });

    Ok(receiver)
}
