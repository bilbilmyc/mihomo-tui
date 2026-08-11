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
use clap::{CommandFactory, Parser, Subcommand};
use crossterm::{
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{fmt::Display, io::stdout, path::PathBuf, process::ExitCode};

const COMMAND_HELP_TEMPLATE: &str =
    "{about-with-newline}\n用法：{usage}\n\n命令：\n{subcommands}\n选项：\n{options}\n{after-help}";
const LEAF_HELP_TEMPLATE: &str =
    "{about-with-newline}\n用法：{usage}\n\n选项：\n{options}\n{after-help}";

const TOP_LEVEL_AFTER_HELP: &str = r#"快速开始：
  sudo mihomo-tui
      使用 /etc/mihomo-tui/config.yaml 启动本机管理界面。
      在界面中完成配置后按 p 校验并启动或重载 Mihomo。

  mihomo-tui --controller http://127.0.0.1:9093
      只连接已有 Mihomo Controller，不安装或管理本机 systemd 服务。

常用内核命令：
  mihomo-tui core status          查看当前、已安装、推荐及兼容版本
  sudo mihomo-tui core upgrade   显式升级托管内核，失败时自动回滚

完整服务器说明：
  /usr/share/doc/mihomo-tui/server-guide.md
  源码仓库中的 docs/server-guide.md"#;

const CORE_AFTER_HELP: &str = r#"使用示例：
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
    help_template = COMMAND_HELP_TEMPLATE,
    override_usage = "mihomo-tui [选项] [命令]",
    disable_help_flag = true,
    disable_version_flag = true,
    disable_help_subcommand = true,
    next_line_help = true
)]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,
    /// 外部 Mihomo Controller 地址，例如 http://127.0.0.1:9093；也可通过 MIHOMO_CONTROLLER 设置。
    #[arg(long, env = "MIHOMO_CONTROLLER", hide_env = true, value_name = "地址")]
    controller: Option<String>,
    /// Mihomo API 密钥；在 shell 中优先使用 MIHOMO_SECRET 设置。
    #[arg(
        long,
        env = "MIHOMO_SECRET",
        hide_env = true,
        hide_env_values = true,
        value_name = "密钥"
    )]
    secret: Option<String>,
    /// 仅在独立配置不存在时导入的旧 Mihomo 配置；也可通过 MIHOMO_CONFIG 设置。
    #[arg(long, env = "MIHOMO_CONFIG", hide_env = true, value_name = "路径")]
    config: Option<PathBuf>,
    /// mihomo-tui 管理的单一原生配置路径；也可通过 MIHOMO_TUI_CONFIG 设置。
    #[arg(long, env = "MIHOMO_TUI_CONFIG", hide_env = true, value_name = "路径")]
    workspace: Option<PathBuf>,
    /// 应用配置时不自动下载缺失的 Mihomo。
    #[arg(long)]
    no_auto_install: bool,
    #[arg(
        short = 'h',
        long = "help",
        action = clap::ArgAction::Help,
        global = true,
        help = "显示帮助"
    )]
    help: Option<bool>,
    #[arg(
        short = 'V',
        long = "version",
        action = clap::ArgAction::Version,
        help = "显示版本"
    )]
    version: Option<bool>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 检查或显式升级托管 Mihomo 内核。
    #[command(
        after_help = CORE_AFTER_HELP,
        help_template = COMMAND_HELP_TEMPLATE,
        override_usage = "mihomo-tui core <命令>"
    )]
    Core {
        #[command(subcommand)]
        command: CoreCommand,
    },
    /// 显示顶层或指定命令的帮助。
    #[command(help_template = LEAF_HELP_TEMPLATE)]
    Help {
        /// 命令路径，例如 core status。
        #[arg(value_name = "命令", num_args = 0..)]
        command: Vec<String>,
    },
}

#[derive(Debug, Subcommand)]
enum CoreCommand {
    /// 查看已安装、当前、推荐及兼容的内核版本。
    #[command(after_help = CORE_STATUS_AFTER_HELP, help_template = LEAF_HELP_TEMPLATE)]
    Status,
    /// 显式暂存、激活并健康检查内核更新，失败时回滚。
    #[command(after_help = CORE_UPGRADE_AFTER_HELP, help_template = LEAF_HELP_TEMPLATE)]
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
        Command::Help { command } => {
            println!("{}", command_help(&command).map_err(std::io::Error::other)?);
            Ok(())
        }
    }
}

fn command_help(path: &[String]) -> Result<String, String> {
    let mut root = Args::command();
    root.build();
    let mut selected = &mut root;
    for name in path {
        selected = selected
            .find_subcommand_mut(name)
            .ok_or_else(|| format!("未知命令路径：{}", path.join(" ")))?;
    }
    Ok(selected.render_long_help().to_string())
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
    fn help_subcommand_accepts_nested_command_paths() {
        for arguments in [
            vec!["mihomo-tui", "help"],
            vec!["mihomo-tui", "help", "core"],
            vec!["mihomo-tui", "help", "core", "status"],
        ] {
            assert!(
                Args::try_parse_from(arguments.clone()).is_ok(),
                "help subcommand rejected {arguments:?}"
            );
        }

        let help = command_help(&["core".into(), "status".into()]).unwrap();
        assert!(help.contains("查看已安装、当前、推荐及兼容的内核版本"));
        assert!(help.contains("用法：mihomo-tui core status"));
        assert!(!help.contains("Usage:"));
        assert!(!help.contains("Print help"));
    }

    #[test]
    fn every_help_screen_uses_chinese_headings_and_descriptions() {
        for arguments in [
            vec!["mihomo-tui", "-h"],
            vec!["mihomo-tui", "core", "-h"],
            vec!["mihomo-tui", "core", "status", "-h"],
            vec!["mihomo-tui", "core", "upgrade", "-h"],
        ] {
            let error = Args::try_parse_from(arguments.clone()).unwrap_err();

            assert_eq!(error.kind(), clap::error::ErrorKind::DisplayHelp);
            let help = error.to_string();
            assert!(
                help.contains("用法："),
                "help has no Chinese usage heading:\n{help}"
            );
            assert!(
                help.contains("选项："),
                "help has no Chinese options heading:\n{help}"
            );
            assert!(
                help.contains("显示帮助"),
                "help has no Chinese help description:\n{help}"
            );
            for english in [
                "Usage:",
                "Commands:",
                "Options:",
                "Print help",
                "Print version",
                "Print this message",
                "[OPTIONS]",
                "[COMMAND]",
                "<COMMAND>",
            ] {
                assert!(
                    !help.contains(english),
                    "help for {arguments:?} still contains {english:?}:\n{help}"
                );
            }
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

    #[test]
    fn github_ci_keeps_the_complete_linux_release_contract() {
        let workflow = include_str!("../.github/workflows/ci.yml");
        let document: serde_yaml::Value = serde_yaml::from_str(workflow).unwrap();
        let jobs = document
            .get("jobs")
            .and_then(|jobs| jobs.as_mapping())
            .unwrap();

        for job in [
            "quality",
            "security-audit",
            "packages",
            "prepare-draft-release",
            "release-packages",
            "verify-draft-release",
        ] {
            assert!(
                jobs.contains_key(serde_yaml::Value::from(job)),
                "CI is missing the {job} job"
            );
        }
        for required in [
            "ubuntu-24.04-arm",
            "aarch64",
            "arm64",
            "refs/tags/v",
            "--draft",
            "refusing to replace assets on a published release",
            "gh release upload",
            "sha256sum --check -- *.sha256",
        ] {
            assert!(workflow.contains(required), "CI is missing {required}");
        }
        for forbidden in ["actions/upload-artifact@", "actions/download-artifact@"] {
            assert!(
                !workflow.contains(forbidden),
                "CI must not depend on quota-limited {forbidden}"
            );
        }

        let package_action = include_str!("../.github/actions/build-linux-packages/action.yml");
        let _: serde_yaml::Value = serde_yaml::from_str(package_action).unwrap();
        for builder in ["build-native.sh", "build-deb.sh", "build-rpm.sh"] {
            assert!(
                package_action.contains(builder),
                "package action is missing {builder}"
            );
        }
        for verifier in [
            "test-native-binary.sh",
            "test-deb-package.sh",
            "test-rpm-package.sh",
            "test-install-deb.sh",
            "test-install-rpm.sh",
        ] {
            assert!(
                package_action.contains(verifier),
                "package action is missing {verifier}"
            );
        }
    }
}
