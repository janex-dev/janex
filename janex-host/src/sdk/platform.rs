// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Target platforms independent of the running Janex executable.

use crate::{Result, error::invalid};
use serde::{Deserialize, Serialize};

/// The operating system, CPU architecture and C library targeted by an SDK archive.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SdkPlatform {
    /// `windows`, `linux`, `macos`, or `freebsd`.
    pub os: String,
    /// Normalized architecture, such as `x86-64` or `aarch64`.
    pub arch: String,
    /// `glibc` or `musl` on Linux, otherwise `native`.
    pub libc: String,
}

impl SdkPlatform {
    /// Returns the native operating-system platform, including when Janex runs under emulation.
    pub fn native() -> Result<Self> {
        Ok(Self {
            os: std::env::consts::OS.into(),
            arch: janex_platform::native_architecture()?,
            libc: Self::default_libc(std::env::consts::OS).into(),
        })
    }

    /// Chooses the host Linux libc, or the conventional libc for another target OS.
    pub fn default_libc(os: &str) -> &'static str {
        if os == "linux" {
            if cfg!(target_os = "linux") && std::path::Path::new("/etc/alpine-release").exists() {
                "musl"
            } else {
                "glibc"
            }
        } else {
            "native"
        }
    }

    /// Validates supported platform components independently of provider availability.
    pub fn validate(&self) -> Result<()> {
        if !matches!(self.os.as_str(), "windows" | "linux" | "macos" | "freebsd")
            || !matches!(
                self.arch.as_str(),
                "x86" | "x86-64" | "aarch64" | "arm" | "ppc64le" | "s390x" | "riscv64"
            )
            || if self.os == "linux" {
                !matches!(self.libc.as_str(), "glibc" | "musl")
            } else {
                self.libc != "native"
            }
        {
            return Err(invalid("invalid SDK target platform"));
        }
        Ok(())
    }

    /// Returns an unambiguous platform key for defaults and display.
    pub fn key(&self) -> String {
        if self.os == "linux" {
            format!("{}-{}-{}", self.os, self.arch, self.libc)
        } else {
            format!("{}-{}", self.os, self.arch)
        }
    }

    /// Returns a Java tool filename for this target, without inspecting the host OS.
    pub(super) fn executable(&self, name: &str) -> String {
        if self.os == "windows" {
            format!("{name}.exe")
        } else {
            name.into()
        }
    }
}
