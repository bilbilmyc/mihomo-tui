mod app;
mod config;
mod dialogs;
mod discovery;
mod mihomo;
mod models;

use app::App;
use clap::Parser;
use crossterm::{
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{
    io::stdout,
    path::PathBuf,
    process::{Command, Stdio},
};

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
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let controller_was_explicit = args.controller.is_some();
    let discovered = discovery::discover(args.config.as_deref());
    let controller = args
        .controller
        .or_else(|| discovered.as_ref().map(|info| info.controller.clone()));
    let secret = args
        .secret
        .or_else(|| discovered.as_ref().and_then(|info| info.secret.clone()));
    let config_path = args
        .config
        .or_else(|| discovered.as_ref().map(|info| info.config_path.clone()));
    if should_start_local_service(controller_was_explicit, controller.as_deref()) {
        ensure_mihomo_background_service();
    }
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;
    let result = App::new(controller, secret, config_path).run(&mut terminal);
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

fn ensure_mihomo_background_service() {
    let _ = Command::new("systemctl")
        .args(["start", "mihomo"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn should_start_local_service(controller_was_explicit: bool, controller: Option<&str>) -> bool {
    if controller_was_explicit {
        return false;
    }
    let Some(controller) = controller else {
        return false;
    };
    let Ok(url) = reqwest::Url::parse(controller) else {
        return false;
    };
    matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_auto_starts_a_discovered_local_controller() {
        assert!(should_start_local_service(
            false,
            Some("http://127.0.0.1:9090")
        ));
        assert!(!should_start_local_service(
            true,
            Some("http://127.0.0.1:9090")
        ));
        assert!(!should_start_local_service(
            false,
            Some("https://mihomo.example.com")
        ));
        assert!(!should_start_local_service(false, None));
    }
}
