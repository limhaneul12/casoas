//! Binary identity for the native CLI configuration exercised by the denial canary.
use std::path::Path;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use std::path::PathBuf;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct VerifiedBinary {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    path: PathBuf,
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    stamp: FileStamp,
}

impl VerifiedBinary {
    pub(super) fn still_matches(&self) -> bool {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            std::fs::metadata(&self.path)
                .is_ok_and(|metadata| FileStamp::from(&metadata) == self.stamp)
        }
        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        false
    }
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[derive(Clone, PartialEq, Eq)]
struct FileStamp {
    dev: u64,
    inode: u64,
    size: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
impl From<&std::fs::Metadata> for FileStamp {
    fn from(metadata: &std::fs::Metadata) -> Self {
        use std::os::unix::fs::MetadataExt;
        Self {
            dev: metadata.dev(),
            inode: metadata.ino(),
            size: metadata.len(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        }
    }
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn verify(path: PathBuf) -> Option<VerifiedBinary> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    use std::sync::Mutex;

    const TESTED_SIZE: u64 = 201_432_752;
    const TESTED_SHA256: &str = "e61679b8ba54abae9a56534208e21f65e1dede6e4eb0144dbd64535f42d07724";
    static CACHE: Mutex<Option<(VerifiedBinary, bool)>> = Mutex::new(None);
    let path = path.canonicalize().ok()?;
    let metadata = std::fs::metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() != TESTED_SIZE {
        return None;
    }
    let identity = VerifiedBinary {
        path,
        stamp: FileStamp::from(&metadata),
    };
    let mut cache = CACHE.lock().ok()?;
    if let Some((cached, accepted)) = cache.as_ref()
        && cached == &identity
    {
        return accepted.then_some(identity);
    }

    let file = std::fs::File::open(&identity.path).ok()?;
    let opened = file.metadata().ok()?;
    if !opened.is_file() || FileStamp::from(&opened) != identity.stamp {
        return None;
    }
    let mut reader = std::io::BufReader::new(&file).take(TESTED_SIZE + 1);
    let mut digest = Sha256::new();
    let mut chunk = vec![0_u8; 64 * 1024];
    let mut bytes = 0_u64;
    loop {
        let read = reader.read(&mut chunk).ok()?;
        if read == 0 {
            break;
        }
        bytes += read as u64;
        digest.update(&chunk[..read]);
    }
    let accepted = bytes == TESTED_SIZE
        && identity.still_matches()
        && format!("{:x}", digest.finalize()) == TESTED_SHA256;
    *cache = Some((identity.clone(), accepted));
    accepted.then_some(identity)
}

pub(super) async fn verified(path: &Path) -> Option<VerifiedBinary> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        static HASH_JOB: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        let _permit = HASH_JOB.lock().await;
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || verify(path))
            .await
            .ok()
            .flatten()
    }
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        let _ = path;
        None
    }
}
