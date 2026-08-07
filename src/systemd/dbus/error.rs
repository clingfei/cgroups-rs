// Copyright (c) 2025 Ant Group
//
// SPDX-License-Identifier: Apache-2.0
//

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid properties")]
    InvalidProperties,

    #[error("systemd job failed with result: {0}")]
    JobFailed(String),

    #[error("systemd JobRemoved signal stream closed")]
    JobRemovedSignalStreamClosed,

    #[error("dbus error: {0}")]
    Dbus(#[from] zbus::Error),
}
