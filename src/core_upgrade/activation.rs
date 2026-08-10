use crate::{
    core::CoreVersion,
    core_manager::{CorePaths, switch_current},
    mihomo::MihomoClient,
    runtime,
};
use std::{thread, time::Duration};

const HEALTH_ATTEMPTS: usize = 6;
const HEALTH_RETRY_DELAY: Duration = Duration::from_millis(250);

pub(super) trait ActivationOperations {
    fn switch(&mut self, version: CoreVersion) -> Result<(), String>;
    fn configure_service(&mut self) -> Result<(), String>;
    fn restart_service(&mut self) -> Result<(), String>;
    fn health(&mut self, expected: CoreVersion) -> Result<(), String>;
}

pub(super) fn activate_with_rollback(
    operations: &mut impl ActivationOperations,
    previous: CoreVersion,
    candidate: CoreVersion,
) -> Result<(), String> {
    let activate = activate_version(operations, candidate);
    let Err(activation_error) = activate else {
        return Ok(());
    };

    match activate_version(operations, previous) {
        Ok(()) => Err(format!(
            "managed core activation failed and was rolled back to {previous}: {activation_error}"
        )),
        Err(rollback_error) => Err(format!(
            "managed core activation failed: {activation_error}; rollback to {previous} also failed: {rollback_error}"
        )),
    }
}

fn activate_version(
    operations: &mut impl ActivationOperations,
    version: CoreVersion,
) -> Result<(), String> {
    operations.switch(version)?;
    operations.configure_service()?;
    operations.restart_service()?;
    operations.health(version)
}

pub(super) struct HostActivation {
    pub(super) paths: CorePaths,
    pub(super) client: MihomoClient,
}

impl ActivationOperations for HostActivation {
    fn switch(&mut self, version: CoreVersion) -> Result<(), String> {
        switch_current(&self.paths, version)
    }

    fn configure_service(&mut self) -> Result<(), String> {
        runtime::configure_managed_core_service()
    }

    fn restart_service(&mut self) -> Result<(), String> {
        runtime::restart_service()
    }

    fn health(&mut self, expected: CoreVersion) -> Result<(), String> {
        let mut last_error = "health check was not attempted".to_string();
        for attempt in 0..HEALTH_ATTEMPTS {
            match self.client.version() {
                Ok(actual) if actual == expected => match self.client.proxies() {
                    Ok(_) => return Ok(()),
                    Err(error) => last_error = format!("/proxies failed: {error}"),
                },
                Ok(actual) => {
                    last_error = format!("/version expected {expected}, received {actual}")
                }
                Err(error) => last_error = format!("/version failed: {error}"),
            }
            if attempt + 1 < HEALTH_ATTEMPTS {
                thread::sleep(HEALTH_RETRY_DELAY);
            }
        }
        Err(format!(
            "Mihomo controller did not become healthy after {HEALTH_ATTEMPTS} attempts: {last_error}"
        ))
    }
}
