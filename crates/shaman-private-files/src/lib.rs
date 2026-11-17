//! Files and directories kept to the user this process runs as, and files
//! replaced whole or not at all. Whatever keeps records another user on the
//! machine is not to read — a Server's runtime files, its Logs, the records
//! of its Pairings — writes them through here.

use std::{fs, io, path::Path};

use anyhow::{Context, Result};

/// Replaces the file at `path` with `contents`, whole or not at all, and for
/// good: written beside it, protected to the current user before anything is
/// written, flushed to the disk, then renamed over it, the rename itself on
/// the disk before this returns. So an interrupted write leaves what it held
/// before rather than part of either, and a crash or power loss once this
/// has returned leaves the replacement. Where it fails having renamed the
/// replacement over the file, the replacement is in place, but may not
/// outlast a crash.
pub fn replace_private_file(path: &Path, contents: &[u8]) -> Result<()> {
    use std::io::Write as _;

    let directory = path
        .parent()
        .with_context(|| format!("{path:?} lies in no directory"))?;
    let mut replacement = tempfile::Builder::new()
        .prefix(".replacing-")
        .suffix(".tmp")
        .tempfile_in(directory)
        .with_context(|| format!("create a replacement beside {path:?}"))?;
    protect_current_user_file(replacement.path())?;
    replacement
        .as_file_mut()
        .write_all(contents)
        .with_context(|| format!("write a replacement for {path:?}"))?;
    replacement
        .as_file()
        .sync_all()
        .with_context(|| format!("flush a replacement for {path:?}"))?;
    rename_for_good(replacement, path, directory)?;
    protect_current_user_file(path)
}

/// Renames `replacement` over `path`, in `directory`, and has the rename on
/// the disk before returning.
///
/// A rename is an entry in its directory, which is on the disk once the
/// directory is flushed. A filesystem that cannot flush a directory at all,
/// as some network and FUSE filesystems cannot, leaves the rename to its own
/// guarantees, as PostgreSQL's durable rename does.
#[cfg(unix)]
fn rename_for_good(
    replacement: tempfile::NamedTempFile,
    path: &Path,
    directory: &Path,
) -> Result<()> {
    replacement
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("replace {path:?}"))?;
    let flushed = fs::File::open(directory).and_then(|directory| directory.sync_all());
    match flushed {
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::InvalidInput | io::ErrorKind::Unsupported
            ) =>
        {
            Ok(())
        }
        flushed => {
            flushed.with_context(|| format!("flush the replacement of {path:?} to the disk"))
        }
    }
}

/// Renames `replacement` over `path` and has the rename on the disk before
/// returning.
///
/// Windows gives no way to flush a directory. `MoveFileExW` with
/// `MOVEFILE_WRITE_THROUGH` instead does not return until the move is on the
/// disk.
#[cfg(windows)]
fn rename_for_good(
    replacement: tempfile::NamedTempFile,
    path: &Path,
    _directory: &Path,
) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    // Kept, as persisting it would: no longer marked temporary, nor taken
    // away once dropped.
    let kept = replacement
        .into_temp_path()
        .keep()
        .map_err(|error| error.error)
        .with_context(|| format!("keep a replacement for {path:?}"))?;
    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>()
    };
    let (from, to) = (wide(&kept), wide(path));
    // SAFETY: both paths are NUL-terminated and outlive the call.
    let moved = unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        let error = io::Error::last_os_error();
        let _ = fs::remove_file(&kept);
        return Err(error).with_context(|| format!("replace {path:?}"));
    }
    Ok(())
}

#[cfg(unix)]
pub fn protect_current_user_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("protect runtime directory {path:?}"))
}

#[cfg(unix)]
pub fn protect_current_user_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("protect runtime file {path:?}"))
}

#[cfg(windows)]
pub fn protect_current_user_directory(path: &Path) -> Result<()> {
    protect_windows_owner(path, true)
}

#[cfg(windows)]
pub fn protect_current_user_file(path: &Path) -> Result<()> {
    protect_windows_owner(path, false)
}

#[cfg(windows)]
fn protect_windows_owner(path: &Path, directory: bool) -> Result<()> {
    use std::{os::windows::ffi::OsStrExt, ptr};

    use anyhow::anyhow;
    use windows_sys::Win32::{
        Foundation::{ERROR_SUCCESS, LocalFree},
        Security::{
            ACL,
            Authorization::{
                ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
                SE_FILE_OBJECT, SetNamedSecurityInfoW,
            },
            DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl,
            PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
        },
    };

    struct LocalSecurityDescriptor(PSECURITY_DESCRIPTOR);

    impl Drop for LocalSecurityDescriptor {
        fn drop(&mut self) {
            // SAFETY: the descriptor was allocated by
            // ConvertStringSecurityDescriptorToSecurityDescriptorW.
            unsafe {
                LocalFree(self.0);
            }
        }
    }

    let current_user_sid = current_windows_user_sid()?;
    let sddl = if directory {
        format!("D:P(A;OICI;FA;;;{current_user_sid})")
    } else {
        format!("D:P(A;;FA;;;{current_user_sid})")
    };
    let sddl = sddl.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let mut descriptor = ptr::null_mut();
    // SAFETY: sddl is NUL-terminated and descriptor points to writable storage.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("build current-user ACL for {path:?}"));
    }
    let descriptor = LocalSecurityDescriptor(descriptor);

    let mut dacl_present = 0;
    let mut dacl_defaulted = 0;
    let mut dacl: *mut ACL = ptr::null_mut();
    // SAFETY: descriptor remains alive and all out-pointers reference initialized storage.
    if unsafe {
        GetSecurityDescriptorDacl(
            descriptor.0,
            &mut dacl_present,
            &mut dacl,
            &mut dacl_defaulted,
        )
    } == 0
        || dacl_present == 0
        || dacl.is_null()
    {
        return Err(anyhow!("current-user security descriptor has no DACL"));
    }

    let path_utf16 = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    // SAFETY: path is NUL-terminated, dacl is owned by the live descriptor, and null owner,
    // group, and SACL pointers are permitted when only DACL flags are requested.
    let status = unsafe {
        SetNamedSecurityInfoW(
            path_utf16.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            dacl,
            ptr::null_mut(),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(std::io::Error::from_raw_os_error(status as i32))
            .with_context(|| format!("protect runtime path {path:?}"));
    }
    Ok(())
}

/// The user this process runs as, as its token names it: the `TOKEN_USER` the
/// token holds, kept in a buffer of words so it is aligned as it must be.
#[cfg(windows)]
pub struct CurrentWindowsUser(Vec<usize>);

#[cfg(windows)]
impl CurrentWindowsUser {
    pub fn read() -> Result<Self> {
        use std::{mem, ptr};

        use windows_sys::Win32::{
            Foundation::{CloseHandle, HANDLE},
            Security::{GetTokenInformation, TOKEN_QUERY, TokenUser},
            System::Threading::{GetCurrentProcess, OpenProcessToken},
        };

        struct OwnedHandle(HANDLE);

        impl Drop for OwnedHandle {
            fn drop(&mut self) {
                // SAFETY: the handle was returned by OpenProcessToken and is owned by this guard.
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }

        let mut token = ptr::null_mut();
        // SAFETY: GetCurrentProcess returns a valid pseudo-handle and token points to writable
        // storage.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(std::io::Error::last_os_error()).context("open current process token");
        }
        let token = OwnedHandle(token);

        let mut required_bytes = 0;
        // SAFETY: a null buffer with length zero is the documented size-query operation.
        unsafe {
            GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut required_bytes);
        }
        if required_bytes == 0 {
            return Err(std::io::Error::last_os_error()).context("size current user token data");
        }

        let word_count = (required_bytes as usize).div_ceil(mem::size_of::<usize>());
        let mut token_data = vec![0usize; word_count];
        // SAFETY: token_data is aligned, writable, and at least required_bytes long.
        if unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                token_data.as_mut_ptr().cast(),
                required_bytes,
                &mut required_bytes,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error()).context("read current user token data");
        }
        Ok(Self(token_data))
    }

    /// The user's SID, which lies within this reading and is valid only as long as it is.
    pub fn sid(&self) -> windows_sys::Win32::Security::PSID {
        use windows_sys::Win32::Security::TOKEN_USER;

        // SAFETY: read initialized the buffer with a TOKEN_USER value.
        unsafe { (*self.0.as_ptr().cast::<TOKEN_USER>()).User.Sid }
    }
}

#[cfg(windows)]
fn current_windows_user_sid() -> Result<String> {
    use std::{ptr, slice};

    use windows_sys::Win32::{
        Foundation::LocalFree, Security::Authorization::ConvertSidToStringSidW,
    };

    let user = CurrentWindowsUser::read()?;
    let mut sid_string = ptr::null_mut();
    // SAFETY: user holds a valid SID and sid_string points to writable storage.
    if unsafe { ConvertSidToStringSidW(user.sid(), &mut sid_string) } == 0 {
        return Err(std::io::Error::last_os_error()).context("format current user SID");
    }
    struct LocalSidString(*mut u16);
    impl Drop for LocalSidString {
        fn drop(&mut self) {
            // SAFETY: the string was allocated by ConvertSidToStringSidW.
            unsafe {
                LocalFree(self.0.cast());
            }
        }
    }
    let sid_string = LocalSidString(sid_string);
    // SAFETY: ConvertSidToStringSidW returns a valid NUL-terminated UTF-16 string.
    let length = unsafe {
        let mut length = 0;
        while *sid_string.0.add(length) != 0 {
            length += 1;
        }
        length
    };
    // SAFETY: length was measured within the NUL-terminated allocation.
    String::from_utf16(unsafe { slice::from_raw_parts(sid_string.0, length) })
        .context("decode current user SID")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_private_file_is_replaced_whole_and_kept_to_its_owner() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("records.json");
        replace_private_file(&path, b"first").unwrap();
        replace_private_file(&path, b"second").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second");
        assert_eq!(
            fs::read_dir(directory.path()).unwrap().count(),
            1,
            "nothing is left beside it"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn a_replacement_that_cannot_land_leaves_nothing_beside_its_place() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("records.json");
        fs::create_dir(&path).unwrap();
        assert!(replace_private_file(&path, b"lost").is_err());
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
        assert!(path.is_dir());
    }
}
