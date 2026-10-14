use super::*;
use std::{
    fs,
    os::unix::fs::{symlink, MetadataExt},
    path::PathBuf,
};
pub(crate) struct Directory(pub PathBuf);
impl Directory {
    pub(crate) fn new() -> Self {
        let nonce = NEXT_TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("raiv-disk-test-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn cid(body: &[u8]) -> DaslCid {
    DaslCid::for_block(raiv::format::DaslCodec::Drisl, body)
}
#[test]
fn exact_bytes_reopen_inode_reuse_lock_and_ignored_orphan_staging() {
    let directory = Directory::new();
    let disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    assert!(matches!(
        DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()),
        Err(DiskError::Io(_))
    ));
    let body = [0xa0];
    let cid = cid(&body);
    disk.put(cid, &body).unwrap();
    let path = directory.0.join(cid.to_string());
    let inode = fs::metadata(&path).unwrap().ino();
    disk.put(cid, &body).unwrap();
    assert_eq!(fs::metadata(&path).unwrap().ino(), inode);
    assert_eq!(fs::read(&path).unwrap(), body);
    assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
    fs::write(directory.0.join(".raiv-block-interrupted"), [0xff]).unwrap();
    drop(disk);
    let reopened = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    assert_eq!(reopened.load(cid).unwrap(), body);
}
#[test]
fn validation_before_publication_and_corrupt_existing_file_is_never_overwritten() {
    let directory = Directory::new();
    let disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    let bad = [0x18, 0x00];
    assert!(matches!(
        disk.put(cid(&bad), &bad),
        Err(DiskError::Data(StoreError::Codec(_)))
    ));
    let valid = [0xa0];
    let good = cid(&valid);
    assert!(matches!(
        disk.put(good, &[0x80]),
        Err(DiskError::Data(StoreError::Integrity))
    ));
    let raw = DaslCid::for_block(raiv::format::DaslCodec::Raw, &valid);
    assert!(disk.put(raw, &valid).is_err());
    assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 0);
    disk.put(good, &valid).unwrap();
    let path = directory.0.join(good.to_string());
    fs::write(&path, [0x80]).unwrap();
    assert!(matches!(
        disk.put(good, &valid),
        Err(DiskError::Data(StoreError::Integrity))
    ));
    assert_eq!(fs::read(&path).unwrap(), [0x80]);
    assert!(disk.load(good).is_err());
    fs::write(&path, [0xa0; 65_537]).unwrap();
    assert!(matches!(disk.load(good), Err(DiskError::Length)));
    fs::write(&path, []).unwrap();
    assert!(disk.load(good).is_err());
}
#[test]
fn symlinks_nonregular_paths_and_renamed_directory_never_redirect_reads_or_overwrite() {
    let directory = Directory::new();
    let elsewhere = Directory::new();
    let link = elsewhere.0.join("link");
    symlink(&directory.0, &link).unwrap();
    assert!(DiskBlockStore::open_existing(&link, DecodeLimits::default()).is_err());
    let disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    let bytes = [0xa0];
    let good = cid(&bytes);
    let name = good.to_string();
    let target = elsewhere.0.join("sentinel");
    fs::write(&target, [0x80]).unwrap();
    symlink(&target, directory.0.join(&name)).unwrap();
    assert!(disk.put(good, &bytes).is_err());
    assert!(disk.load(good).is_err());
    assert_eq!(fs::read(&target).unwrap(), [0x80]);
    fs::remove_file(directory.0.join(&name)).unwrap();
    fs::create_dir(directory.0.join(&name)).unwrap();
    assert!(matches!(disk.load(good), Err(DiskError::NotRegular)));
    fs::remove_dir(directory.0.join(&name)).unwrap();
    let renamed = elsewhere.0.join("renamed");
    fs::rename(&directory.0, &renamed).unwrap();
    fs::create_dir(&directory.0).unwrap();
    disk.put(good, &bytes).unwrap();
    assert_eq!(disk.load(good).unwrap(), bytes);
    assert!(renamed.join(&name).is_file());
    assert!(!directory.0.join(&name).exists());
    drop(disk);
}
#[test]
fn tighter_wire_bounds_reject_existing_bytes_without_rewrite() {
    let directory = Directory::new();
    let disk = DiskBlockStore::open_existing(&directory.0, DecodeLimits::default()).unwrap();
    let bytes = [0xa1, 0x61, b'x', 0xa0];
    let good = cid(&bytes);
    disk.put(good, &bytes).unwrap();
    drop(disk);
    let wire = DecodeLimits {
        max_block_bytes: 3,
        ..DecodeLimits::default()
    };
    let disk = DiskBlockStore::open_existing(&directory.0, wire).unwrap();
    assert!(matches!(disk.load(good), Err(DiskError::Length)));
    assert!(matches!(
        disk.put(good, &bytes),
        Err(DiskError::Data(StoreError::Codec(_)))
    ));
    assert_eq!(fs::read(directory.0.join(good.to_string())).unwrap(), bytes);
}
