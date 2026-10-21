use super::{
    model::{Ledger, Phase, RestartWitness, Stamp, Transaction, digest},
    policy, restore,
    store::{Store, random_id},
};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Service {
    pub active: bool,
    pub starting: bool,
    pub enabled: bool,
    pub job: bool,
    pub invocation: Option<String>,
}

#[derive(Debug, Clone)]
pub enum BindingError {
    Conflict(String),
    Unavailable(String),
}
impl From<String> for BindingError {
    fn from(value: String) -> Self {
        Self::Unavailable(value)
    }
}
impl From<&str> for BindingError {
    fn from(value: &str) -> Self {
        Self::Conflict(value.into())
    }
}
impl From<BindingError> for String {
    fn from(value: BindingError) -> Self {
        match value {
            BindingError::Conflict(message) | BindingError::Unavailable(message) => message,
        }
    }
}

pub trait Backend {
    fn stamp(&mut self) -> Result<Stamp, String>;
    fn binding(&mut self) -> Result<String, BindingError>;
    fn service(&mut self) -> Result<Service, String>;
    fn validate(&mut self, snapshot: &Path) -> Result<(), String>;
    fn arm(&mut self, new_trial: bool) -> Result<(), String>;
    fn disarm(&mut self) -> Result<(), String>;
    fn restart(&mut self) -> Result<(), String>;
}

/// All mutating methods are called with the process-independent runtime lock held.
pub struct Engine<B> {
    pub store: Store,
    pub source: PathBuf,
    pub host: B,
}

impl<B: Backend> Engine<B> {
    fn publish(&self, ledger: &mut Ledger, tx: &Transaction) -> Result<(), String> {
        ledger.transaction = Some(tx.clone());
        self.store.save(ledger)
    }

    fn live_service(&mut self) -> Result<Service, String> {
        let service = self.host.service()?;
        if !service.active
            || !service.enabled
            || service.invocation.as_ref().is_none_or(String::is_empty)
        {
            return Err("保护应用要求服务已运行、已启用且可确认当前 InvocationID".into());
        }
        Ok(service)
    }

    pub fn checkpoint(&mut self) -> Result<Ledger, String> {
        let mut ledger = self.store.load()?;
        if ledger.pending() {
            return Err("仍有未解决的保护事务，不能重建基线".into());
        }
        self.live_service()?;
        let binding = self.host.binding()?;
        let bytes = restore::read_source(&self.source)?.ok_or("规范配置不存在")?;
        policy::network_only(&bytes, &bytes)?;
        if let Some(tx) = &ledger.transaction {
            restore::cleanup(&self.source, tx)?;
        }
        self.store.prune(&ledger)?;
        let snapshot = self.store.snapshot(&bytes, &binding)?;
        self.host
            .validate(&self.store.snapshot_path(&snapshot.id)?)?;
        self.live_service()?;
        if self.host.binding()? != binding
            || restore::read_source(&self.source)?.as_deref() != Some(bytes.as_slice())
        {
            return Err("建立基线期间配置或服务绑定发生变化，未采纳".into());
        }
        ledger.checkpoint = Some(snapshot);
        ledger.transaction = None;
        self.store.save(&ledger)?;
        self.store.prune(&ledger)?;
        Ok(ledger)
    }

    pub fn apply(&mut self, seconds: u64) -> Result<Ledger, String> {
        if !(30..=300).contains(&seconds) {
            return Err("确认窗口必须在 30–300 秒之间".into());
        }
        let mut ledger = self.store.load()?;
        if ledger.pending() {
            return Err("已有未解决保护事务，请先确认或恢复".into());
        }
        let baseline = ledger
            .checkpoint
            .clone()
            .ok_or("尚无可工作基线；请先 network checkpoint --confirm-working")?;
        self.live_service()?;
        if self.host.binding()? != baseline.binding {
            return Err("核心或服务绑定已变化，必须重新确认可工作基线".into());
        }
        let baseline_bytes = self.store.read_snapshot(&baseline)?;
        let bytes = restore::read_source(&self.source)?.ok_or("规范配置不存在")?;
        policy::network_only(&baseline_bytes, &bytes)?;
        if let Some(tx) = &ledger.transaction {
            restore::cleanup(&self.source, tx)?;
        }
        self.store.prune(&ledger)?;
        let candidate = self.store.snapshot(&bytes, &baseline.binding)?;
        self.host
            .validate(&self.store.snapshot_path(&candidate.id)?)?;
        if self.host.binding()? != baseline.binding
            || restore::read_source(&self.source)?.as_deref() != Some(bytes.as_slice())
        {
            return Err("准备期间配置或核心绑定变化，未应用".into());
        }
        if self.host.arm(true).is_err() {
            let _ = self.host.disarm();
            return Err("无法验证独立恢复任务，未请求核心重启".into());
        }
        let service = self.live_service()?;
        let mut tx = Transaction::new(
            random_id()?,
            baseline,
            candidate,
            &self.host.stamp()?,
            seconds,
            service.invocation.ok_or("缺少应用前 InvocationID")?,
        )?;
        tx.phase = Phase::Armed;
        self.publish(&mut ledger, &tx)?;
        tx.phase = Phase::Trial;
        self.publish(&mut ledger, &tx)?;
        if self.host.restart().is_err() {
            let _ = self.recover(Some(&tx.id), true, false);
            return Err("核心重启请求失败；恢复保护已保留，请查看 network status".into());
        }
        self.store.prune(&ledger)?;
        Ok(ledger)
    }

    pub fn confirm(&mut self, id: &str) -> Result<Ledger, String> {
        let mut ledger = self.store.load()?;
        let mut tx = ledger.transaction.clone().ok_or("没有待确认事务")?;
        if tx.id == id && tx.phase == Phase::Confirmed {
            return Ok(ledger);
        }
        tx.check_confirmation(id, &self.host.stamp()?)?;
        let binding = match self.host.binding() {
            Ok(binding) => binding,
            Err(error) => return self.inspection_failure(error),
        };
        if binding != tx.baseline.binding {
            return Err("服务/核心绑定已改变，不能确认".into());
        }
        self.store.read_snapshot(&tx.candidate)?;
        if restore::read_source(&self.source)?
            .as_deref()
            .map(digest)
            .as_deref()
            != Some(tx.candidate.sha256.as_str())
        {
            return Err("规范配置不再是试运行候选，不能确认".into());
        }
        let service = self.live_service()?;
        if service.invocation.as_deref() == Some(tx.pre_apply_invocation.as_str()) {
            return Err("尚未观察到新核心调用，请勿把旧核心仍 active 当成应用成功".into());
        }
        tx.check_confirmation(id, &self.host.stamp()?)?;
        ledger.checkpoint = Some(tx.candidate.clone());
        tx.phase = Phase::Confirmed;
        tx.last_error = None;
        self.publish(&mut ledger, &tx)?;
        self.finish(&mut ledger, &mut tx);
        Ok(ledger)
    }

    fn finish(&mut self, ledger: &mut Ledger, tx: &mut Transaction) {
        let cleanup = restore::cleanup(&self.source, tx);
        let disarm = self.host.disarm();
        if cleanup.is_err() || disarm.is_err() {
            tx.last_error =
                Some("事务已耐久完成，但部分临时文件/计时器需人工清理；终态不会再次回滚".into());
            let _ = self.publish(ledger, tx);
        }
    }

    fn fail(
        &self,
        ledger: &mut Ledger,
        tx: &mut Transaction,
        conflict: bool,
        message: &str,
    ) -> Result<Ledger, String> {
        tx.phase = if conflict {
            Phase::Conflict
        } else {
            Phase::RecoveryFailed
        };
        tx.last_error = Some(crate::sanitize::text(message, None));
        self.publish(ledger, tx)?;
        Err(tx.last_error.clone().unwrap_or_default())
    }

    pub fn inspection_failure(&self, error: BindingError) -> Result<Ledger, String> {
        let mut ledger = self.store.load()?;
        let Some(mut tx) = ledger.transaction.clone().filter(|tx| tx.phase.pending()) else {
            return Ok(ledger);
        };
        let (conflict, message) = match error {
            BindingError::Conflict(message) => (true, message),
            BindingError::Unavailable(message) => {
                tx.attempts = (tx.attempts + 1).min(3);
                (false, message)
            }
        };
        self.fail(&mut ledger, &mut tx, conflict, &message)
    }

    pub fn recover(
        &mut self,
        id: Option<&str>,
        manual: bool,
        startup: bool,
    ) -> Result<Ledger, String> {
        let mut ledger = self.store.load()?;
        let Some(mut tx) = ledger.transaction.clone() else {
            return Ok(ledger);
        };
        if id.is_some_and(|id| id != tx.id) {
            return Err("恢复事务 ID 不匹配，未操作".into());
        }
        if !tx.phase.pending() {
            return Ok(ledger);
        }
        if !manual && tx.phase == Phase::Conflict {
            return Ok(ledger);
        }
        let now = self.host.stamp()?;
        if !manual && !startup && matches!(tx.phase, Phase::Armed | Phase::Trial) && !tx.due(&now) {
            return Ok(ledger);
        }
        if !manual && !startup && tx.attempts >= 3 && tx.phase == Phase::RecoveryFailed {
            return Ok(ledger);
        }
        let binding = match self.host.binding() {
            Ok(binding) => binding,
            Err(error) => return self.inspection_failure(error),
        };
        if binding != tx.baseline.binding {
            return self.fail(
                &mut ledger,
                &mut tx,
                true,
                "核心或服务绑定已变化，未在未知绑定下恢复配置",
            );
        }
        let service = match self.host.service() {
            Ok(service) => service,
            Err(error) => return self.inspection_failure(BindingError::Unavailable(error)),
        };
        if !service.enabled {
            return self.fail(&mut ledger, &mut tx, true, "服务启用状态已改变，需人工处理");
        }
        if startup && (service.active || !service.starting) {
            return Err("启动前恢复仅允许在核心尚未运行的启动阶段执行".into());
        }
        if manual {
            self.host.arm(false)?;
            tx.attempts = 0;
        }
        let source_hash = restore::read_source(&self.source)?.as_deref().map(digest);
        if tx.phase == Phase::RestartPending
            && source_hash.as_deref() == Some(tx.baseline.sha256.as_str())
            && let Some(witness) = &tx.rollback_witness
        {
            let activation_matches = if witness.same_activation {
                witness.stamp.boot_id == now.boot_id && service.invocation == witness.invocation
            } else {
                witness.stamp.boot_id != now.boot_id || service.invocation != witness.invocation
            };
            if service.active && service.invocation.is_some() && activation_matches {
                // Reestablish file/directory durability before terminal publication.
                if let Err(error) = restore::install(&self.source, &self.store, &tx) {
                    return self.fail(&mut ledger, &mut tx, error.conflict, &error.message);
                }
                tx.phase = Phase::RolledBack;
                tx.last_error = None;
                ledger.checkpoint = Some(tx.baseline.clone());
                self.publish(&mut ledger, &tx)?;
                self.finish(&mut ledger, &mut tx);
                return Ok(ledger);
            }
            if !startup
                && (service.starting || service.job)
                && witness.stamp.boot_id == now.boot_id
                && now.uptime_ms.saturating_sub(witness.stamp.uptime_ms) < 30_000
            {
                return Ok(ledger);
            }
        }
        if tx.attempts >= 3 && !manual && !startup {
            return self.fail(
                &mut ledger,
                &mut tx,
                false,
                "恢复已达3次尝试上限，请人工检查后按事务 ID 重试",
            );
        }
        tx.attempts = (tx.attempts + 1).min(3);
        tx.phase = Phase::RollingBack;
        tx.rollback_witness = None;
        self.publish(&mut ledger, &tx)?;
        if let Err(error) = restore::install(&self.source, &self.store, &tx) {
            return self.fail(&mut ledger, &mut tx, error.conflict, &error.message);
        }
        let service = self.host.service()?;
        if service.active && service.invocation.is_none() {
            return self.fail(
                &mut ledger,
                &mut tx,
                false,
                "无法取得恢复前核心调用凭据，未宣告成功",
            );
        }
        if service.invocation.is_none() {
            if startup {
                return self.fail(
                    &mut ledger,
                    &mut tx,
                    false,
                    "启动调用缺少有效 InvocationID，不能记录恢复凭据",
                );
            }
            // No known activation witness exists yet. Queue a start, but do not claim it completed.
            if self.host.restart().is_err() {
                return self.fail(
                    &mut ledger,
                    &mut tx,
                    false,
                    "基线已安装，但核心启动请求失败",
                );
            }
            return Ok(ledger);
        }
        tx.rollback_witness = Some(RestartWitness {
            stamp: self.host.stamp()?,
            invocation: service.invocation,
            same_activation: startup,
        });
        tx.phase = Phase::RestartPending;
        self.publish(&mut ledger, &tx)?;
        if !startup && self.host.restart().is_err() {
            return self.fail(
                &mut ledger,
                &mut tx,
                false,
                "基线已安装，但恢复重启请求失败",
            );
        }
        Ok(ledger)
    }
}

#[cfg(test)]
mod tests;
