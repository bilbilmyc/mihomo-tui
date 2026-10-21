mod clock;
mod engine;
mod host;
mod model;
mod policy;
mod restore;
mod store;
mod units;

use engine::{Backend, BindingError, Engine};
use host::Host;
use model::{Gate, Ledger};
use serde::Serialize;
use std::{fs, path::Path};
use store::Store;

pub use model::Phase;
pub use units::HOOK_PATH;
pub const PROTOCOL: u32 = model::PROTOCOL;

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub initialized: bool,
    ledger: Option<Box<Ledger>>,
    pub remaining_seconds: Option<u64>,
}

impl Status {
    fn from_ledger(ledger: Option<Ledger>) -> Result<Self, String> {
        let remaining_seconds = if let Some(tx) = ledger
            .as_ref()
            .and_then(|ledger| ledger.transaction.as_ref())
            .filter(|tx| matches!(tx.phase, Phase::Armed | Phase::Trial))
        {
            let now = clock::current()?;
            Some(if tx.due(&now) {
                0
            } else {
                (tx.deadline_uptime_ms - now.uptime_ms).div_ceil(1000)
            })
        } else {
            None
        };
        Ok(Self {
            initialized: ledger.is_some(),
            ledger: ledger.map(Box::new),
            remaining_seconds,
        })
    }

    pub fn pending(&self) -> bool {
        self.ledger.as_ref().is_some_and(|ledger| ledger.pending())
    }
    pub fn phase(&self) -> Option<Phase> {
        self.ledger
            .as_ref()?
            .transaction
            .as_ref()
            .map(|tx| tx.phase)
    }
    pub fn id(&self) -> Option<&str> {
        Some(&self.ledger.as_ref()?.transaction.as_ref()?.id)
    }

    pub fn render(&self) -> String {
        let Some(ledger) = &self.ledger else {
            return "保护设施尚未初始化；请先核实当前配置已加载且网络可用，再执行 network checkpoint --confirm-working".into();
        };
        let baseline = if ledger.checkpoint.is_some() {
            "已保存人工确认基线（绑定变化后须重建）"
        } else {
            "尚无人工确认基线"
        };
        let Some(tx) = &ledger.transaction else {
            return format!("保护设施已初始化；{baseline}；无待处理事务");
        };
        let mut text = format!(
            "{baseline}\n事务 {}：{}",
            self.id().unwrap_or("未知"),
            self.phase().map(|phase| phase.label()).unwrap_or("无事务")
        );
        if let Some(seconds) = self.remaining_seconds {
            text.push_str(&format!("；剩余 {seconds} 秒，未确认将恢复基线"));
        }
        if let Some(error) = &tx.last_error {
            text.push_str(&format!("\n{}", crate::sanitize::text(error, None)));
        }
        text
    }
}

pub fn status() -> Result<Status, String> {
    let ledger = Store::existing(Path::new(units::ROOT))?
        .map(|store| store.load())
        .transpose()?;
    Status::from_ledger(ledger)
}

pub fn reject_pending() -> Result<(), String> {
    if let Some(store) = Store::existing(Path::new(units::ROOT))?
        && store.load()?.pending()
    {
        return Err("有未解决的网络保护事务，拒绝编辑、普通应用或升级；请先 network status".into());
    }
    Ok(())
}

pub fn canonical_source(path: &Path) -> bool {
    let source = Path::new(crate::workspace::DEFAULT_SOURCE_PATH);
    path == source
        || (path.file_name() == source.file_name()
            && path
                .parent()
                .and_then(|parent| parent.canonicalize().ok())
                .zip(
                    source
                        .parent()
                        .and_then(|parent| parent.canonicalize().ok()),
                )
                .is_some_and(|(left, right)| left == right))
}

pub fn lock_config_access(path: &Path) -> Result<Option<crate::system::RuntimeLock>, String> {
    if canonical_source(path) {
        crate::system::acquire_runtime_lock().map(Some)
    } else {
        Ok(None)
    }
}

pub fn lock_config_edit(path: &Path) -> Result<Option<crate::system::RuntimeLock>, String> {
    let lock = lock_config_access(path)?;
    if lock.is_some() {
        reject_pending()?;
    }
    Ok(lock)
}

pub fn validate_hook() -> Result<(), String> {
    host::validate_hook()
}
pub fn read_service_file(path: &Path) -> Result<Vec<u8>, String> {
    host::read_public(path, 64 * 1024)
}

fn require_root() -> Result<(), String> {
    if !crate::system::effective_root() {
        return Err("本机保护操作需要 root 权限".into());
    }
    Ok(())
}

fn existing_engine() -> Result<Engine<Host>, String> {
    let store = Store::existing(Path::new(units::ROOT))?
        .ok_or("尚未建立保护基线；请先 network checkpoint --confirm-working")?;
    let host = Host::new(store.clone())?;
    host.compatible()?;
    Ok(Engine {
        store,
        source: crate::workspace::DEFAULT_SOURCE_PATH.into(),
        host,
    })
}

pub fn checkpoint(confirm_working: bool) -> Result<Status, String> {
    if !confirm_working {
        return Err("必须明确声明磁盘配置已加载且网络可用：--confirm-working".into());
    }
    require_root()?;
    let _lock = crate::system::acquire_runtime_lock()?;
    reject_pending()?;
    for parent in ["/var", "/var/lib"] {
        if !crate::system::trusted_root_directory(Path::new(parent)) {
            return Err("恢复状态父目录不安全".into());
        }
    }
    let parent = Path::new("/var/lib/mihomo-tui");
    if fs::symlink_metadata(parent).is_err() {
        store::private_directory(parent)?;
    }
    if !crate::system::trusted_root_directory(parent) {
        return Err("恢复状态父目录不安全".into());
    }
    let store = Store::initialize(Path::new(units::ROOT))?;
    let mut host = Host::new(store.clone())?;
    host.bootstrap()?;
    let mut engine = Engine {
        store,
        source: crate::workspace::DEFAULT_SOURCE_PATH.into(),
        host,
    };
    Status::from_ledger(Some(engine.checkpoint()?))
}

pub fn apply(seconds: u64) -> Result<Status, String> {
    require_root()?;
    let _lock = crate::system::acquire_runtime_lock()?;
    Status::from_ledger(Some(existing_engine()?.apply(seconds)?))
}

pub fn confirm(id: &str) -> Result<Status, String> {
    require_root()?;
    let _lock = crate::system::acquire_runtime_lock()?;
    Status::from_ledger(Some(existing_engine()?.confirm(id)?))
}

pub fn rollback(id: &str) -> Result<Status, String> {
    require_root()?;
    let _lock = crate::system::acquire_runtime_lock()?;
    Status::from_ledger(Some(existing_engine()?.recover(Some(id), true, false)?))
}

pub fn recover(startup: bool) -> Result<(), String> {
    require_root()?;
    let Some(store) = Store::existing(Path::new(units::ROOT))? else {
        return Ok(());
    };
    let ledger = store.load()?;
    // These paths must remain lock-free: a parent may hold the mutation lock while starting Mihomo.
    if !ledger.pending() {
        return Ok(());
    }
    let now = clock::current()?;
    if !startup
        && ledger
            .transaction
            .as_ref()
            .is_some_and(|tx| matches!(tx.phase, Phase::Trial | Phase::Armed) && !tx.due(&now))
    {
        return Ok(());
    }
    let mut host = Host::new(store.clone())?;
    if !startup
        && ledger.transaction.as_ref().is_some_and(|tx| {
            tx.phase == Phase::Conflict || (tx.phase == Phase::RecoveryFailed && tx.attempts >= 3)
        })
    {
        let _ = host.disarm();
        return Ok(());
    }
    let observed_id = ledger.transaction.as_ref().map(|tx| tx.id.clone());
    let incompatible = host.compatible().err().map(BindingError::Conflict);
    if startup
        && incompatible.is_none()
        && let (Ok(source), Ok(binding)) = (
            restore::read_source(Path::new(crate::workspace::DEFAULT_SOURCE_PATH)),
            host.binding(),
        )
    {
        let hash = source.as_deref().map(model::digest);
        if ledger.startup_gate(&now, hash.as_deref(), &binding) == Gate::Allow {
            return Ok(());
        }
    }
    let _lock = crate::system::acquire_runtime_lock()?;
    let current = store.load()?;
    if !current.pending() {
        return Ok(());
    }
    if current.transaction.as_ref().map(|tx| &tx.id) != observed_id.as_ref() {
        return if startup {
            Err("启动前事务已变化，已拒绝使用旧任务继续".into())
        } else {
            Ok(())
        };
    }
    let mut engine = Engine {
        store: store.clone(),
        source: crate::workspace::DEFAULT_SOURCE_PATH.into(),
        host,
    };
    if let Some(error) = incompatible {
        let result = engine.inspection_failure(error);
        return finish_recovery(&mut engine, result);
    }
    if startup {
        let source = match restore::read_source(Path::new(crate::workspace::DEFAULT_SOURCE_PATH)) {
            Ok(source) => source,
            Err(error) => {
                let result = engine.inspection_failure(BindingError::Conflict(error));
                return finish_recovery(&mut engine, result);
            }
        };
        let binding = match engine.host.binding() {
            Ok(binding) => binding,
            Err(error) => {
                let result = engine.inspection_failure(error);
                return finish_recovery(&mut engine, result);
            }
        };
        let hash = source.as_deref().map(model::digest);
        match current.startup_gate(&clock::current()?, hash.as_deref(), &binding) {
            Gate::Allow => return Ok(()),
            Gate::Refuse => {
                let result = engine.inspection_failure(BindingError::Conflict(
                    "启动配置或绑定未知，已拒绝启动核心".into(),
                ));
                return finish_recovery(&mut engine, result);
            }
            Gate::Recover => {}
        }
        let service = engine.host.service()?;
        if !service.starting
            || service.active
            || service
                .invocation
                .as_ref()
                .is_none_or(|id| !model::valid_invocation(id))
            || std::env::var("INVOCATION_ID").ok().as_deref() != service.invocation.as_deref()
        {
            return Err("启动前恢复仅供当前 systemd 启动调用使用".into());
        }
    }
    let result = engine.recover(None, false, startup);
    finish_recovery(&mut engine, result)
}

fn finish_recovery(
    engine: &mut Engine<Host>,
    result: Result<Ledger, String>,
) -> Result<(), String> {
    if engine.store.load()?.transaction.as_ref().is_some_and(|tx| {
        tx.phase == Phase::Conflict || (tx.phase == Phase::RecoveryFailed && tx.attempts >= 3)
    }) {
        let _ = engine.host.disarm();
    }
    result.map(|_| ())
}

#[cfg(test)]
pub(crate) fn test_status(phase: Phase) -> Status {
    let baseline = model::Snapshot::fixture("b", "1");
    let candidate = model::Snapshot::fixture("c", "2");
    let mut tx = model::Transaction::new(
        "a".repeat(32),
        baseline.clone(),
        candidate.clone(),
        &model::Stamp {
            boot_id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into(),
            uptime_ms: 0,
        },
        90,
        "e".repeat(32),
    )
    .unwrap();
    tx.phase = phase;
    let ledger = Ledger {
        checkpoint: Some(if phase == Phase::Confirmed {
            candidate
        } else {
            baseline
        }),
        transaction: Some(tx),
        ..Ledger::default()
    };
    ledger.validate().unwrap();
    Status {
        initialized: true,
        remaining_seconds: if phase == Phase::Trial {
            Some(90)
        } else {
            None
        },
        ledger: Some(Box::new(ledger)),
    }
}
