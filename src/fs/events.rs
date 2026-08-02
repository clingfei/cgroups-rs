// Copyright (c) 2020 Ant Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

use eventfd::{eventfd, EfdFlags};
use nix::errno::Errno;
use nix::sys::eventfd;
use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use crate::fs::error::ErrorKind::*;
use crate::fs::error::*;

fn read_oom_count(path: &Path) -> Result<u64> {
    let file = File::open(path)
        .map_err(|e| Error::with_cause(ReadFailed(path.display().to_string()), e))?;
    let reader = BufReader::new(file);
    for line in reader.lines() {
        let line =
            line.map_err(|e| Error::with_cause(ReadFailed(path.display().to_string()), e))?;

        let mut parts = line.split_whitespace();
        if let (Some(key), Some(value)) = (parts.next(), parts.next()) {
            if key == "oom" {
                let count: u64 = value
                    .parse::<u64>()
                    .map_err(|e| Error::with_cause(ReadFailed(path.display().to_string()), e))?;
                return Ok(count);
            }
        }
    }
    Err(Error::from_string("oom not found".to_string()))
}

// notify_on_oom returns channel on which you can expect event about OOM,
// if cgroup was destroyed without OOM this channel will be closed.
pub fn notify_on_oom_v2(key: &str, dir: &Path) -> Result<Receiver<String>> {
    let path = dir.join("memory.events");
    let inotify = Inotify::init(InitFlags::IN_CLOEXEC)
        .map_err(|e| Error::with_cause(ReadFailed(path.display().to_string()), e))?;
    inotify
        .add_watch(&path, AddWatchFlags::IN_MODIFY)
        .map_err(|e| Error::with_cause(ReadFailed(path.display().to_string()), e))?;

    // Establish the watch before taking the baseline so changes after the
    // baseline read are guaranteed to have a queued inotify event.
    let mut base = read_oom_count(&path)?;
    let (sender, receiver) = mpsc::channel();
    let key = key.to_string();

    thread::spawn(move || loop {
        let events = match inotify.read_events() {
            Ok(events) => events,
            Err(Errno::EINTR) => continue,
            Err(_) => return,
        };

        for event in events {
            if event
                .mask
                .intersects(AddWatchFlags::IN_IGNORED | AddWatchFlags::IN_UNMOUNT)
            {
                return;
            }

            if event
                .mask
                .intersects(AddWatchFlags::IN_MODIFY | AddWatchFlags::IN_Q_OVERFLOW)
            {
                let count = match read_oom_count(&path) {
                    Ok(count) => count,
                    Err(_) => return,
                };

                if count > base {
                    if sender.send(key.clone()).is_err() {
                        return;
                    }
                    base = count;
                }
            }
        }
    });
    Ok(receiver)
}

// notify_on_oom returns channel on which you can expect event about OOM,
// if cgroup was destroyed without OOM this channel will be closed.
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

    let eventfd = eventfd(0, EfdFlags::EFD_CLOEXEC)
        .map_err(|e| Error::with_cause(ReadFailed("eventfd".to_string()), e))?;

    let event_control_path = cg_dir.join("cgroup.event_control");
    let data = if arg.is_empty() {
        format!("{} {}", eventfd, event_file.as_raw_fd())
    } else {
        format!("{} {} {}", eventfd, event_file.as_raw_fd(), arg)
    };

    // write to file and set mode to 0700(FIXME)
    fs::write(&event_control_path, data.clone()).map_err(|e| {
        Error::with_cause(
            WriteFailed(event_control_path.display().to_string(), data),
            e,
        )
    })?;

    let mut eventfd_file = unsafe { File::from_raw_fd(eventfd) };

    let (sender, receiver) = mpsc::channel();
    let key = key.to_string();

    thread::spawn(move || {
        loop {
            let mut buf = [0; 8];
            if eventfd_file.read(&mut buf).is_err() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cgroups-rs-events-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_read_oom_count() {
        let dir = test_dir();
        let path = dir.join("memory.events");

        // full format, including the newer sock_throttled line
        fs::write(
            &path,
            "low 0\nhigh 0\nmax 0\noom 3\noom_kill 0\noom_group_kill 0\nsock_throttled 0\n",
        )
        .unwrap();
        assert_eq!(read_oom_count(&path).unwrap(), 3);

        // line order must not matter, and missing lines are fine (older kernels)
        fs::write(&path, "oom 7\nlow 0\n").unwrap();
        assert_eq!(read_oom_count(&path).unwrap(), 7);

        // key absent -> error
        fs::write(&path, "low 0\nhigh 0\n").unwrap();
        assert!(read_oom_count(&path).is_err());

        // file absent -> error
        assert!(read_oom_count(&dir.join("nope")).is_err());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_notify_on_oom_v2() {
        let dir = test_dir();
        let events = dir.join("memory.events");
        fs::write(&events, "low 0\nhigh 0\nmax 0\noom 5\noom_kill 0\n").unwrap();

        let rx = notify_on_oom_v2("test-key", &dir).unwrap();

        // Pre-existing OOMs are the baseline, not notified.
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(200)),
            Err(mpsc::RecvTimeoutError::Timeout)
        );

        // A kill-count change without a new OOM must not notify.
        fs::write(&events, "low 0\nhigh 0\nmax 0\noom 5\noom_kill 1\n").unwrap();
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(200)),
            Err(mpsc::RecvTimeoutError::Timeout)
        );

        // One new OOM produces exactly one notification.
        fs::write(&events, "low 0\nhigh 0\nmax 0\noom 6\noom_kill 1\n").unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), "test-key");

        // no duplicate flood while the count stays the same
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(200)),
            Err(mpsc::RecvTimeoutError::Timeout)
        );

        // A second OOM is notified again.
        fs::write(&events, "low 0\nhigh 0\nmax 0\noom 7\noom_kill 1\n").unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), "test-key");

        // Destroying the cgroup closes the channel.
        fs::remove_dir_all(&dir).unwrap();
        let closed = (0..20).any(|_| {
            matches!(
                rx.recv_timeout(Duration::from_millis(200)),
                Err(mpsc::RecvTimeoutError::Disconnected)
            )
        });
        assert!(closed);
    }
}
