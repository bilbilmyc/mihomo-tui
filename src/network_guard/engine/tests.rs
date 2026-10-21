use super::super::{
    model::{Phase, RestartWitness, Stamp},
    store::{self, Store},
};
use super::*;
use std::path::{Path, PathBuf};

pub(super) struct Fake {
    pub now: Stamp,
    pub binding: String,
    pub binding_error: Option<BindingError>,
    pub state: Service,
    pub restarts: usize,
    pub armed: bool,
    pub fail_arm: bool,
    pub fail_restart: bool,
}

impl Backend for Fake {
    fn stamp(&mut self) -> Result<Stamp, String> {
        Ok(self.now.clone())
    }
    fn binding(&mut self) -> Result<String, BindingError> {
        if let Some(error) = &self.binding_error {
            return Err(error.clone());
        }
        Ok(self.binding.clone())
    }
    fn service(&mut self) -> Result<Service, String> {
        Ok(self.state.clone())
    }
    fn validate(&mut self, _: &Path) -> Result<(), String> {
        Ok(())
    }
    fn arm(&mut self, _: bool) -> Result<(), String> {
        if self.fail_arm {
            Err("fixture arm failure".into())
        } else {
            self.armed = true;
            Ok(())
        }
    }
    fn disarm(&mut self) -> Result<(), String> {
        self.armed = false;
        Ok(())
    }
    fn restart(&mut self) -> Result<(), String> {
        assert!(self.armed);
        self.restarts += 1;
        if self.fail_restart {
            return Err("fixture restart failure".into());
        }
        self.state.invocation = Some(format!("{:032x}", self.restarts));
        self.state.active = true;
        self.state.starting = false;
        self.state.job = false;
        Ok(())
    }
}

struct Fixture {
    root: PathBuf,
    engine: Engine<Fake>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

fn fixture() -> Fixture {
    let root = store::tests::directory();
    let source = root.join("config.yaml");
    store::write_new(&source, b"rules: ['MATCH,DIRECT']\ntun: {enable: false}\n").unwrap();
    let backend = Fake {
        now: Stamp {
            boot_id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into(),
            uptime_ms: 10_000,
        },
        binding: "f".repeat(64),
        binding_error: None,
        state: Service {
            active: true,
            starting: false,
            enabled: true,
            job: false,
            invocation: Some("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".into()),
        },
        restarts: 0,
        armed: false,
        fail_arm: false,
        fail_restart: false,
    };
    let store = Store::initialize(&root.join("state")).unwrap();
    let mut engine = Engine {
        store,
        source,
        host: backend,
    };
    engine.checkpoint().unwrap();
    std::fs::write(
        &engine.source,
        b"rules: ['MATCH,DIRECT']\ntun: {enable: true}\n",
    )
    .unwrap();
    Fixture { root, engine }
}

#[test]
fn a_trial_is_armed_before_restart_and_confirmation_atomically_promotes_its_snapshot() {
    let mut f = fixture();
    let ledger = f.engine.apply(90).unwrap();
    let tx = ledger.transaction.unwrap();
    assert_eq!(tx.phase, Phase::Trial);
    assert!(f.engine.host.armed);
    let confirmed = f.engine.confirm(&tx.id).unwrap();
    assert_eq!(confirmed.transaction.unwrap().phase, Phase::Confirmed);
    assert_eq!(confirmed.checkpoint, Some(tx.candidate));
    assert!(!f.engine.host.armed);
}

#[test]
fn arming_failure_or_changed_core_cannot_restart_the_service() {
    let mut f = fixture();
    f.engine.host.fail_arm = true;
    assert!(f.engine.apply(90).is_err());
    assert_eq!(f.engine.host.restarts, 0);
    assert!(!f.engine.store.load().unwrap().pending());
    f.engine.host.fail_arm = false;
    f.engine.host.binding = "e".repeat(64);
    assert!(f.engine.apply(90).is_err());
    assert_eq!(f.engine.host.restarts, 0);
}

#[test]
fn an_old_active_invocation_cannot_confirm_a_new_candidate() {
    let mut f = fixture();
    let tx = f.engine.apply(90).unwrap().transaction.unwrap();
    f.engine.host.state.invocation = Some(tx.pre_apply_invocation.clone());
    assert!(f.engine.confirm(&tx.id).is_err());
    assert_eq!(
        f.engine.store.load().unwrap().transaction.unwrap().phase,
        Phase::Trial
    );
    assert_eq!(f.engine.store.load().unwrap().checkpoint, Some(tx.baseline));
}

#[test]
fn expired_confirmation_is_rejected_and_recovery_verifies_a_new_baseline_activation() {
    let mut f = fixture();
    let tx = f.engine.apply(90).unwrap().transaction.unwrap();
    f.engine.host.now.uptime_ms = tx.deadline_uptime_ms;
    assert!(f.engine.confirm(&tx.id).is_err());
    let ledger = f.engine.recover(None, false, false).unwrap();
    assert_eq!(ledger.transaction.unwrap().phase, Phase::RestartPending);
    assert!(
        String::from_utf8(std::fs::read(&f.engine.source).unwrap())
            .unwrap()
            .contains("enable: false")
    );
    let ledger = f.engine.recover(None, false, false).unwrap();
    assert_eq!(ledger.transaction.unwrap().phase, Phase::RolledBack);
}

#[test]
fn interrupted_file_restore_cannot_mistake_the_still_running_candidate_for_rollback_success() {
    let mut f = fixture();
    let mut ledger = f.engine.apply(90).unwrap();
    let tx = ledger.transaction.as_mut().unwrap();
    tx.phase = Phase::RollingBack;
    super::super::restore::install(&f.engine.source, &f.engine.store, tx).unwrap();
    f.engine.store.save(&ledger).unwrap();
    let ledger = f.engine.recover(None, false, false).unwrap();
    assert_eq!(ledger.transaction.unwrap().phase, Phase::RestartPending);
    assert_eq!(f.engine.host.restarts, 2);
}

#[test]
fn a_restart_intent_survives_a_crash_before_the_restart_was_queued() {
    let mut f = fixture();
    let mut ledger = f.engine.apply(90).unwrap();
    let tx = ledger.transaction.as_mut().unwrap();
    tx.phase = Phase::RestartPending;
    tx.rollback_witness = Some(RestartWitness {
        stamp: f.engine.host.now.clone(),
        invocation: f.engine.host.state.invocation.clone(),
        same_activation: false,
    });
    super::super::restore::install(&f.engine.source, &f.engine.store, tx).unwrap();
    f.engine.store.save(&ledger).unwrap();
    let ledger = f.engine.recover(None, false, false).unwrap();
    assert_eq!(ledger.transaction.unwrap().phase, Phase::RestartPending);
    assert_eq!(f.engine.host.restarts, 2);
}

#[test]
fn boot_recovery_installs_before_start_and_does_not_restart_its_own_unit() {
    let mut f = fixture();
    f.engine.apply(90).unwrap();
    f.engine.host.now = Stamp {
        boot_id: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".into(),
        uptime_ms: 1000,
    };
    f.engine.host.state.active = false;
    f.engine.host.state.starting = true;
    f.engine.host.state.invocation = Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into());
    let ledger = f.engine.recover(None, false, true).unwrap();
    assert_eq!(ledger.transaction.unwrap().phase, Phase::RestartPending);
    assert_eq!(f.engine.host.restarts, 1);
    f.engine.host.state.active = true;
    f.engine.host.state.starting = false;
    let ledger = f.engine.recover(None, false, false).unwrap();
    assert_eq!(ledger.transaction.unwrap().phase, Phase::RolledBack);
    assert_eq!(f.engine.host.restarts, 1);
}

#[test]
fn external_edits_conflict_and_recovery_restarts_have_a_finite_budget() {
    let mut f = fixture();
    let tx = f.engine.apply(90).unwrap().transaction.unwrap();
    std::fs::write(&f.engine.source, b"later external edit").unwrap();
    f.engine.host.now.uptime_ms = tx.deadline_uptime_ms;
    assert!(f.engine.recover(None, false, false).is_err());
    assert_eq!(
        f.engine.store.load().unwrap().transaction.unwrap().phase,
        Phase::Conflict
    );
    assert_eq!(
        std::fs::read(&f.engine.source).unwrap(),
        b"later external edit"
    );
    let mut g = fixture();
    let tx = g.engine.apply(90).unwrap().transaction.unwrap();
    g.engine.host.now.uptime_ms = tx.deadline_uptime_ms;
    g.engine.host.fail_restart = true;
    for _ in 0..6 {
        let _ = g.engine.recover(None, false, false);
    }
    let ledger = g.engine.store.load().unwrap();
    assert_eq!(ledger.transaction.unwrap().phase, Phase::RecoveryFailed);
    assert_eq!(g.engine.host.restarts, 4);
}

#[test]
fn old_confirmation_and_old_timer_wakeup_cannot_change_a_later_trial() {
    let mut f = fixture();
    let first = f.engine.apply(90).unwrap().transaction.unwrap();
    f.engine.confirm(&first.id).unwrap();
    f.engine.host.now.uptime_ms = first.deadline_uptime_ms;
    std::fs::write(
        &f.engine.source,
        b"rules: ['MATCH,DIRECT']\ntun: {enable: true}\ndns: {enable: true}\n",
    )
    .unwrap();
    let second = f.engine.apply(90).unwrap().transaction.unwrap();
    assert!(f.engine.confirm(&first.id).is_err());
    let current = f
        .engine
        .recover(None, false, false)
        .unwrap()
        .transaction
        .unwrap();
    assert_eq!(current.id, second.id);
    assert_eq!(current.phase, Phase::Trial);
}

#[test]
fn unsupported_binding_inspection_is_persisted_as_conflict_and_transient_failures_are_bounded() {
    let mut f = fixture();
    let tx = f.engine.apply(90).unwrap().transaction.unwrap();
    f.engine.host.now.uptime_ms = tx.deadline_uptime_ms;
    f.engine.host.binding_error = Some(BindingError::Conflict("unknown service override".into()));
    assert!(f.engine.recover(None, false, false).is_err());
    assert_eq!(
        f.engine.store.load().unwrap().transaction.unwrap().phase,
        Phase::Conflict
    );
    assert_eq!(f.engine.host.restarts, 1);
    let mut g = fixture();
    let tx = g.engine.apply(90).unwrap().transaction.unwrap();
    g.engine.host.now.uptime_ms = tx.deadline_uptime_ms;
    g.engine.host.binding_error = Some(BindingError::Unavailable(
        "temporary inspection failure".into(),
    ));
    for _ in 0..4 {
        let _ = g.engine.recover(None, false, false);
    }
    let state = g.engine.store.load().unwrap().transaction.unwrap();
    assert_eq!(state.phase, Phase::RecoveryFailed);
    assert_eq!(state.attempts, 3);
    assert_eq!(g.engine.host.restarts, 1);
}
