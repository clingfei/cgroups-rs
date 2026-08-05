// Copyright (c) 2018 Levente Kurusa
// Copyright (c) 2020 Ant Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

//! Integration tests for the freezer controller on both cgroup versions.
//!
//! These tests move a forked child process into the cgroup and freeze it,
//! keeping the test process itself outside so it can thaw the child again.

use std::thread::sleep;
use std::time::Duration;

use nix::unistd::{fork, ForkResult};

use cgroups_rs::fs::freezer::FreezerController;
use cgroups_rs::fs::Cgroup;
use cgroups_rs::{CgroupPid, FreezerState};

/// Polls the freezer state until it reaches the expected value, tolerating
/// transient states such as `Freezing`.
fn wait_for_state(freezer: &FreezerController, expected: FreezerState) {
    for _ in 0..100 {
        if freezer.state().unwrap() == expected {
            return;
        }
        sleep(Duration::from_millis(10));
    }
    assert_eq!(freezer.state().unwrap(), expected);
}

/// Moves `task` into the cgroup, freezes it, verifies the state, thaws it
/// and moves the task back out.
///
/// On v2 the task is moved through `cgroup.procs` (`add_task_by_tgid`),
/// because `add_task` writes `cgroup.threads`, which is only writable in
/// threaded mode. On v1 the issue's original `add_task` flow is used.
fn freeze_and_thaw(cg: &Cgroup, task: u64) {
    let freezer: &FreezerController = cg.controller_of().expect("freezer controller not found");
    let task = CgroupPid::from(task);

    if cg.v2() {
        cg.add_task_by_tgid(task).unwrap();
    } else {
        cg.add_task(task).unwrap();
    }
    freezer.freeze().unwrap();
    wait_for_state(freezer, FreezerState::Frozen);
    freezer.thaw().unwrap();
    wait_for_state(freezer, FreezerState::Thawed);
    if cg.v2() {
        cg.remove_task_by_tgid(task).unwrap();
    } else {
        cg.remove_task(task).unwrap();
    }
}

/// Forks a child that just sleeps, runs `f` in the parent, then kills and
/// reaps the child.
fn with_sleeping_child(f: impl FnOnce(u64)) {
    match unsafe { fork() }.unwrap() {
        ForkResult::Child => {
            sleep(Duration::from_secs(60));
            std::process::exit(0);
        }
        ForkResult::Parent { child } => {
            f(child.as_raw() as u64);
            unsafe {
                libc::kill(child.as_raw(), libc::SIGKILL);
                libc::waitpid(child.as_raw(), std::ptr::null_mut(), 0);
            }
        }
    }
}

#[test]
fn test_freezer_v2_specified_controllers() {
    if !cgroups_rs::fs::hierarchies::is_cgroup2_unified_mode() {
        return;
    }

    // Regression test for issue #124: creating a v2 cgroup with only the
    // freezer controller specified must succeed, even though the kernel
    // does not list freezer in cgroup.controllers.
    let cg = Cgroup::new_with_specified_controllers(
        cgroups_rs::fs::hierarchies::auto(),
        String::from("test_freezer_v2_specified_controllers"),
        Some(vec![String::from("freezer")]),
    )
    .unwrap();

    with_sleeping_child(|pid| {
        freeze_and_thaw(&cg, pid);
    });
    cg.delete().unwrap();
}

#[test]
fn test_freezer_v1_freeze_thaw() {
    if cgroups_rs::fs::hierarchies::is_cgroup2_unified_mode() {
        return;
    }

    let cg = Cgroup::new(
        cgroups_rs::fs::hierarchies::auto(),
        String::from("test_freezer_v1_freeze_thaw"),
    )
    .unwrap();

    // Skip when the freezer subsystem is not mounted on this host.
    if cg.controller_of::<FreezerController>().is_none() {
        cg.delete().unwrap();
        return;
    }

    with_sleeping_child(|pid| {
        freeze_and_thaw(&cg, pid);
    });
    cg.delete().unwrap();
}
