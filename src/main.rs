mod app;
mod config;
mod core;
mod core_manager;
mod core_package;
mod core_upgrade;
mod dialogs;
mod discovery;
mod mihomo;
mod models;
mod runtime;
mod system;
mod workspace;

use app::{App, ConfigReload};
use clap::{Parser, Subcommand};
use crossterm::{
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{fmt::Display, io::stdout, path::PathBuf, process::ExitCode};

const TOP_LEVEL_AFTER_HELP: &str = r#"快速开始:
  sudo mihomo-tui
      使用 /etc/mihomo-tui/config.yaml 启动本机管理界面。
      在界面中完成配置后按 p 校验并启动或重载 Mihomo。

  mihomo-tui --controller http://127.0.0.1:9093
      只连接已有 Mihomo Controller，不安装或管理本机 systemd 服务。

常用内核命令:
  mihomo-tui core status          查看当前、已安装、推荐及兼容版本
  sudo mihomo-tui core upgrade   显式升级托管内核，失败时自动回滚

完整服务器说明:
  /usr/share/doc/mihomo-tui/server-guide.md
  源码仓库中的 docs/server-guide.md"#;

const CORE_AFTER_HELP: &str = r#"使用示例:
  mihomo-tui core status
      只读检查托管内核状态，不修改系统。

  sudo mihomo-tui core upgrade
      校验候选内核和配置，切换后执行健康检查，失败时自动回滚。"#;

const CORE_STATUS_AFTER_HELP: &str = r#"该命令是只读操作，不会下载、切换、启动或重载 Mihomo。"#;

const CORE_UPGRADE_AFTER_HELP: &str = r#"升级要求 root 权限、受信任的托管配置和已安装的 mihomo.service。
升级是显式事务：候选校验 -> 原子切换 -> 服务重启 -> API 健康检查；失败时自动回滚。"#;

#[derive(Debug, Parser)]
#[command(
    name = "mihomo-tui",
    version,
    about = "通过 SSH 管理 Mihomo 的终端控制中心",
    long_about = "mihomo-tui 是面向 Linux 服务器的 Mihomo 终端控制中心。\n不带子命令时启动 TUI；也可以只连接已有 Controller，或显式检查和升级托管内核。",
    after_help = TOP_LEVEL_AFTER_HELP,
    next_line_help = true
)]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,
    /// 外部 Mihomo Controller 地址，例如 http://127.0.0.1:9093。
    #[arg(long, env = "MIHOMO_CONTROLLER")]
    controller: Option<String>,
    /// Mihomo API 密钥；在 shell 中优先使用 MIHOMO_SECRET。
    #[arg(long, env = "MIHOMO_SECRET", hide_env_values = true)]
    secret: Option<String>,
    /// 仅在独立配置不存在时导入的旧 Mihomo 配置。
    #[arg(long, env = "MIHOMO_CONFIG")]
    config: Option<PathBuf>,
    /// mihomo-tui 管理的单一原生配置路径。
    #[arg(long, env = "MIHOMO_TUI_CONFIG")]
    workspace: Option<PathBuf>,
    /// 应用配置时不自动下载缺失的 Mihomo。
    #[arg(long)]
    no_auto_install: bool,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 检查或显式升级托管 Mihomo 内核。
    #[command(after_help = CORE_AFTER_HELP)]
    Core {
        #[command(subcommand)]
        command: CoreCommand,
    },
}

#[derive(Debug, Subcommand)]
enum CoreCommand {
    /// 查看已安装、当前、推荐及兼容的内核版本。
    #[command(after_help = CORE_STATUS_AFTER_HELP)]
    Status,
    /// 显式暂存、激活并健康检查内核更新，失败时回滚。
    #[command(after_help = CORE_UPGRADE_AFTER_HELP)]
    Upgrade,
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
    if let Some(command) = args.command {
        return run_command(command);
    }
    let controller_was_explicit = args.controller.is_some();
    let config_was_explicit = args.config.is_some();
    let workspace_was_explicit = args.workspace.is_some();
    let source_path = args
        .workspace
        .unwrap_or_else(|| PathBuf::from(workspace::DEFAULT_SOURCE_PATH));
    let import_path = args
        .config
        .unwrap_or_else(|| PathBuf::from(workspace::DEFAULT_IMPORT_PATH));
    if config_was_explicit
        && std::fs::symlink_metadata(&source_path).is_err()
        && std::fs::symlink_metadata(&import_path).is_err()
    {
        return Err(std::io::Error::other(format!(
            "显式 Mihomo 待导入配置不存在：{}",
            import_path.display()
        ))
        .into());
    }
    workspace::initialize(&source_path, Some(&import_path)).map_err(std::io::Error::other)?;
    let discovered = discovery::discover(&source_path);
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
    let config_reload = apply_policy(
        controller_was_explicit,
        config_was_explicit,
        workspace_was_explicit,
    );
    let result = App::with_workspace(
        controller,
        secret,
        Some(source_path),
        config_reload,
        !args.no_auto_install,
    )
    .run(&mut terminal);
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

fn run_command(command: Command) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        Command::Core {
            command: CoreCommand::Status,
        } => {
            println!("{}", core_manager::status().map_err(std::io::Error::other)?);
            Ok(())
        }
        Command::Core {
            command: CoreCommand::Upgrade,
        } => {
            println!(
                "{}",
                core_upgrade::upgrade().map_err(std::io::Error::other)?
            );
            Ok(())
        }
    }
}

fn render_fatal_error(error: impl Display) -> String {
    format!("mihomo-tui: {error}")
}

fn apply_policy(
    controller_was_explicit: bool,
    config_was_explicit: bool,
    workspace_was_explicit: bool,
) -> ConfigReload {
    if controller_was_explicit || config_was_explicit || workspace_was_explicit {
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
        assert_eq!(apply_policy(true, false, false), ConfigReload::None);
        assert_eq!(apply_policy(false, true, false), ConfigReload::None);
        assert_eq!(apply_policy(true, true, false), ConfigReload::None);
        assert_eq!(apply_policy(false, false, true), ConfigReload::None);
    }

    #[test]
    fn default_owned_config_enables_local_reload() {
        assert_eq!(
            apply_policy(false, false, false),
            ConfigReload::LocalSystemd
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

    #[test]
    fn core_status_is_an_explicit_cli_subcommand() {
        let args = Args::try_parse_from(["mihomo-tui", "core", "status"]);

        assert!(matches!(
            args.unwrap().command,
            Some(Command::Core {
                command: CoreCommand::Status
            })
        ));
    }

    #[test]
    fn core_upgrade_is_an_explicit_cli_subcommand() {
        let args = Args::try_parse_from(["mihomo-tui", "core", "upgrade"]);

        assert!(matches!(
            args.unwrap().command,
            Some(Command::Core {
                command: CoreCommand::Upgrade
            })
        ));
    }

    #[test]
    fn top_level_help_explains_the_server_quick_start_and_modes() {
        let error = Args::try_parse_from(["mihomo-tui", "-h"]).unwrap_err();

        assert_eq!(error.kind(), clap::error::ErrorKind::DisplayHelp);
        let help = error.to_string();
        for expected in [
            "快速开始",
            "sudo mihomo-tui",
            "/etc/mihomo-tui/config.yaml",
            "--controller",
            "/usr/share/doc/mihomo-tui/server-guide.md",
        ] {
            assert!(
                help.contains(expected),
                "help is missing {expected:?}:\n{help}"
            );
        }
    }

    #[test]
    fn core_help_explains_inspection_and_transactional_upgrade() {
        let error = Args::try_parse_from(["mihomo-tui", "core", "-h"]).unwrap_err();

        assert_eq!(error.kind(), clap::error::ErrorKind::DisplayHelp);
        let help = error.to_string();
        for expected in ["core status", "core upgrade", "健康检查", "自动回滚"] {
            assert!(
                help.contains(expected),
                "help is missing {expected:?}:\n{help}"
            );
        }
    }

    #[test]
    fn github_workflows_are_valid_yaml_with_explicit_jobs() {
        for (name, workflow) in [
            ("ci", include_str!("../.github/workflows/ci.yml")),
            (
                "managed-core-sync",
                include_str!("../.github/workflows/managed-core-sync.yml"),
            ),
        ] {
            let document: serde_yaml::Value = serde_yaml::from_str(workflow).unwrap();

            assert!(document.get("on").is_some(), "{name} has no trigger");
            assert!(document.get("jobs").is_some(), "{name} has no jobs");
            assert!(
                document.get("permissions").is_some(),
                "{name} has no explicit permissions"
            );
        }
    }
}
