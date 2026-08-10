mod install;
mod systemd;

#[cfg(test)]
mod tests;

use crate::system::{acquire_runtime_lock, effective_root};

pub use install::validate_config;
pub(crate) use systemd::{
    configure_managed_core_service, restart_service, validate_upgrade_service,
};
pub use systemd::{configure_workspace_service, reload_service};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeMode {
    External,
    ManagedLocal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inventory {
    pub binary: bool,
    pub unit: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    Prepare,
    Install,
    RejectPartialInstall,
}

pub fn plan(mode: RuntimeMode, inventory: Inventory, auto_install: bool) -> Action {
    if mode == RuntimeMode::External {
        return Action::None;
    }
    match (inventory.binary, inventory.unit) {
        (true, true) => Action::Prepare,
        (false, false) if auto_install => Action::Install,
        (false, false) => Action::None,
        _ => Action::RejectPartialInstall,
    }
}

pub fn ensure(mode: RuntimeMode, auto_install: bool) -> Result<(), String> {
    if mode == RuntimeMode::External {
        return Ok(());
    }
    let inventory = install::inspect()?;
    match plan(mode, inventory, auto_install) {
        Action::None => Ok(()),
        Action::RejectPartialInstall => Err(partial_install_error(inventory)),
        Action::Prepare | Action::Install => {
            if !effective_root() {
                return Err(root_required_message().into());
            }
            let _lock = acquire_runtime_lock()?;
            let inventory = install::inspect()?;
            match plan(mode, inventory, auto_install) {
                Action::None => Ok(()),
                Action::Prepare => install::prepare_existing_for_apply(),
                Action::Install => install::install_for_apply(),
                Action::RejectPartialInstall => Err(partial_install_error(inventory)),
            }
        }
    }
}

pub(crate) fn root_required_message() -> &'static str {
    "权限不足：自动安装或管理本机 Mihomo 需要 root 权限。\n请在原命令前添加 sudo 重新运行，例如：sudo mihomo-tui\n如果只连接已有实例，请使用 --controller。"
}

fn partial_install_error(inventory: Inventory) -> String {
    format!(
        "检测到不完整的 Mihomo 安装（binary={}，service={}），为避免覆盖现有文件已停止；请修复后重试或使用 --controller",
        inventory.binary, inventory.unit
    )
}
