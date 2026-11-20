use super::deep_links::{validate, MAX_DEEP_LINK_BYTES};
use std::{
    fs::{self, DirBuilder, File, OpenOptions, TryLockError},
    io,
    os::unix::{
        fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
        net::UnixDatagram,
    },
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

pub(super) struct Instance {
    socket: UnixDatagram,
    path: PathBuf,
    _lock: File,
}

impl Instance {
    /// None means the primary accepted the activation datagram.
    pub(super) fn acquire(link: Option<&str>) -> io::Result<Option<Self>> {
        // SAFETY: geteuid has no arguments and reads the process credentials.
        let uid = unsafe { libc::geteuid() };
        let root = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        Self::acquire_in(&root.join(format!("stonks-gpui-{uid}")), uid, link)
    }

    fn acquire_in(dir: &Path, uid: u32, link: Option<&str>) -> io::Result<Option<Self>> {
        match DirBuilder::new().mode(0o700).create(dir) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        let metadata = fs::symlink_metadata(dir)?;
        if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o077 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "instance directory is not private",
            ));
        }
        let lock = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(dir.join("lock"))?;
        let path = dir.join("activate");
        let sender = UnixDatagram::unbound()?;
        sender.set_nonblocking(true)?;
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match lock.try_lock() {
                Ok(()) => {
                    match fs::remove_file(&path) {
                        Ok(()) => {}
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                        Err(error) => return Err(error),
                    }
                    let socket = UnixDatagram::bind(&path)?;
                    socket.set_nonblocking(true)?;
                    return Ok(Some(Self {
                        socket,
                        path,
                        _lock: lock,
                    }));
                }
                Err(TryLockError::WouldBlock) => {}
                Err(TryLockError::Error(error)) => return Err(error),
            }
            match sender.send_to(link.unwrap_or("").as_bytes(), &path) {
                Ok(_) => return Ok(None),
                Err(error)
                    if Instant::now() < deadline
                        && matches!(
                            error.kind(),
                            io::ErrorKind::WouldBlock
                                | io::ErrorKind::NotFound
                                | io::ErrorKind::ConnectionRefused
                        ) =>
                {
                    // Another process may hold the lock before bind, or while exiting.
                    thread::sleep(Duration::from_millis(20));
                }
                Err(error) => return Err(error),
            }
        }
    }

    pub(super) fn drain(&self, mut accept: impl FnMut(Option<super::DeepLink>)) {
        let mut buffer = [0; MAX_DEEP_LINK_BYTES + 1];
        for _ in 0..16 {
            match self.socket.recv(&mut buffer) {
                Ok(0) => accept(None),
                Ok(len) => {
                    if let Some(link) = std::str::from_utf8(&buffer[..len]).ok().and_then(validate)
                    {
                        accept(Some(link));
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => {
                    eprintln!("Stonks activation receive failed: {}", error.kind());
                    break;
                }
            }
        }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        // Unlink while holding the lock so a replacement cannot lose its socket.
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::DeepLink;

    #[test]
    fn secondary_launches_forward_and_primary_can_restart() {
        let temp = tempfile::tempdir().unwrap();
        let uid = temp.path().metadata().unwrap().uid();
        let dir = temp.path().join("instance");
        let primary = Instance::acquire_in(&dir, uid, None).unwrap().unwrap();
        assert!(Instance::acquire_in(&dir, uid, None).unwrap().is_none());
        assert!(
            Instance::acquire_in(&dir, uid, Some("stonks://invite/join-me"))
                .unwrap()
                .is_none()
        );
        let sender = UnixDatagram::unbound().unwrap();
        sender.send_to(b"https://invalid", &primary.path).unwrap();
        sender
            .send_to(&[b'x'; MAX_DEEP_LINK_BYTES + 10], &primary.path)
            .unwrap();
        let mut received = Vec::new();
        primary.drain(|link| received.push(link));
        assert_eq!(
            received,
            [
                None,
                Some(DeepLink::Invite {
                    code: "join-me".into()
                })
            ]
        );
        drop(primary);
        // A crash can leave a socket pathname behind without a lock holder.
        let stale = UnixDatagram::bind(dir.join("activate")).unwrap();
        drop(stale);
        assert!(Instance::acquire_in(&dir, uid, None).unwrap().is_some());
    }
}
