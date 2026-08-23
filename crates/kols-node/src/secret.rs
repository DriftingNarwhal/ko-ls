//! Writing a secret to disk so that only this user can read it back.
//!
//! Every caller is writing something that either *is* an identity or opens one:
//! the seed, the MLS group blob, an epoch key, a DEK wrapping. `design/02` §6.3
//! says the seed is the only copy of a member and there is no recovery service,
//! so the file permissions are the whole of what stands between another account
//! on this machine and every network its holder belongs to.
//!
//! # Why this is a module rather than three lines in `store`
//!
//! It was three lines, and they were `#[cfg(unix)]` — so on Windows the chmod
//! was skipped and the file inherited whatever its directory granted, silently.
//! Silence is the part that matters: a seed under the wrong
//! ACL looks exactly like a seed under the right one, and nothing in the client
//! would ever have said otherwise.
//!
//! So the shape here is *restrict, then write, or refuse*:
//!
//! 1. Create the file empty, truncating any earlier copy.
//! 2. Restrict it to this user.
//! 3. Only then write the bytes.
//!
//! The ordering is deliberate. Restricting after writing leaves a window in
//! which the secret is on disk under the directory's permissions, and the
//! window is not the interesting part — a crash inside it leaves the file there
//! afterwards. Restricting an empty file costs the same and exposes nothing.
//!
//! If step 2 fails the file is removed and the write refuses, because a secret
//! written where somebody else can read it is worse than a secret not written:
//! the second is an error the user sees, and the first is one nobody ever does.

use std::fs;
use std::io;
use std::path::Path;

/// `ERROR_INVALID_FUNCTION`, which is what a filesystem with no notion of
/// Windows permissions returns when asked to set some.
#[cfg(windows)]
const ERROR_INVALID_FUNCTION: i32 = 1;

/// Explains a refusal, because the raw OS error does not.
///
/// "Incorrect function. (os error 1)" is what this looked like the first time
/// `kols.exe` ran, and it names neither the file, nor what was being attempted,
/// nor the one thing that would fix it. An error nobody can act on costs more
/// than the check that produced it saves.
fn unprotected(call: &str, path: &Path, err: &io::Error) -> io::Error {
    #[cfg(windows)]
    let hint = if err.raw_os_error() == Some(ERROR_INVALID_FUNCTION) {
        ". That error means this filesystem has no Windows permissions to set — a FAT or \
         exFAT drive, a network share, or a \\\\wsl$\\ path. A seed cannot be protected \
         there. Run from a local NTFS drive, or point KOLS_HOME at a directory on one"
    } else {
        ""
    };
    #[cfg(not(windows))]
    let hint = "";

    io::Error::new(
        err.kind(),
        format!(
            "could not restrict {} to your account, so the secret was not written — \
             {call} failed: {err}{hint}",
            path.display()
        ),
    )
}

/// Writes `bytes` to `path`, readable and writable by this user and nobody else.
///
/// Refuses rather than falling back to the platform's default permissions — see
/// the module documentation for why that trade only goes one way.
pub fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    fs::File::create(path)?;
    if let Err(err) = restrict_to_owner(path).map_err(|(call, err)| unprotected(call, path, &err)) {
        // Leaving an empty file behind would be harmless; leaving it and then
        // having a later write land in it would not, so it goes.
        let _ = fs::remove_file(path);
        return Err(err);
    }
    fs::write(path, bytes)
}

/// Restricts an existing file to the account running this process.
#[cfg(unix)]
fn restrict_to_owner(path: &Path) -> Result<(), (&'static str, io::Error)> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|err| ("chmod 0600", err))
}

/// Restricts an existing file to the account running this process.
///
/// # What this does, in Windows' terms
///
/// It replaces the file's DACL with a single entry granting this user full
/// access, and marks that DACL **protected** — which is the load-bearing half.
/// An unprotected DACL still inherits the entries its directory hands down, and
/// inheritance is exactly the defect this exists to fix: a seed in a user's
/// profile directory picks up whatever that directory grants, which on a shared
/// or administrator-configured machine is not necessarily one account.
///
/// # How this is verified
///
/// Cross-compilation proves the calls exist with the shapes assumed here and
/// proves nothing about the ACL that results, so this has been run rather than
/// only built: a seed written by `kols.exe` on NTFS shows this account and
/// nothing else in its Security tab, with no inherited `SYSTEM` or
/// `Administrators` entry — which is what the *protected* flag is for and the
/// one thing no build could demonstrate. The refusal path was confirmed the
/// same way, from a `\\wsl$\` path that has no permissions to set.
///
/// It stays outside `cargo test` because this module's tests are
/// `#[cfg(all(test, unix))]`
/// and this function compiles out of it entirely; CI checks the built artifact
/// on a Windows runner instead. A test that looks complete and is not is worse
/// than none.
#[cfg(windows)]
fn restrict_to_owner(path: &Path) -> Result<(), (&'static str, io::Error)> {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE, LocalFree};
    use windows_sys::Win32::Security::Authorization::{
        EXPLICIT_ACCESS_W, NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT, SET_ACCESS, SetEntriesInAclW,
        SetNamedSecurityInfoW, TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
    };
    use windows_sys::Win32::Security::{
        ACL, DACL_SECURITY_INFORMATION, GetTokenInformation, NO_INHERITANCE,
        PROTECTED_DACL_SECURITY_INFORMATION, TOKEN_QUERY, TOKEN_USER, TokenUser,
    };
    use windows_sys::Win32::Storage::FileSystem::FILE_ALL_ACCESS;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    /// Closes its handle however the function it guards leaves.
    struct Token(HANDLE);

    impl Drop for Token {
        fn drop(&mut self) {
            // SAFETY: `self.0` came from a successful `OpenProcessToken` and is
            // closed exactly once, here.
            unsafe { CloseHandle(self.0) };
        }
    }

    let mut raw = std::ptr::null_mut();
    // SAFETY: `GetCurrentProcess` returns a pseudo-handle that needs no closing,
    // and `raw` is a live pointer to a handle-sized slot for the call to fill.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) } == 0 {
        return Err(("OpenProcessToken", io::Error::last_os_error()));
    }
    let token = Token(raw);

    // The user's SID arrives inside a variable-length TOKEN_USER, so ask for the
    // length first. The buffer is u64-aligned rather than a Vec<u8>, because
    // TOKEN_USER holds a pointer and reading one out of an under-aligned
    // allocation is undefined however reliably it happens to work.
    let mut needed = 0u32;
    // SAFETY: a null buffer with zero length is the documented way to ask for
    // the size; this call is expected to fail and only `needed` is read.
    unsafe { GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut needed) };
    if needed == 0 {
        return Err(("GetTokenInformation", io::Error::last_os_error()));
    }
    let mut buffer = vec![0u64; (needed as usize).div_ceil(size_of::<u64>())];
    // SAFETY: `buffer` is at least `needed` bytes and outlives every read of the
    // SID inside it, which is what the trustee below borrows.
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    } == 0
    {
        return Err(("GetTokenInformation", io::Error::last_os_error()));
    }
    // SAFETY: the call above filled `buffer` with a TOKEN_USER, and the buffer is
    // aligned for one.
    let sid = unsafe { (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid };

    let access = EXPLICIT_ACCESS_W {
        grfAccessPermissions: FILE_ALL_ACCESS,
        grfAccessMode: SET_ACCESS,
        grfInheritance: NO_INHERITANCE,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: std::ptr::null_mut(),
            MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_USER,
            ptstrName: sid.cast(),
        },
    };

    let mut acl: *mut ACL = std::ptr::null_mut();
    // SAFETY: one entry is described by `access`, which lives across the call,
    // and `acl` receives an allocation this function frees below.
    let built = unsafe { SetEntriesInAclW(1, &access, std::ptr::null_mut(), &mut acl) };
    if built != ERROR_SUCCESS {
        return Err((
            "SetEntriesInAclW",
            io::Error::from_raw_os_error(built as i32),
        ));
    }

    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide.push(0);
    // SAFETY: `wide` is nul-terminated and outlives the call, and `acl` is the
    // ACL built immediately above.
    let applied = unsafe {
        SetNamedSecurityInfoW(
            wide.as_mut_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            acl,
            std::ptr::null_mut(),
        )
    };
    // SAFETY: `acl` was allocated by `SetEntriesInAclW` and is freed once.
    unsafe { LocalFree(acl.cast()) };

    if applied != ERROR_SUCCESS {
        return Err((
            "SetNamedSecurityInfoW",
            io::Error::from_raw_os_error(applied as i32),
        ));
    }
    Ok(())
}

/// Refuses, because this platform has no supported way to restrict a file.
///
/// Reachable only on a target that is neither Unix nor Windows. Refusing is the
/// same call the two implemented paths make when they fail: a secret this
/// process cannot protect is one it declines to write.
#[cfg(not(any(unix, windows)))]
fn restrict_to_owner(_path: &Path) -> Result<(), (&'static str, io::Error)> {
    Err((
        "restricting a file",
        io::Error::other("this platform has no supported way to restrict a file to its owner"),
    ))
}

#[cfg(all(test, unix))]
mod tests {
    use super::write_private;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    /// A directory that removes itself, however the test ends.
    ///
    /// Not a nicety: this dev container's storage is the host's, so a test that
    /// leaves its scratch behind spends somebody's disk every run. `Drop` runs on
    /// an unwind too, which a bare `remove_dir_all` at the end of a test does not.
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("kols-secret-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn file(&self) -> std::path::PathBuf {
            self.0.join("secret")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_secret_is_unreadable_to_anybody_else() {
        let scratch = Scratch::new("fresh");
        let path = scratch.file();
        write_private(&path, b"seed").unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "a fresh secret is readable beyond its owner");
        assert_eq!(fs::read(&path).unwrap(), b"seed");
    }

    #[test]
    fn overwriting_a_readable_file_leaves_it_unreadable() {
        // The case the ordering exists for. A secret written over something the
        // directory already made world-readable must not inherit that, and the
        // permissions of the file that was there say nothing about the one that
        // replaces it.
        let scratch = Scratch::new("overwrite");
        let path = scratch.file();
        fs::write(&path, b"public").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        write_private(&path, b"seed").unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "an overwritten secret kept the old mode");
        assert_eq!(fs::read(&path).unwrap(), b"seed");
    }

    #[test]
    fn a_refusal_says_which_file_and_which_call() {
        // "Incorrect function. (os error 1)" is what this said the first time
        // kols.exe ran, which named neither. A refusal nobody can act on costs
        // more than the check that produced it saves.
        let err = super::unprotected(
            "SetNamedSecurityInfoW",
            std::path::Path::new("/somewhere/seed"),
            &std::io::Error::from_raw_os_error(1),
        );
        let said = err.to_string();
        assert!(said.contains("/somewhere/seed"), "{said}");
        assert!(said.contains("SetNamedSecurityInfoW"), "{said}");
        assert!(said.contains("was not written"), "{said}");
    }

    #[test]
    fn a_secret_that_cannot_be_written_leaves_nothing_behind() {
        // Not the refusal path itself — that needs a filesystem this cannot
        // restrict on — but the property the refusal depends on: a failed write
        // must not leave a file a later reader would treat as a secret.
        let scratch = Scratch::new("missing");
        let path = scratch.file().join("no-such-directory").join("secret");
        assert!(write_private(&path, b"seed").is_err());
        assert!(!path.exists());
    }
}
