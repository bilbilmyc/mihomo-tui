use super::{
    model::{Transaction, digest, hex_id},
    store::{self, CONFIG_LIMIT, Store},
};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub struct Failure {
    pub conflict: bool,
    pub message: String,
}
impl Failure {
    fn conflict(message: &str) -> Self {
        Self {
            conflict: true,
            message: message.into(),
        }
    }
    fn io(message: &str) -> Self {
        Self {
            conflict: false,
            message: message.into(),
        }
    }
}
impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self {
            conflict: false,
            message,
        }
    }
}

pub fn read_source(source: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::symlink_metadata(source) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("无法检查规范配置".into()),
        Ok(_) => store::read_private_links(source, CONFIG_LIMIT).map(Some),
    }
}

pub fn paths(source: &Path, id: &str) -> Result<(PathBuf, PathBuf), String> {
    if !hex_id(id, 32) {
        return Err("恢复事务 ID 无效".into());
    }
    let parent = source.parent().ok_or("规范配置缺少父目录")?;
    if !crate::system::trusted_root_directory(parent) {
        return Err("规范配置目录不安全".into());
    }
    Ok((
        parent.join(format!(".mihomo-tui-guard-{id}.old")),
        parent.join(format!(".mihomo-tui-guard-{id}.new")),
    ))
}

fn sync_baseline(source: &Path, expected_hash: &str) -> Result<(), Failure> {
    let mut file = store::open_private(source, CONFIG_LIMIT, true)?;
    if digest(&store::read_handle(&mut file, CONFIG_LIMIT, true)?) != expected_hash {
        return Err(Failure::conflict("同步前配置已改变，未宣告恢复成功"));
    }
    file.sync_all().map_err(|_| Failure::io("无法同步恢复配置"))
}

fn remove_prepared(prepared: &Path, baseline_hash: &str) -> Result<(), Failure> {
    if fs::symlink_metadata(prepared).is_ok() {
        if digest(&store::read_private_links(prepared, CONFIG_LIMIT)?) != baseline_hash {
            return Err(Failure::conflict("临时恢复文件不匹配，已保留供人工检查"));
        }
        fs::remove_file(prepared).map_err(|_| Failure::io("无法清理恢复临时文件"))?;
    }
    Ok(())
}

pub fn install(source: &Path, store: &Store, tx: &Transaction) -> Result<(), Failure> {
    let bytes = store.read_snapshot(&tx.baseline)?;
    let (displaced, prepared) = paths(source, &tx.id)?;
    let parent = source
        .parent()
        .ok_or_else(|| Failure::io("配置缺少父目录"))?;
    let current = read_source(source)?;
    if let Some(current) = &current {
        let hash = digest(current);
        if hash == tx.baseline.sha256 {
            // The file may have been published just before a crash; finish durability first.
            sync_baseline(source, &tx.baseline.sha256)?;
            remove_prepared(&prepared, &tx.baseline.sha256)?;
            store::sync_dir(parent)?;
            return Ok(());
        }
        if hash != tx.candidate.sha256 {
            return Err(Failure::conflict("配置已被后续编辑修改，未覆盖该编辑"));
        }
        if fs::symlink_metadata(&displaced).is_ok() {
            return Err(Failure::conflict(
                "捕获文件存在且规范路径又被写入，需人工处理",
            ));
        }
    } else if fs::symlink_metadata(&displaced).is_err() {
        return Err(Failure::conflict(
            "配置与捕获文件均缺失，不能推断可以覆盖的内容",
        ));
    }
    if fs::symlink_metadata(&prepared).is_ok() {
        if digest(&store::read_private_links(&prepared, CONFIG_LIMIT)?) != tx.baseline.sha256 {
            return Err(Failure::conflict("恢复临时文件内容未知，未覆盖"));
        }
    } else {
        let temporary = parent.join(format!(
            ".mihomo-tui-guard-{}-{}.tmp",
            tx.id,
            store::random_id()?
        ));
        store::write_new(&temporary, &bytes)?;
        fs::rename(&temporary, &prepared).map_err(|_| Failure::io("无法准备恢复文件"))?;
        store::sync_dir(parent)?;
    }
    if current.is_some() {
        // Capture the entry actually present at rename time, including a racing edit.
        fs::rename(source, &displaced).map_err(|_| Failure::io("无法捕获待替换配置"))?;
        store::sync_dir(parent)?;
    }
    let old_hash = digest(&store::read_private_links(&displaced, CONFIG_LIMIT)?);
    if old_hash != tx.candidate.sha256 && old_hash != tx.baseline.sha256 {
        // Re-publish an unexpected captured edit only if the canonical name is still absent.
        match fs::hard_link(&displaced, source) {
            Ok(()) => store::sync_dir(parent)?,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(Failure::io("后续编辑已保留，但无法重建规范路径")),
        }
        return Err(Failure::conflict("捕获到并发编辑，已保留内容且未发布基线"));
    }
    match fs::hard_link(&prepared, source) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if read_source(source)?.as_deref().map(digest).as_deref()
                != Some(tx.baseline.sha256.as_str())
            {
                return Err(Failure::conflict("规范路径已被另一个写入者创建，未覆盖"));
            }
        }
        Err(_) => {
            return Err(Failure::io(
                "无法不覆盖地发布基线，请检查文件系统硬链接支持",
            ));
        }
    }
    sync_baseline(source, &tx.baseline.sha256)?;
    remove_prepared(&prepared, &tx.baseline.sha256)?;
    store::sync_dir(parent)?;
    Ok(())
}

pub fn cleanup(source: &Path, tx: &Transaction) -> Result<(), String> {
    let (displaced, prepared) = paths(source, &tx.id)?;
    if fs::symlink_metadata(&displaced).is_ok() {
        let hash = digest(&store::read_private_links(&displaced, CONFIG_LIMIT)?);
        if hash != tx.candidate.sha256 && hash != tx.baseline.sha256 {
            return Err("捕获文件包含未知编辑，已保留".into());
        }
        fs::remove_file(displaced).map_err(|_| "无法清理已备份的候选配置")?;
    }
    remove_prepared(&prepared, &tx.baseline.sha256).map_err(|error| error.message)?;
    store::sync_dir(source.parent().ok_or("配置缺少父目录")?)
}

#[cfg(test)]
mod tests {
    use super::super::{
        model::{Stamp, Transaction},
        store::{Store, tests::directory, write_new},
    };
    use super::*;

    fn fixture() -> (std::path::PathBuf, Store, std::path::PathBuf, Transaction) {
        let root = directory();
        let store = Store::initialize(&root.join("state")).unwrap();
        let source = root.join("config.yaml");
        write_new(&source, b"candidate").unwrap();
        let baseline = store.snapshot(b"baseline", &"f".repeat(64)).unwrap();
        let candidate = store.snapshot(b"candidate", &"f".repeat(64)).unwrap();
        let tx = Transaction::new(
            "a".repeat(32),
            baseline,
            candidate,
            &Stamp {
                boot_id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into(),
                uptime_ms: 0,
            },
            90,
            "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".into(),
        )
        .unwrap();
        (root, store, source, tx)
    }

    #[test]
    fn restoration_is_idempotent_and_preserves_the_displaced_candidate_until_completion() {
        let (root, store, source, tx) = fixture();
        install(&source, &store, &tx).unwrap();
        install(&source, &store, &tx).unwrap();
        assert_eq!(std::fs::read(&source).unwrap(), b"baseline");
        let (displaced, _) = paths(&source, &tx.id).unwrap();
        assert_eq!(std::fs::read(displaced).unwrap(), b"candidate");
        cleanup(&source, &tx).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unknown_later_edits_are_not_overwritten() {
        let (root, store, source, tx) = fixture();
        std::fs::write(&source, b"later edit").unwrap();
        assert!(install(&source, &store, &tx).unwrap_err().conflict);
        assert_eq!(std::fs::read(&source).unwrap(), b"later edit");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_crash_after_capture_and_after_publish_can_be_resumed_without_losing_files() {
        let (root, store, source, tx) = fixture();
        let (old, prepared) = paths(&source, &tx.id).unwrap();
        write_new(&prepared, b"baseline").unwrap();
        std::fs::rename(&source, &old).unwrap();
        install(&source, &store, &tx).unwrap();
        assert_eq!(std::fs::read(&source).unwrap(), b"baseline");
        // Emulate a crash after no-clobber publication but before temporary unlink.
        std::fs::hard_link(&source, &prepared).unwrap();
        install(&source, &store, &tx).unwrap();
        assert!(!prepared.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unexpected_displaced_content_is_republished_without_overwriting_another_writer() {
        let (root, store, source, tx) = fixture();
        let (old, _) = paths(&source, &tx.id).unwrap();
        std::fs::rename(&source, &old).unwrap();
        std::fs::write(&old, b"racing edit").unwrap();
        assert!(install(&source, &store, &tx).unwrap_err().conflict);
        assert_eq!(std::fs::read(&source).unwrap(), b"racing edit");
        assert_eq!(std::fs::read(old).unwrap(), b"racing edit");
        std::fs::remove_dir_all(root).unwrap();
    }
}
