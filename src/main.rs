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
    /// Mihomo runtime config imported on first use and written by explicit apply.
    #[arg(long, env = "MIHOMO_CONFIG")]
    config: Option<PathBuf>,
    /// Independent mihomo-tui configuration source.
    #[arg(long, env = "MIHOMO_TUI_CONFIG")]
    workspace: Option<PathBuf>,
    /// Do not download Mihomo during explicit apply.
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
    let source_path = args
        .workspace
        .unwrap_or_else(|| PathBuf::from(workspace::DEFAULT_SOURCE_PATH));
    let runtime_path = args
        .config
        .unwrap_or_else(|| PathBuf::from(workspace::DEFAULT_RUNTIME_PATH));
    if config_was_explicit && std::fs::symlink_metadata(&runtime_path).is_err() {
        return Err(std::io::Error::other(format!(
            "显式 Mihomo 运行配置不存在：{}",
            runtime_path.display()
        ))
        .into());
    }
    workspace::initialize(&source_path, Some(&runtime_path)).map_err(std::io::Error::other)?;
    let discovered = discovery::discover(Some(&source_path));
    let controller = args
        .controller
        .or_else(|| discovered.as_ref().map(|info| info.controller.clone()));
    let secret = args.secret.or_else(|| {
        (!controller_was_explicit)
            .then(|| discovered.as_ref().and_then(|info| info.secret.clone()))
            .flatten()
    });
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;
    let config_reload = apply_policy(controller_was_explicit, config_was_explicit);
    let result = App::with_workspace(
        controller,
        secret,
        Some(source_path),
        Some(runtime_path),
        config_reload,
        !args.no_auto_install,
    )
    .run(&mut terminal);
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

fn render_fatal_error(error: impl Display) -> String {
    format!("mihomo-tui: {error}")
}

fn apply_policy(controller_was_explicit: bool, config_was_explicit: bool) -> ConfigReload {
    if controller_was_explicit || config_was_explicit {
        ConfigReload::None
    } else {
        ConfigReload::LocalSystemd
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_controller_or_config_never_manages_the_local_service() {
        assert_eq!(apply_policy(true, false), ConfigReload::None);
        assert_eq!(apply_policy(false, true), ConfigReload::None);
        assert_eq!(apply_policy(true, true), ConfigReload::None);
    }

    #[test]
    fn default_runtime_target_enables_local_apply() {
        assert_eq!(apply_policy(false, false), ConfigReload::LocalSystemd);
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
