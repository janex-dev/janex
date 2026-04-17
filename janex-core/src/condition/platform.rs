// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

use crate::condition::java::normalize_arch_name;
use serde::Serialize;

/// Information about the current host platform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Platform {
    /// The operating system of the host machine.
    pub os: OperatingSystem,
    /// The CPU of the host machine.
    pub cpu: Cpu,
}

impl Platform {
    /// Creates a platform descriptor for CEL condition evaluation.
    pub fn new(os: OperatingSystem, cpu: Cpu) -> Self {
        Self { os, cpu }
    }

    /// Detects the current host platform using runtime system information.
    pub fn current() -> Self {
        Self {
            os: OperatingSystem::current(),
            cpu: Cpu::current(),
        }
    }
}

/// Information about an operating system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OperatingSystem {
    /// The normalized operating-system name.
    pub name: String,
    /// The parsed operating-system version.
    pub version: OperatingSystemVersion,
}

impl OperatingSystem {
    /// Creates an operating-system descriptor with a normalized name.
    pub fn new(name: impl Into<String>, version: OperatingSystemVersion) -> Self {
        let name = name.into();
        Self {
            name: normalize_os_name(&name).to_string(),
            version,
        }
    }

    /// Detects the current operating system and best-effort parses its version.
    pub fn current() -> Self {
        let info = os_info::get();
        Self {
            name: normalize_os_name(std::env::consts::OS).to_string(),
            version: OperatingSystemVersion::parse(info.version().to_string()),
        }
    }
}

/// The parsed version of an operating system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OperatingSystemVersion {
    /// The full, unparsed version string.
    pub full: String,
    /// The major version number.
    pub major: u64,
    /// The minor version number.
    pub minor: u64,
}

impl OperatingSystemVersion {
    /// Creates an operating-system version from explicit components.
    pub fn new(full: impl Into<String>, major: u64, minor: u64) -> Self {
        Self {
            full: full.into(),
            major,
            minor,
        }
    }

    /// Best-effort parses the first two numeric components of an operating-system version string.
    pub fn parse(full: impl Into<String>) -> Self {
        let full = full.into();
        let mut numbers = full
            .split(|ch: char| !ch.is_ascii_digit())
            .filter(|part| !part.is_empty())
            .filter_map(|part| part.parse::<u64>().ok())
            .collect::<Vec<_>>()
            .into_iter();
        Self {
            full,
            major: numbers.next().unwrap_or(0),
            minor: numbers.next().unwrap_or(0),
        }
    }
}

/// Information about the host CPU.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Cpu {
    /// The normalized CPU architecture name.
    pub arch: String,
}

impl Cpu {
    /// Creates a CPU descriptor with a normalized architecture name.
    pub fn new(arch: impl Into<String>) -> Self {
        let arch = arch.into();
        Self {
            arch: normalize_arch_name(&arch).to_string(),
        }
    }

    /// Detects the current host CPU architecture.
    pub fn current() -> Self {
        let info = os_info::get();
        if let Some(arch) = info.architecture() {
            // Use the native host architecture when the OS can report it.
            // This lets a 32-bit launcher still recognize x86-64 or arm64 hosts.
            return Self::new(arch);
        }

        // Fall back to the compile-time target only on platforms where runtime
        // architecture detection is unavailable.
        Self::new(std::env::consts::ARCH)
    }
}

/// Normalizes operating-system spellings to the names described by `FileFormat.md`.
pub(crate) fn normalize_os_name(name: &str) -> &str {
    match name {
        "darwin" | "macos" => "macos",
        other => other,
    }
}
