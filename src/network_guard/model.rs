use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const PROTOCOL: u32 = 1;

pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn hex_id(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stamp {
    pub boot_id: String,
    pub uptime_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub id: String,
    pub sha256: String,
    pub binding: String,
}

impl Snapshot {
    fn validate(&self) -> bool {
        hex_id(&self.id, 32) && hex_id(&self.sha256, 64) && hex_id(&self.binding, 64)
    }

    #[cfg(test)]
    pub fn fixture(id: &str, hash: &str) -> Self {
        Self {
            id: id.repeat(32),
            sha256: hash.repeat(64),
            binding: "f".repeat(64),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Armed,
    Trial,
    RollingBack,
    RestartPending,
    Confirmed,
    RolledBack,
    Conflict,
    RecoveryFailed,
}

impl Phase {
    pub fn pending(self) -> bool {
        !matches!(self, Self::Confirmed | Self::RolledBack)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestartWitness {
    pub stamp: Stamp,
    pub invocation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transaction {
    pub id: String,
    pub baseline: Snapshot,
    pub candidate: Snapshot,
    pub started: Stamp,
    pub deadline_uptime_ms: u64,
    pub phase: Phase,
    pub pre_apply_invocation: String,
    pub rollback_witness: Option<RestartWitness>,
    pub attempts: u8,
    pub last_error: Option<String>,
}

impl Transaction {
    pub fn new(
        id: String,
        baseline: Snapshot,
        candidate: Snapshot,
        stamp: &Stamp,
        seconds: u64,
        invocation: String,
    ) -> Result<Self, String> {
        if !(30..=300).contains(&seconds)
            || !hex_id(&id, 32)
            || baseline.binding != candidate.binding
        {
            return Err("保护事务参数或基线绑定无效".into());
        }
        let deadline = stamp
            .uptime_ms
            .checked_add(seconds * 1000)
            .ok_or("确认期限溢出")?;
        Ok(Self {
            id,
            baseline,
            candidate,
            started: stamp.clone(),
            deadline_uptime_ms: deadline,
            phase: Phase::Trial,
            pre_apply_invocation: invocation,
            rollback_witness: None,
            attempts: 0,
            last_error: None,
        })
    }

    pub fn due(&self, now: &Stamp) -> bool {
        now.boot_id != self.started.boot_id
            || now.uptime_ms < self.started.uptime_ms
            || now.uptime_ms >= self.deadline_uptime_ms
    }

    pub fn check_confirmation(&self, id: &str, now: &Stamp) -> Result<(), String> {
        if id != self.id || self.phase != Phase::Trial {
            return Err("事务 ID 或阶段不匹配，不能确认".into());
        }
        if self.due(now) {
            return Err("确认窗口已过期或主机已重启；不能取消恢复保护".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ledger {
    pub protocol: u32,
    pub checkpoint: Option<Snapshot>,
    pub transaction: Option<Transaction>,
}

impl Default for Ledger {
    fn default() -> Self {
        Self {
            protocol: PROTOCOL,
            checkpoint: None,
            transaction: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    Allow,
    Recover,
    Refuse,
}

impl Ledger {
    pub fn pending(&self) -> bool {
        self.transaction
            .as_ref()
            .is_some_and(|tx| tx.phase.pending())
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.protocol != PROTOCOL
            || self
                .checkpoint
                .as_ref()
                .is_some_and(|snapshot| !snapshot.validate())
        {
            return Err("恢复账本协议或基线格式不可识别".into());
        }
        if let Some(tx) = &self.transaction {
            let duration = tx.deadline_uptime_ms.checked_sub(tx.started.uptime_ms);
            if !hex_id(&tx.id, 32)
                || !tx.baseline.validate()
                || !tx.candidate.validate()
                || tx.baseline.binding != tx.candidate.binding
                || !duration.is_some_and(|duration| (30_000..=300_000).contains(&duration))
                || tx.started.boot_id.is_empty()
                || tx.started.boot_id.len() > 64
                || tx.pre_apply_invocation.is_empty()
                || tx.pre_apply_invocation.len() > 128
                || tx.attempts > 3
                || tx
                    .last_error
                    .as_ref()
                    .is_some_and(|error| error.len() > 4096)
                || tx.rollback_witness.as_ref().is_some_and(|witness| {
                    witness.stamp.boot_id.len() > 64
                        || witness.invocation.as_ref().is_some_and(|id| id.len() > 128)
                })
            {
                return Err("恢复事务元数据无效；未修改配置".into());
            }
            let checkpoint = if tx.phase == Phase::Confirmed {
                &tx.candidate
            } else {
                &tx.baseline
            };
            if self.checkpoint.as_ref() != Some(checkpoint) {
                return Err("恢复账本的基线引用不一致".into());
            }
        }
        Ok(())
    }

    pub fn startup_gate(&self, now: &Stamp, source_hash: Option<&str>, binding: &str) -> Gate {
        let Some(tx) = self.transaction.as_ref().filter(|tx| tx.phase.pending()) else {
            return Gate::Allow;
        };
        if binding != tx.baseline.binding || tx.phase == Phase::Conflict {
            return Gate::Refuse;
        }
        if matches!(
            tx.phase,
            Phase::RollingBack | Phase::RestartPending | Phase::RecoveryFailed
        ) {
            return if source_hash == Some(tx.baseline.sha256.as_str()) {
                Gate::Allow
            } else {
                Gate::Recover
            };
        }
        if tx.phase != Phase::Trial || tx.due(now) {
            return Gate::Recover;
        }
        if source_hash == Some(tx.candidate.sha256.as_str()) {
            Gate::Allow
        } else {
            Gate::Refuse
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trial() -> Transaction {
        Transaction::new(
            "a".repeat(32),
            Snapshot::fixture("b", "1"),
            Snapshot::fixture("c", "2"),
            &Stamp {
                boot_id: "boot-a".into(),
                uptime_ms: 10_000,
            },
            90,
            "invocation-before".into(),
        )
        .unwrap()
    }

    #[test]
    fn expiry_uses_boot_time_and_reboot_never_grants_another_confirmation_window() {
        let trial = trial();
        assert!(!trial.due(&Stamp {
            boot_id: "boot-a".into(),
            uptime_ms: 99_999
        }));
        assert!(trial.due(&Stamp {
            boot_id: "boot-a".into(),
            uptime_ms: 100_000
        }));
        assert!(trial.due(&Stamp {
            boot_id: "boot-b".into(),
            uptime_ms: 10
        }));
        assert!(trial.due(&Stamp {
            boot_id: "boot-a".into(),
            uptime_ms: 1
        }));
    }

    #[test]
    fn only_the_exact_trial_can_be_confirmed_before_the_deadline() {
        let mut trial = trial();
        let now = Stamp {
            boot_id: "boot-a".into(),
            uptime_ms: 20_000,
        };
        assert!(trial.check_confirmation(&trial.id, &now).is_ok());
        assert!(trial.check_confirmation(&"f".repeat(32), &now).is_err());
        trial.phase = Phase::RollingBack;
        assert!(trial.check_confirmation(&trial.id, &now).is_err());
    }

    #[test]
    fn startup_has_a_lock_free_path_for_idle_and_terminal_transactions() {
        let mut ledger = Ledger::default();
        let now = Stamp {
            boot_id: "boot-a".into(),
            uptime_ms: 20_000,
        };
        assert_eq!(ledger.startup_gate(&now, None, "unknown"), Gate::Allow);
        let mut trial = trial();
        trial.phase = Phase::Confirmed;
        ledger.transaction = Some(trial);
        assert_eq!(ledger.startup_gate(&now, None, "unknown"), Gate::Allow);
    }

    #[test]
    fn candidate_is_never_allowed_after_reboot_or_deadline() {
        let mut trial = trial();
        trial.phase = Phase::Trial;
        let ledger = Ledger {
            transaction: Some(trial.clone()),
            ..Ledger::default()
        };
        let now = Stamp {
            boot_id: "boot-b".into(),
            uptime_ms: 10,
        };
        assert_eq!(
            ledger.startup_gate(
                &now,
                Some(&trial.candidate.sha256),
                &trial.candidate.binding
            ),
            Gate::Recover
        );
    }
}
