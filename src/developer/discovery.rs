//! Per-user discovery records contain no authorization capability.
//! Windows uses the profile directory's inherited ACL, not same-user isolation.
use super::*;
use std::io::{self, Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub(super) struct Publication {
    path: PathBuf,
    bytes: Vec<u8>,
    identity: same_file::Handle,
}
impl Publication {
    pub(super) fn publish(home: &Path, instance: &str, port: u16) -> io::Result<Self> {
        if !valid_hex_id(instance) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        // The profile root is trusted. Resolve platform aliases such as macOS
        // /var before checking the application-controlled child paths.
        let mut directory = home.canonicalize()?;
        for component in [".mangameeya", "developer"] {
            directory.push(component);
            reject_symlink(&directory)?;
            if !directory.exists() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt;
                    let mut builder = fs::DirBuilder::new();
                    builder.mode(0o700).create(&directory)?;
                }
                #[cfg(not(unix))]
                fs::create_dir(&directory)?;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
            }
        }
        let path = directory.join(format!("{instance}.json"));
        reject_symlink(&path)?;
        let bytes = serde_json::to_vec(&serde_json::json!({
            "v":1, "protocol_version":1, "instance_id":instance,
            "port":port, "app_version":env!("CARGO_PKG_VERSION")
        }))?;
        let mut temp = tempfile::NamedTempFile::new_in(&directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            temp.as_file()
                .set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        temp.write_all(&bytes)?;
        temp.as_file().sync_all()?;
        let file = temp.persist_noclobber(&path).map_err(|e| e.error)?;
        let identity = match same_file::Handle::from_file(file) {
            Ok(identity) => identity,
            Err(e) => {
                let _ = fs::remove_file(&path);
                return Err(e);
            }
        };
        Ok(Self {
            path,
            bytes,
            identity,
        })
    }
}
fn reject_symlink(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(io::ErrorKind::PermissionDenied.into()),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}
impl Drop for Publication {
    fn drop(&mut self) {
        if !self.path.ancestors().all(|p| reject_symlink(p).is_ok()) {
            return;
        }
        let Ok(metadata) = fs::symlink_metadata(&self.path) else {
            return;
        };
        if !metadata.is_file() || metadata.len() != self.bytes.len() as u64 {
            return;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.dev() != self.identity.dev() || metadata.ino() != self.identity.ino() {
                return;
            }
        }
        #[cfg(not(unix))]
        if !same_file::Handle::from_path(&self.path).is_ok_and(|identity| identity == self.identity)
        {
            return;
        }
        let Ok(mut file) = self.identity.as_file().try_clone() else {
            return;
        };
        if file.seek(SeekFrom::Start(0)).is_err() {
            return;
        }
        let mut bytes = Vec::new();
        if file
            .take(self.bytes.len() as u64 + 1)
            .read_to_end(&mut bytes)
            .is_ok()
            && bytes == self.bytes
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn publication_is_minimal_private_and_removed() {
        let home = tempfile::tempdir().unwrap();
        let id = "a".repeat(64);
        let record = Publication::publish(home.path(), &id, 1234).unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(&record.path).unwrap()).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 5);
        assert_eq!(value["instance_id"], id);
        assert_eq!(value["port"], 1234);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&record.path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(record.path.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        let path = record.path.clone();
        drop(record);
        assert!(!path.exists());
        let record = Publication::publish(home.path(), &id, 1234).unwrap();
        fs::write(&record.path, b"replacement").unwrap();
        drop(record);
        assert_eq!(fs::read(path).unwrap(), b"replacement");
    }
    #[test]
    fn replaced_records_are_not_removed() {
        let home = tempfile::tempdir().unwrap();
        let record = Publication::publish(home.path(), &"a".repeat(64), 1234).unwrap();
        let path = record.path.clone();
        let replacement = tempfile::NamedTempFile::new_in(path.parent().unwrap()).unwrap();
        fs::write(replacement.path(), &record.bytes).unwrap();
        fs::remove_file(&path).unwrap();
        replacement.persist_noclobber(&path).unwrap();
        drop(record);
        assert!(path.exists());
        fs::remove_file(&path).unwrap();
        let record = Publication::publish(home.path(), &"a".repeat(64), 1234).unwrap();
        let large = File::options().write(true).open(&path).unwrap();
        large.set_len(128 * 1024 * 1024).unwrap();
        drop(large);
        drop(record);
        assert_eq!(fs::metadata(path).unwrap().len(), 128 * 1024 * 1024);
    }
    #[cfg(unix)]
    #[test]
    fn symlink_directory_is_rejected() {
        let home = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(other.path(), home.path().join(".mangameeya")).unwrap();
        assert!(Publication::publish(home.path(), &"a".repeat(64), 1).is_err());
    }
}
