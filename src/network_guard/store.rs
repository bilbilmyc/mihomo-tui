use super::model::{Ledger, Snapshot, digest, hex_id};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub const CONFIG_LIMIT: usize = 2 * 1024 * 1024;
const LEDGER_LIMIT: usize = 64 * 1024;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SealedLedger {
    payload: Ledger,
    sha256: String,
}

pub fn random_id() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|_| "无法取得安全的恢复事务 ID".to_string())?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub fn sync_dir(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| "无法同步恢复目录".into())
}

pub fn private_directory(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            builder.mode(0o700);
            builder.create(path).map_err(|_| "无法创建私有恢复目录")?;
            if let Some(parent) = path.parent() {
                sync_dir(parent)?;
            }
        }
        Err(_) => return Err("无法检查恢复目录".into()),
    }
    check_directory(path)
}

fn check_directory(path: &Path) -> Result<(), String> {
    if !crate::system::trusted_root_directory(path) {
        return Err("恢复目录所有者、权限或类型不安全".into());
    }
    #[cfg(unix)]
    if fs::symlink_metadata(path)
        .map_err(|_| "无法检查恢复目录权限")?
        .mode()
        & 0o077
        != 0
    {
        return Err("恢复目录必须为私有 0700 权限".into());
    }
    Ok(())
}

pub fn read_private(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    read_private_file(path, limit, false)
}

pub fn read_private_links(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    read_private_file(path, limit, true)
}

fn read_private_file(path: &Path, limit: usize, allow_links: bool) -> Result<Vec<u8>, String> {
    let mut file = open_private(path, limit, allow_links)?;
    read_handle(&mut file, limit, allow_links)
}

fn validate_metadata(
    metadata: &fs::Metadata,
    limit: usize,
    allow_links: bool,
) -> Result<(), String> {
    if !metadata.is_file() {
        return Err("恢复来源不是普通文件".into());
    }
    #[cfg(unix)]
    if !crate::system::owner_is_trusted(metadata.uid())
        || metadata.mode() & 0o077 != 0
        || (!allow_links && metadata.nlink() != 1)
    {
        return Err("恢复文件所有者、权限或链接数不安全".into());
    }
    if metadata.len() > limit as u64 {
        return Err("恢复文件超过读取上限".into());
    }
    Ok(())
}

pub fn open_private(path: &Path, limit: usize, allow_links: bool) -> Result<File, String> {
    open_private_after(path, limit, allow_links, || {})
}

fn open_private_after(
    path: &Path,
    limit: usize,
    allow_links: bool,
    before_open: impl FnOnce(),
) -> Result<File, String> {
    #[cfg(target_os = "linux")]
    {
        let mut options = OpenOptions::new();
        // Linux O_NOFOLLOW | O_NONBLOCK: reject symlinks, and never wait on a substituted FIFO.
        options.read(true).custom_flags(0o400000 | 0o4000);
        before_open();
        let file = options
            .open(path)
            .map_err(|_| "无法安全打开恢复文件（禁止符号链接）")?;
        validate_metadata(
            &file.metadata().map_err(|_| "无法验证恢复文件描述符")?,
            limit,
            allow_links,
        )?;
        Ok(file)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (path, limit, allow_links, before_open);
        Err("保护配置功能仅支持 Linux".into())
    }
}

pub fn read_handle(file: &mut File, limit: usize, allow_links: bool) -> Result<Vec<u8>, String> {
    validate_metadata(
        &file.metadata().map_err(|_| "无法检查打开的恢复文件")?,
        limit,
        allow_links,
    )?;
    let mut bytes = Vec::new();
    Read::by_ref(file)
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取恢复文件".to_string())?;
    if bytes.len() > limit {
        return Err("恢复文件超过读取上限".into());
    }
    validate_metadata(
        &file.metadata().map_err(|_| "无法复查恢复文件描述符")?,
        limit,
        allow_links,
    )?;
    Ok(bytes)
}

pub fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path).map_err(|_| "无法创建私有恢复文件")?;
    if file
        .write_all(bytes)
        .and_then(|()| file.sync_all())
        .is_err()
    {
        let _ = fs::remove_file(path);
        return Err("无法耐久写入恢复文件".into());
    }
    sync_dir(path.parent().ok_or("恢复文件缺少父目录")?)
}

pub fn atomic_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if fs::symlink_metadata(path).is_ok() {
        read_private(path, LEDGER_LIMIT)?;
    }
    let parent = path.parent().ok_or("恢复文件缺少父目录")?;
    let temporary = parent.join(format!(".ledger-{}.tmp", random_id()?));
    write_new(&temporary, bytes)?;
    if fs::rename(&temporary, path).is_err() {
        let _ = fs::remove_file(&temporary);
        return Err("无法原子提交恢复账本".into());
    }
    sync_dir(parent)
}

#[derive(Clone)]
pub struct Store {
    pub root: PathBuf,
}

impl Store {
    pub fn existing(root: &Path) -> Result<Option<Self>, String> {
        match fs::symlink_metadata(root) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err("无法访问恢复状态目录".into()),
            Ok(_) => {}
        }
        // Read-only inspection must not create a missing child directory.
        if !root.is_dir() || !root.join("snapshots").is_dir() {
            return Err("恢复状态目录不完整".into());
        }
        check_directory(root)?;
        check_directory(&root.join("snapshots"))?;
        let store = Self { root: root.into() };
        store.load()?;
        Ok(Some(store))
    }

    pub fn initialize(root: &Path) -> Result<Self, String> {
        private_directory(root)?;
        private_directory(&root.join("snapshots"))?;
        let store = Self { root: root.into() };
        if !store
            .root
            .join("ledger.json")
            .try_exists()
            .map_err(|_| "无法检查恢复账本")?
        {
            store.save(&Ledger::default())?;
        }
        store.load()?;
        Ok(store)
    }

    pub fn load(&self) -> Result<Ledger, String> {
        let bytes = read_private(&self.root.join("ledger.json"), LEDGER_LIMIT)?;
        let envelope: SealedLedger =
            serde_json::from_slice(&bytes).map_err(|_| "恢复账本无法解析；未重置或修改配置")?;
        let encoded = serde_json::to_vec(&envelope.payload).map_err(|_| "无法核对账本完整性")?;
        if !hex_id(&envelope.sha256, 64) || digest(&encoded) != envelope.sha256 {
            return Err("恢复账本完整性校验失败；未修改配置".into());
        }
        let ledger = envelope.payload;
        ledger.validate()?;
        Ok(ledger)
    }

    pub fn save(&self, ledger: &Ledger) -> Result<(), String> {
        ledger.validate()?;
        let encoded = serde_json::to_vec(ledger).map_err(|_| "无法编码恢复账本")?;
        let envelope = SealedLedger {
            payload: ledger.clone(),
            sha256: digest(&encoded),
        };
        let bytes = serde_json::to_vec(&envelope).map_err(|_| "无法编码恢复账本完整性封装")?;
        if bytes.len() > LEDGER_LIMIT {
            return Err("恢复账本超过上限".into());
        }
        atomic_private(&self.root.join("ledger.json"), &bytes)
    }

    pub fn snapshot_path(&self, id: &str) -> Result<PathBuf, String> {
        if !hex_id(id, 32) {
            return Err("恢复快照 ID 无效".into());
        }
        Ok(self.root.join("snapshots").join(format!("{id}.yaml")))
    }

    pub fn snapshot(&self, bytes: &[u8], binding: &str) -> Result<Snapshot, String> {
        if bytes.len() > CONFIG_LIMIT || !hex_id(binding, 64) {
            return Err("恢复快照大小或服务绑定无效".into());
        }
        let id = random_id()?;
        write_new(&self.snapshot_path(&id)?, bytes)?;
        Ok(Snapshot {
            id,
            sha256: digest(bytes),
            binding: binding.into(),
        })
    }

    pub fn read_snapshot(&self, snapshot: &Snapshot) -> Result<Vec<u8>, String> {
        let bytes = read_private(&self.snapshot_path(&snapshot.id)?, CONFIG_LIMIT)?;
        if digest(&bytes) != snapshot.sha256 {
            return Err("恢复快照哈希不匹配；未修改配置".into());
        }
        Ok(bytes)
    }

    pub fn prune(&self, ledger: &Ledger) -> Result<(), String> {
        let mut keep = Vec::new();
        if let Some(snapshot) = &ledger.checkpoint {
            keep.push(snapshot.id.as_str());
        }
        if let Some(tx) = &ledger.transaction {
            keep.extend([tx.baseline.id.as_str(), tx.candidate.id.as_str()]);
        }
        let directory = self.root.join("snapshots");
        for (index, entry) in fs::read_dir(&directory)
            .map_err(|_| "无法检查旧恢复快照")?
            .enumerate()
        {
            if index >= 256 {
                return Err("恢复目录条目过多，需要人工检查".into());
            }
            let entry = entry.map_err(|_| "无法检查恢复快照")?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(id) = name.strip_suffix(".yaml").filter(|id| hex_id(id, 32))
                && !keep.contains(&id)
            {
                read_private(&entry.path(), CONFIG_LIMIT)?;
                fs::remove_file(entry.path()).map_err(|_| "无法清理未引用的恢复快照")?;
            }
        }
        sync_dir(&directory)
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::super::model::{Ledger, digest};
    use super::*;

    pub fn directory() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "mihomo-guard-{}-{}",
            std::process::id(),
            random_id().unwrap()
        ));
        private_directory(&path).unwrap();
        path
    }

    #[test]
    fn read_only_status_never_initializes_missing_state() {
        let path =
            std::env::temp_dir().join(format!("mihomo-guard-missing-{}", random_id().unwrap()));
        assert!(Store::existing(&path).unwrap().is_none());
        assert!(!path.exists());
    }

    #[test]
    fn snapshots_and_ledger_are_private_bounded_and_hash_verified() {
        let path = directory();
        let store = Store::initialize(&path).unwrap();
        let snapshot = store
            .snapshot(b"tun: {enable: false}\n", &"f".repeat(64))
            .unwrap();
        assert_eq!(snapshot.sha256, digest(b"tun: {enable: false}\n"));
        let ledger = Ledger {
            checkpoint: Some(snapshot.clone()),
            ..Ledger::default()
        };
        store.save(&ledger).unwrap();
        assert_eq!(store.load().unwrap().checkpoint, Some(snapshot.clone()));
        assert_eq!(
            store.read_snapshot(&snapshot).unwrap(),
            b"tun: {enable: false}\n"
        );
        std::fs::write(store.snapshot_path(&snapshot.id).unwrap(), b"tampered").unwrap();
        assert!(store.read_snapshot(&snapshot).is_err());
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn malformed_or_future_metadata_is_not_silently_reset() {
        let path = directory();
        let store = Store::initialize(&path).unwrap();
        atomic_private(
            &path.join("ledger.json"),
            br#"{"protocol":999,"checkpoint":null,"transaction":null}"#,
        )
        .unwrap();
        assert!(store.load().is_err());
        assert!(Store::initialize(&path).is_err());
        assert!(store.snapshot_path("../../etc/shadow").is_err());
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn unreferenced_snapshots_are_pruned_but_the_checkpoint_is_retained() {
        let path = directory();
        let store = Store::initialize(&path).unwrap();
        let baseline = store.snapshot(b"baseline", &"f".repeat(64)).unwrap();
        let orphan = store.snapshot(b"orphan", &"f".repeat(64)).unwrap();
        let ledger = Ledger {
            checkpoint: Some(baseline.clone()),
            ..Ledger::default()
        };
        assert!(!ledger.pending());
        store.save(&ledger).unwrap();
        store.prune(&ledger).unwrap();
        assert!(store.snapshot_path(&baseline.id).unwrap().exists());
        assert!(!store.snapshot_path(&orphan.id).unwrap().exists());
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn valid_looking_metadata_corruption_is_detected_by_the_ledger_seal() {
        let root = directory();
        let store = Store::initialize(&root).unwrap();
        let snapshot = store.snapshot(b"baseline", &"f".repeat(64)).unwrap();
        store
            .save(&Ledger {
                checkpoint: Some(snapshot),
                ..Ledger::default()
            })
            .unwrap();
        let path = root.join("ledger.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let payload = if value.get("payload").is_some() {
            &mut value["payload"]
        } else {
            &mut value
        };
        payload["checkpoint"]["binding"] = serde_json::Value::String("e".repeat(64));
        atomic_private(&path, &serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(store.load().is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_and_public_snapshot_permissions_are_refused() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let path = directory();
        let store = Store::initialize(&path).unwrap();
        let snapshot = store.snapshot(b"rules: []", &"f".repeat(64)).unwrap();
        let file = store.snapshot_path(&snapshot.id).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(store.read_snapshot(&snapshot).is_err());
        std::fs::remove_file(&file).unwrap();
        symlink("/dev/null", &file).unwrap();
        assert!(store.read_snapshot(&snapshot).is_err());
        std::fs::remove_dir_all(path).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn descriptor_validation_rejects_replacements_at_the_open_boundary_without_fifo_hangs() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        for kind in ["symlink", "public", "fifo"] {
            let root = directory();
            let path = root.join("config.yaml");
            let baseline = root.join("baseline.yaml");
            write_new(&path, b"candidate").unwrap();
            write_new(&baseline, b"baseline").unwrap();
            let before = std::time::Instant::now();
            let result = open_private_after(&path, CONFIG_LIMIT, true, || {
                std::fs::remove_file(&path).unwrap();
                match kind {
                    "symlink" => symlink(&baseline, &path).unwrap(),
                    "public" => {
                        std::fs::write(&path, b"baseline").unwrap();
                        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
                            .unwrap();
                    }
                    _ => assert!(
                        std::process::Command::new("/usr/bin/mkfifo")
                            .args(["-m", "600"])
                            .arg(&path)
                            .status()
                            .unwrap()
                            .success()
                    ),
                }
            });
            assert!(result.is_err(), "replacement {kind} was accepted");
            assert!(before.elapsed() < std::time::Duration::from_secs(2));
            assert!(std::fs::symlink_metadata(&path).is_ok());
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}
