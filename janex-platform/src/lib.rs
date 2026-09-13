// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Native architecture discovery independent of the launcher's instruction set.

use std::io;

/// Normalizes common operating-system and Java CPU architecture names.
/// Unknown names are retained unchanged.
pub fn normalize_architecture(name: &str) -> &str {
    match name {
        "amd64" | "AMD64" | "x86_64" | "x64" => "x86-64",
        "i386" | "i486" | "i586" | "i686" | "x86" => "x86",
        "arm64" | "ARM64" | "aarch64" => "aarch64",
        other => other,
    }
}

/// Returns the operating system's native architecture for runtime preference.
/// This is not the architecture to use when selecting resources for an emulated JVM.
pub fn native_architecture() -> io::Result<String> {
    #[cfg(windows)]
    {
        windows_architecture()
    }
    #[cfg(unix)]
    {
        use std::process::Command;
        #[cfg(target_os = "macos")]
        if let Ok(output) = Command::new("/usr/sbin/sysctl")
            .args(["-n", "hw.optional.arm64"])
            .output()
            && output.status.success()
            && output.stdout.trim_ascii() == b"1"
        {
            return Ok("aarch64".into());
        }
        let output = Command::new("/usr/bin/uname").arg("-m").output()?;
        if !output.status.success() {
            return Err(io::Error::other("cannot determine native architecture"));
        }
        let name = std::str::from_utf8(&output.stdout)
            .map_err(io::Error::other)?
            .trim();
        if name.is_empty() {
            return Err(io::Error::other("empty native architecture"));
        }
        Ok(normalize_architecture(name).into())
    }
    #[cfg(not(any(windows, unix)))]
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "native architecture discovery is unsupported",
        ))
    }
}

/// Queries the native machine even when this process runs under WOW64 or ARM emulation.
#[cfg(windows)]
#[allow(unsafe_code)]
fn windows_architecture() -> io::Result<String> {
    use windows_sys::Win32::System::{
        LibraryLoader::{GetModuleHandleW, GetProcAddress},
        SystemInformation::{GetNativeSystemInfo, SYSTEM_INFO},
        Threading::GetCurrentProcess,
    };
    // SAFETY: The DLL name is NUL-terminated and kernel32 remains loaded for this process.
    let module =
        unsafe { GetModuleHandleW("kernel32.dll\0".encode_utf16().collect::<Vec<_>>().as_ptr()) };
    if module.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: The export name is NUL-terminated; the returned address is checked before calling.
    let function = unsafe { GetProcAddress(module, c"IsWow64Process2".as_ptr().cast()) };
    if let Some(function) = function {
        // SAFETY: This is the documented ABI of the named kernel32 export.
        let query: unsafe extern "system" fn(*mut core::ffi::c_void, *mut u16, *mut u16) -> i32 =
            unsafe { std::mem::transmute(function) };
        let (mut process, mut native) = (0, 0);
        // SAFETY: The pseudo-handle is valid and both output pointers reference live u16 values.
        if unsafe { query(GetCurrentProcess(), &mut process, &mut native) } == 0 {
            return Err(io::Error::last_os_error());
        }
        return machine_architecture(native).map(str::to_owned);
    }
    // IsWow64Process2 is absent on older Windows, before Windows on ARM was introduced.
    let mut info = SYSTEM_INFO::default();
    // SAFETY: The API initializes this correctly sized SYSTEM_INFO; the architecture field is valid.
    let machine = unsafe {
        GetNativeSystemInfo(&mut info);
        match info.Anonymous.Anonymous.wProcessorArchitecture {
            0 => 0x014c,
            9 => 0x8664,
            12 => 0xaa64,
            _ => 0,
        }
    };
    machine_architecture(machine).map(str::to_owned)
}

/// Converts PE machine identifiers without depending on the process pointer width.
#[cfg(any(windows, test))]
fn machine_architecture(machine: u16) -> io::Result<&'static str> {
    match machine {
        0x014c => Ok("x86"),
        0x8664 => Ok("x86-64"),
        0xaa64 => Ok("aarch64"),
        _ => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "unsupported native Windows architecture",
        )),
    }
}

#[cfg(test)]
mod tests {
    //! Native architecture mappings and discovery.
    use super::*;

    #[test]
    fn machine_types_do_not_depend_on_the_build_target() {
        assert_eq!(machine_architecture(0x014c).unwrap(), "x86");
        assert_eq!(machine_architecture(0x8664).unwrap(), "x86-64");
        assert_eq!(machine_architecture(0xaa64).unwrap(), "aarch64");
        assert!(machine_architecture(0).is_err());
        assert!(!native_architecture().unwrap().is_empty());
    }
}
