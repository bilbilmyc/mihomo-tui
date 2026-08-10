mod app;
mod config;
mod dialogs;
mod discovery;
mod mihomo;
mod models;
mod runtime;
mod workspace;

use app::{App, ConfigReload};
use clap::Parser;
use crossterm::{
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{fmt::Display, io::stdout, path::PathBuf, process::ExitCode};

#[derive(Debug, Parser)]
#[command(
    name = "mihomo-tui",
    version,
    about = "A terminal control center for Mihomo"
)]
struct Args {
    /// Mihomo external controller URL, for example http://127.0.0.1:9093
    #[arg(long, env = "MIHOMO_CONTROLLER")]
    controller: Option<String>,
    /// Mihomo API secret. Prefer MIHOMO_SECRET in shell environments.
    #[arg(long, env = "MIHOMO_SECRET", hide_env_values = true)]
    secret: Option<String>,
    /// Mihomo config file used to auto-discover the controller and secret.
    #[arg(long, env = "MIHOMO_CONFIG")]
    config: Option<PathBuf>,
    /// Do not download Mihomo when no local installation exists.
    #[arg(long)]
    no_auto_install: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{}", render_fatal_error(error));
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let controller_was_explicit = args.controller.is_some();
    let config_was_explicit = args.config.is_some();
    let mut discovered = should_discover_config(controller_was_explicit, config_was_explicit)
        .then(|| discovery::discover(args.config.as_deref()))
        .flatten();
    if config_was_explicit && discovered.is_none() {
        let path = args
            .config
            .as_deref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "<unknown>".into());
        return Err(std::io::Error::other(format!(
            "无法从显式 Mihomo 配置 {path} 读取 external-controller"
        ))
        .into());
    }
    let mode = runtime_mode(
        controller_was_explicit,
        config_was_explicit,
        discovered.as_ref().map(|info| info.origin),
        discovery::user_config_exists(),
    );
    runtime::ensure(mode, !args.no_auto_install).map_err(std::io::Error::other)?;
    if mode == runtime::RuntimeMode::ManagedLocal {
        discovered = discovery::discover(args.config.as_deref());
    }
    if mode == runtime::RuntimeMode::ManagedLocal && discovered.is_none() && !args.no_auto_install {
        return Err(std::io::Error::other(
            "系统 Mihomo 配置没有可读取的 external-controller；请修复 /etc/mihomo/config.yaml 或使用 --controller",
        )
        .into());
    }
    let controller = args
        .controller
        .or_else(|| discovered.as_ref().map(|info| info.controller.clone()));
    let secret = args
        .secret
        .or_else(|| discovered.as_ref().and_then(|info| info.secret.clone()));
    let config_path = args
        .config
        .or_else(|| discovered.as_ref().map(|info| info.config_path.clone()));
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;
    let config_reload = match mode {
        runtime::RuntimeMode::External => ConfigReload::None,
        runtime::RuntimeMode::ManagedLocal => ConfigReload::LocalSystemd,
    };
    let result =
        App::with_config_reload(controller, secret, config_path, config_reload).run(&mut terminal);
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

fn render_fatal_error(error: impl Display) -> String {
    format!("mihomo-tui: {error}")
}

fn should_discover_config(controller_was_explicit: bool, config_was_explicit: bool) -> bool {
    !controller_was_explicit || config_was_explicit
}

fn runtime_mode(
    controller_was_explicit: bool,
    config_was_explicit: bool,
    origin: Option<discovery::ConfigOrigin>,
    user_config_exists: bool,
) -> runtime::RuntimeMode {
    if controller_was_explicit || config_was_explicit {
        return runtime::RuntimeMode::External;
    }
    match origin {
        Some(discovery::ConfigOrigin::System) => runtime::RuntimeMode::ManagedLocal,
        Some(discovery::ConfigOrigin::Explicit | discovery::ConfigOrigin::User) => {
            runtime::RuntimeMode::External
        }
        None if user_config_exists => runtime::RuntimeMode::External,
        None => runtime::RuntimeMode::ManagedLocal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_and_user_connections_never_manage_the_local_service() {
        assert_eq!(
            runtime_mode(true, false, None, false),
            runtime::RuntimeMode::External
        );
        assert_eq!(
            runtime_mode(false, true, None, false),
            runtime::RuntimeMode::External
        );
        assert_eq!(
            runtime_mode(false, false, Some(discovery::ConfigOrigin::User), true),
            runtime::RuntimeMode::External
        );
        assert_eq!(
            runtime_mode(false, false, None, true),
            runtime::RuntimeMode::External
        );
    }

    #[test]
    fn explicit_remote_controller_does_not_discover_local_credentials() {
        assert!(!should_discover_config(true, false));
        assert!(should_discover_config(true, true));
        assert!(should_discover_config(false, false));
    }

    #[test]
    fn system_or_clean_local_state_is_managed() {
        assert_eq!(
            runtime_mode(false, false, Some(discovery::ConfigOrigin::System), false),
            runtime::RuntimeMode::ManagedLocal
        );
        assert_eq!(
            runtime_mode(false, false, None, false),
            runtime::RuntimeMode::ManagedLocal
        );
    }

    #[test]
    fn root_permission_failure_has_an_actionable_cli_message() {
        let output = render_fatal_error(runtime::root_required_message());

        assert!(output.starts_with("mihomo-tui: 权限不足"));
        assert!(output.contains("root"));
        assert!(output.contains("sudo mihomo-tui"));
        assert!(!output.contains("--no-auto-install"));
        assert!(!output.contains("Custom"));
    }
}
