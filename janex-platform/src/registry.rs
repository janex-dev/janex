// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Unicode registry access without invoking a shell or parsing localized command output.

#![allow(unsafe_code)]

use std::{io, ptr};
use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
    System::Registry::*,
};

/// Owns an opened registry key.
struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: Each Key owns one successful open/create result.
        unsafe {
            RegCloseKey(self.0);
        }
    }
}

/// Encodes a NUL-terminated Windows string, rejecting embedded terminators.
fn wide(value: &str) -> io::Result<Vec<u16>> {
    if value.contains('\0') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "NUL in registry string",
        ));
    }
    Ok(value.encode_utf16().chain(Some(0)).collect())
}

/// Selects the machine or user registry hive.
fn hive(system: bool) -> HKEY {
    if system {
        HKEY_LOCAL_MACHINE
    } else {
        HKEY_CURRENT_USER
    }
}

/// Converts a registry API result code without consulting thread-local last-error state.
fn check(code: u32) -> io::Result<()> {
    if code == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(code as i32))
    }
}

/// Opens an existing key in the shared 64-bit registry view.
fn open(system: bool, path: &str, access: u32) -> io::Result<Option<Key>> {
    let path = wide(path)?;
    let mut key = ptr::null_mut();
    // SAFETY: Strings are terminated, the hive is predefined, and the output is valid.
    let code = unsafe {
        RegOpenKeyExW(
            hive(system),
            path.as_ptr(),
            0,
            access | KEY_WOW64_64KEY,
            &mut key,
        )
    };
    if code == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    check(code)?;
    Ok(Some(Key(key)))
}

/// Reads a REG_SZ value; missing keys or values return None and other value types are errors.
pub fn get(system: bool, path: &str, name: &str) -> io::Result<Option<String>> {
    let Some(key) = open(system, path, KEY_QUERY_VALUE)? else {
        return Ok(None);
    };
    let name = wide(name)?;
    let (mut kind, mut length) = (0, 0);
    // SAFETY: A null data pointer requests the required byte count.
    let code = unsafe {
        RegQueryValueExW(
            key.0,
            name.as_ptr(),
            ptr::null(),
            &mut kind,
            ptr::null_mut(),
            &mut length,
        )
    };
    if code == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    check(code)?;
    if kind != REG_SZ || length % 2 != 0 {
        return Err(io::Error::other("unexpected registry value type"));
    }
    let mut data = vec![0u16; length as usize / 2];
    // SAFETY: The writable buffer has the requested byte capacity.
    check(unsafe {
        RegQueryValueExW(
            key.0,
            name.as_ptr(),
            ptr::null(),
            &mut kind,
            data.as_mut_ptr().cast(),
            &mut length,
        )
    })?;
    data.truncate(length as usize / 2);
    if data.last() == Some(&0) {
        data.pop();
    }
    String::from_utf16(&data)
        .map(Some)
        .map_err(io::Error::other)
}

/// Creates a key if necessary and writes a REG_SZ value.
pub fn set(system: bool, path: &str, name: &str, value: &str) -> io::Result<()> {
    let (path, name, value) = (wide(path)?, wide(name)?, wide(value)?);
    let mut key = ptr::null_mut();
    // SAFETY: Inputs are terminated and both optional outputs and security attributes may be null.
    check(unsafe {
        RegCreateKeyExW(
            hive(system),
            path.as_ptr(),
            0,
            ptr::null(),
            0,
            KEY_SET_VALUE | KEY_WOW64_64KEY,
            ptr::null(),
            &mut key,
            ptr::null_mut(),
        )
    })?;
    let key = Key(key);
    // SAFETY: Data points to the complete UTF-16 string, including its terminator.
    check(unsafe {
        RegSetValueExW(
            key.0,
            name.as_ptr(),
            0,
            REG_SZ,
            value.as_ptr().cast(),
            (value.len() * 2) as u32,
        )
    })
}

/// Removes one value, tolerating absence, and prunes empty keys below Software\Classes.
pub fn remove(system: bool, path: &str, name: &str) -> io::Result<()> {
    if let Some(key) = open(system, path, KEY_SET_VALUE)? {
        let name = wide(name)?;
        // SAFETY: The opened key and terminated value name remain live.
        let code = unsafe { RegDeleteValueW(key.0, name.as_ptr()) };
        if code != ERROR_FILE_NOT_FOUND {
            check(code)?;
        }
    }
    let mut current = path;
    while current.starts_with("Software\\Classes\\") {
        let Some(key) = open(system, current, KEY_QUERY_VALUE | KEY_ENUMERATE_SUB_KEYS)? else {
            break;
        };
        let (mut children, mut values) = (0, 0);
        // SAFETY: Only the requested counts are non-null; all other outputs are optional.
        check(unsafe {
            RegQueryInfoKeyW(
                key.0,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
                &mut children,
                ptr::null_mut(),
                ptr::null_mut(),
                &mut values,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        })?;
        if children != 0 || values != 0 {
            break;
        }
        drop(key);
        let encoded = wide(current)?;
        // SAFETY: Only a verified empty key is removed; deletion is nonrecursive.
        check(unsafe { RegDeleteKeyExW(hive(system), encoded.as_ptr(), KEY_WOW64_64KEY, 0) })?;
        current = current.rsplit_once('\\').expect("nested registry path").0;
    }
    Ok(())
}

/// Notifies Explorer that registered associations may have changed.
pub fn notify() {
    use windows_sys::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
    // SAFETY: Association-change notifications carry no item pointers.
    unsafe {
        SHChangeNotify(
            SHCNE_ASSOCCHANGED as i32,
            SHCNF_IDLIST,
            ptr::null(),
            ptr::null(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_values_round_trip_and_unrelated_values_survive_removal() {
        let key = format!(
            "Software\\Classes\\Janex.Test.{}.{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let child = format!("{key}\\shell\\open\\command");
        assert_eq!(get(false, &child, "").unwrap(), None);
        set(
            false,
            &child,
            "",
            "\"C:\\Apps with spaces\\测试😀\\janex.exe\" open -- \"%1\"",
        )
        .unwrap();
        set(false, &child, "Other", "keep").unwrap();
        assert!(get(false, &child, "").unwrap().unwrap().contains("测试😀"));
        remove(false, &child, "").unwrap();
        assert_eq!(
            get(false, &child, "Other").unwrap().as_deref(),
            Some("keep")
        );
        remove(false, &child, "Other").unwrap();
        assert!(open(false, &key, KEY_QUERY_VALUE).unwrap().is_none());
        remove(false, &child, "Other").unwrap();
    }
}
