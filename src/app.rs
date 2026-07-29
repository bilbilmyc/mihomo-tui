use crate::config::{self, ConfigSnapshot};
use crate::mihomo::MihomoClient;
use crate::models::{AppState, Page, Profile, Rule, RuleAction, RuleSet};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{prelude::*, widgets::*};
use std::{
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};

pub struct App {
    pub state: AppState,
    client: Option<MihomoClient>,
    last_refresh: Instant,
    proxy_members_focused: bool,
    selected_proxy_member_index: usize,
    config: ConfigSnapshot,
    config_path: Option<PathBuf>,
    add_provider: Option<AddProviderDialog>,
}

struct AddProviderDialog {
    name: String,
    url: String,
    editing_url: bool,
}

impl App {
    pub fn new(
        controller: Option<String>,
        secret: Option<String>,
        config_path: Option<PathBuf>,
    ) -> Self {
        let mut state = AppState::demo();
        let config = config_path
            .as_deref()
            .map(config::load)
            .transpose()
            .unwrap_or_else(|error| {
                state.status = format!("Config read error: {error}");
                None
            })
            .unwrap_or_default();
        apply_config(&mut state, &config);
        let client = controller
            .as_ref()
            .and_then(|url| MihomoClient::new(url, secret).ok());
        if let Some(url) = controller {
            state.controller = url;
            state.status = "正在连接 Mihomo...".into();
        }
        Self {
            state,
            client,
            last_refresh: Instant::now() - Duration::from_secs(10),
            proxy_members_focused: false,
            selected_proxy_member_index: 0,
            config,
            config_path,
            add_provider: None,
        }
    }

    pub fn run(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        loop {
            self.refresh();
            terminal.draw(|frame| self.draw(frame))?;
            if event::poll(Duration::from_millis(250))? {
                if let Event::Key(key) = event::read()? {
                    if self.handle_key(key) {
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    fn refresh(&mut self) {
        if self.client.is_none() || self.last_refresh.elapsed() < Duration::from_secs(5) {
            return;
        }
        self.last_refresh = Instant::now();
        match self.client.as_ref().unwrap().proxies() {
            Ok(proxies) => {
                self.state.proxies = proxies;
                self.state.status = "已连接，代理列表已刷新".into();
            }
            Err(error) => self.state.status = format!("API 错误：{error}"),
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if self.add_provider.is_some() {
            return self.handle_add_provider_key(key);
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return true;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => true,
            KeyCode::Tab => {
                self.state.page = match self.state.page {
                    Page::Dashboard => Page::Proxies,
                    Page::Proxies => Page::Rules,
                    Page::Rules => Page::Config,
                    Page::Config => Page::Dashboard,
                };
                self.state.selected = 0;
                self.proxy_members_focused = false;
                false
            }
            KeyCode::Char('1') => {
                self.state.page = Page::Dashboard;
                self.state.selected = 0;
                self.proxy_members_focused = false;
                false
            }
            KeyCode::Char('2') => {
                self.state.page = Page::Proxies;
                self.state.selected = 0;
                self.proxy_members_focused = false;
                false
            }
            KeyCode::Char('3') => {
                self.state.page = Page::Rules;
                self.state.selected = 0;
                self.proxy_members_focused = false;
                false
            }
            KeyCode::Char('4') => {
                self.state.page = Page::Config;
                self.state.selected = 0;
                self.proxy_members_focused = false;
                false
            }
            KeyCode::Char('a') if self.state.page == Page::Config => {
                self.add_provider = Some(AddProviderDialog {
                    name: String::new(),
                    url: String::new(),
                    editing_url: false,
                });
                false
            }
            KeyCode::Char('t') if self.state.page == Page::Dashboard => {
                self.toggle_feature("tun", "enable", !self.config.tun_enabled);
                false
            }
            KeyCode::Char('d') if self.state.page == Page::Dashboard => {
                self.toggle_feature("dns", "enable", !self.config.dns_enabled);
                false
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if self.state.page == Page::Proxies && self.proxy_members_focused {
                    self.move_proxy_member(1);
                } else {
                    self.move_selection(1);
                }
                false
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if self.state.page == Page::Proxies && self.proxy_members_focused {
                    self.move_proxy_member(-1);
                } else {
                    self.move_selection(-1);
                }
                false
            }
            KeyCode::Right if self.state.page == Page::Proxies => {
                self.open_proxy_members();
                false
            }
            KeyCode::Left if self.state.page == Page::Proxies => {
                self.proxy_members_focused = false;
                false
            }
            KeyCode::Char(' ') if self.state.page == Page::Rules => {
                self.state.rules.toggle(self.state.selected);
                false
            }
            KeyCode::Char('J') if self.state.page == Page::Rules => {
                self.state.rules.move_rule(self.state.selected, 1);
                false
            }
            KeyCode::Char('K') if self.state.page == Page::Rules => {
                self.state.rules.move_rule(self.state.selected, -1);
                false
            }
            KeyCode::Char('r') if self.state.page == Page::Config => {
                self.refresh_provider();
                false
            }
            KeyCode::Char('r') => {
                self.last_refresh = Instant::now() - Duration::from_secs(10);
                false
            }
            KeyCode::Char('s') if self.state.page == Page::Rules => {
                self.save_rules();
                false
            }
            KeyCode::Enter if self.state.page == Page::Proxies => {
                if self.proxy_members_focused {
                    self.select_proxy();
                } else {
                    self.open_proxy_members();
                }
                false
            }
            _ => false,
        }
    }

    fn move_selection(&mut self, delta: isize) {
        let len = match self.state.page {
            Page::Dashboard => 1,
            Page::Proxies => self.state.proxies.len(),
            Page::Rules => self.state.rules.rules.len(),
            Page::Config => self.config.providers.len(),
        };
        if len == 0 {
            return;
        }
        self.state.selected =
            ((self.state.selected as isize + delta).rem_euclid(len as isize)) as usize;
    }

    fn handle_add_provider_key(&mut self, key: KeyEvent) -> bool {
        let Some(dialog) = self.add_provider.as_mut() else {
            return false;
        };
        match key.code {
            KeyCode::Esc => self.add_provider = None,
            KeyCode::Tab => dialog.editing_url = !dialog.editing_url,
            KeyCode::Backspace => {
                if dialog.editing_url {
                    dialog.url.pop();
                } else {
                    dialog.name.pop();
                }
            }
            KeyCode::Char(character) => {
                if dialog.editing_url {
                    dialog.url.push(character);
                } else {
                    dialog.name.push(character);
                }
            }
            KeyCode::Enter if dialog.editing_url => self.commit_add_provider(),
            KeyCode::Enter => dialog.editing_url = true,
            _ => {}
        }
        false
    }

    fn commit_add_provider(&mut self) {
        let Some(dialog) = self.add_provider.take() else {
            return;
        };
        let Some(path) = self.config_path.as_deref() else {
            self.state.status = "No Mihomo config path discovered".into();
            return;
        };
        match config::add_http_provider(path, &dialog.name, &dialog.url) {
            Ok(backup) => match Command::new("systemctl")
                .args(["reload", "mihomo"])
                .status()
            {
                Ok(status) if status.success() => match config::load(path) {
                    Ok(snapshot) => {
                        apply_config(&mut self.state, &snapshot);
                        self.config = snapshot;
                        self.state.status =
                            format!("Added {}; backup {}", dialog.name, backup.display());
                    }
                    Err(error) => {
                        self.state.status =
                            format!("Added provider but could not reread config: {error}")
                    }
                },
                Ok(status) => {
                    self.state.status = format!("Provider saved, reload failed ({status})")
                }
                Err(error) => {
                    self.state.status = format!("Provider saved, reload unavailable: {error}")
                }
            },
            Err(error) => self.state.status = format!("Provider not added: {error}"),
        }
    }

    fn toggle_feature(&mut self, section: &str, key: &str, enabled: bool) {
        let Some(path) = self.config_path.as_deref() else {
            self.state.status = "No Mihomo config path discovered".into();
            return;
        };
        match config::set_boolean(path, section, key, enabled) {
            Ok(backup) => match Command::new("systemctl")
                .args(["reload", "mihomo"])
                .status()
            {
                Ok(status) if status.success() => match config::load(path) {
                    Ok(snapshot) => {
                        apply_config(&mut self.state, &snapshot);
                        self.config = snapshot;
                        self.state.status = format!(
                            "{} {}; backup {}",
                            section,
                            if enabled { "enabled" } else { "disabled" },
                            backup.display()
                        );
                    }
                    Err(error) => {
                        self.state.status =
                            format!("Saved {section}, but config reload failed: {error}")
                    }
                },
                Ok(status) => {
                    self.state.status = format!("Saved {section}, reload failed ({status})")
                }
                Err(error) => {
                    self.state.status = format!("Saved {section}, reload unavailable: {error}")
                }
            },
            Err(error) => self.state.status = format!("{section} not changed: {error}"),
        }
    }

    fn select_proxy(&mut self) {
        let Some(client) = &self.client else {
            self.state.status = "Demo mode: no API action".into();
            return;
        };
        let Some(proxy) = self.state.proxies.get(self.state.selected) else {
            return;
        };
        let Some(target) = proxy.members.get(self.selected_proxy_member_index) else {
            return;
        };
        let group = proxy.name.clone();
        let target = target.clone();
        match client.select_proxy(&group, &target) {
            Ok(()) => {
                if let Some(proxy) = self.state.proxies.get_mut(self.state.selected) {
                    proxy.now = Some(target.clone());
                }
                self.state.status = format!("{group} -> {target}");
            }
            Err(error) => self.state.status = format!("API error: {error}"),
        }
    }

    fn draw(&self, frame: &mut Frame) {
        let area = frame.area();
        let root = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(area);
        let tabs = Tabs::new(vec!["1 状态", "2 代理", "3 规则", "4 配置"])
            .select(match self.state.page {
                Page::Dashboard => 0,
                Page::Proxies => 1,
                Page::Rules => 2,
                Page::Config => 3,
            })
            .block(Block::bordered().title(" mihomo-tui 控制台 "))
            .highlight_style(Style::default().fg(Color::Yellow));
        frame.render_widget(tabs, root[0]);
        match self.state.page {
            Page::Dashboard => self.dashboard(frame, root[1]),
            Page::Proxies => self.proxies(frame, root[1]),
            Page::Rules => self.rules(frame, root[1]),
            Page::Config => self.config(frame, root[1]),
        }
        if let Some(dialog) = &self.add_provider {
            self.add_provider_dialog(frame, dialog);
        }
        frame.render_widget(
            Paragraph::new(format!(
                " {} | Tab/1-4 页面  j/k 移动  t/d 切换 TUN/DNS  右/回车选节点  左返回组  回车应用  s 保存规则  r 刷新  q 退出界面",
                self.state.status
            ))
            .style(Style::default().fg(Color::Gray)),
            root[2],
        );
    }

    fn dashboard(&self, frame: &mut Frame, area: Rect) {
        let rows = vec![
            ListItem::new("核心       Mihomo API"),
            ListItem::new(format!("控制器     {}", self.state.controller)),
            ListItem::new(format!(
                "配置文件   {}",
                self.config_path
                    .as_deref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "未发现".into())
            )),
            ListItem::new(format!("模式       {}", self.config.mode)),
            ListItem::new(format!(
                "混合端口   {}",
                self.config
                    .mixed_port
                    .map(|port| port.to_string())
                    .unwrap_or_else(|| "未配置".into())
            )),
            ListItem::new(format!(
                "TUN        {} (t 切换)",
                if self.config.tun_enabled {
                    "已启用"
                } else {
                    "已关闭"
                }
            )),
            ListItem::new(format!(
                "DNS        {} ({}, d 切换)",
                if self.config.dns_enabled {
                    "已启用"
                } else {
                    "已关闭"
                },
                self.config.dns_mode
            )),
            ListItem::new(format!("代理组     {}", self.state.proxies.len())),
            ListItem::new(format!(
                "规则       {} 条启用 / 共 {} 条",
                self.state.rules.rules.iter().filter(|r| r.enabled).count(),
                self.state.rules.rules.len()
            )),
            ListItem::new(format!("订阅源     {}", self.config.providers.len())),
        ];
        frame.render_widget(
            List::new(rows)
                .block(Block::bordered().title(" 运行状态 "))
                .highlight_style(Style::default().fg(Color::Cyan)),
            area,
        );
    }

    fn proxies(&self, frame: &mut Frame, area: Rect) {
        let columns = Layout::horizontal([Constraint::Percentage(62), Constraint::Percentage(38)])
            .split(area);
        let rows: Vec<Row> = self
            .state
            .proxies
            .iter()
            .map(|p| {
                Row::new(vec![
                    p.name.clone(),
                    p.proxy_type.clone(),
                    p.now.clone().unwrap_or_else(|| "-".into()),
                    p.delay_ms
                        .map(|d| format!("{d} ms"))
                        .unwrap_or_else(|| "-".into()),
                    p.members.len().to_string(),
                ])
            })
            .collect();
        let table = Table::new(
            rows,
            [
                Constraint::Percentage(27),
                Constraint::Percentage(20),
                Constraint::Percentage(25),
                Constraint::Percentage(15),
                Constraint::Percentage(13),
            ],
        )
        .header(
            Row::new(vec!["代理组", "类型", "当前节点", "延迟", "节点数"])
                .style(Style::default().fg(Color::Yellow)),
        )
        .row_highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White))
        .block(Block::bordered().title(" 代理组 "));
        frame.render_stateful_widget(
            table,
            columns[0],
            &mut TableState::default().with_selected(Some(self.state.selected)),
        );

        let members = self
            .state
            .proxies
            .get(self.state.selected)
            .map(|proxy| {
                proxy
                    .members
                    .iter()
                    .map(|member| {
                        let marker = if Some(member.as_str()) == self.selected_proxy_member() {
                            "* "
                        } else {
                            "  "
                        };
                        ListItem::new(format!("{marker}{member}"))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let title = self
            .state
            .proxies
            .get(self.state.selected)
            .map(|proxy| format!(" 节点：{} ", proxy.name))
            .unwrap_or_else(|| " 节点 ".into());
        frame.render_widget(
            List::new(members)
                .block(Block::bordered().title(title))
                .highlight_style(Style::default().fg(Color::Cyan)),
            columns[1],
        );
    }

    fn selected_proxy_member(&self) -> Option<&str> {
        self.state
            .proxies
            .get(self.state.selected)
            .and_then(|proxy| proxy.members.get(self.selected_proxy_member_index()))
            .map(String::as_str)
    }

    fn selected_proxy_member_index(&self) -> usize {
        if self.proxy_members_focused {
            return self.selected_proxy_member_index;
        }
        self.state
            .proxies
            .get(self.state.selected)
            .and_then(|proxy| {
                proxy
                    .now
                    .as_ref()
                    .and_then(|current| proxy.members.iter().position(|member| member == current))
            })
            .unwrap_or(0)
    }

    fn open_proxy_members(&mut self) {
        self.selected_proxy_member_index = self.selected_proxy_member_index();
        self.proxy_members_focused = true;
    }

    fn move_proxy_member(&mut self, delta: isize) {
        let Some(proxy) = self.state.proxies.get(self.state.selected) else {
            return;
        };
        if proxy.members.is_empty() {
            return;
        }
        self.selected_proxy_member_index = ((self.selected_proxy_member_index as isize + delta)
            .rem_euclid(proxy.members.len() as isize))
            as usize;
    }

    fn save_rules(&mut self) {
        let Some(path) = self.config_path.as_deref() else {
            self.state.status = "No Mihomo config path discovered".into();
            return;
        };
        let rules = self
            .state
            .rules
            .rules
            .iter()
            .filter(|rule| rule.enabled)
            .map(|rule| crate::config::ConfigRule {
                kind: rule.kind.clone(),
                value: rule.value.clone(),
                action: rule.action.label().to_string(),
            })
            .collect::<Vec<_>>();
        match config::save_rules(path, &rules) {
            Ok(backup) => match Command::new("systemctl")
                .args(["reload", "mihomo"])
                .status()
            {
                Ok(status) if status.success() => {
                    self.config.rules = rules;
                    self.state.status = format!("Rules saved; backup {}", backup.display());
                }
                Ok(status) => self.state.status = format!("Rules saved, reload failed ({status})"),
                Err(error) => {
                    self.state.status = format!("Rules saved, reload unavailable: {error}")
                }
            },
            Err(error) => self.state.status = format!("Rules not saved: {error}"),
        }
    }

    fn refresh_provider(&mut self) {
        let Some(provider) = self.config.providers.get(self.state.selected) else {
            self.state.status = "No proxy provider selected".into();
            return;
        };
        if provider.kind != "http" {
            self.state.status = format!(
                "{} is a {} provider; update its source file instead",
                provider.name, provider.kind
            );
            return;
        }
        let Some(client) = &self.client else {
            self.state.status = "No Mihomo controller available".into();
            return;
        };
        match client.refresh_provider(&provider.name) {
            Ok(()) => self.state.status = format!("{} subscription refreshed", provider.name),
            Err(error) => self.state.status = format!("Provider update failed: {error}"),
        }
    }

    fn rules(&self, frame: &mut Frame, area: Rect) {
        let rows: Vec<Row> = self
            .state
            .rules
            .rules
            .iter()
            .map(|rule| {
                Row::new(vec![
                    if rule.enabled { "on" } else { "off" }.to_string(),
                    rule.kind.clone(),
                    rule.value.clone(),
                    rule.action.label().to_string(),
                ])
            })
            .collect();
        let table = Table::new(
            rows,
            [
                Constraint::Length(5),
                Constraint::Length(20),
                Constraint::Percentage(55),
                Constraint::Percentage(20),
            ],
        )
        .header(
            Row::new(vec!["状态", "类型", "匹配项", "动作"])
                .style(Style::default().fg(Color::Yellow)),
        )
        .row_highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White))
        .block(Block::bordered().title(" 规则（空格开关，J/K 排序，s 保存） "));
        frame.render_stateful_widget(
            table,
            area,
            &mut TableState::default().with_selected(Some(self.state.selected)),
        );
    }

    fn config(&self, frame: &mut Frame, area: Rect) {
        let rows: Vec<Row> = self
            .config
            .providers
            .iter()
            .map(|provider| {
                Row::new(vec![
                    provider.name.clone(),
                    provider.kind.clone(),
                    provider.url.clone().unwrap_or_else(|| "-".into()),
                    provider.path.clone().unwrap_or_else(|| "-".into()),
                    provider
                        .interval
                        .map(|seconds| format!("{seconds}s"))
                        .unwrap_or_else(|| "manual".into()),
                ])
            })
            .collect();
        let table = Table::new(
            rows,
            [
                Constraint::Percentage(15),
                Constraint::Percentage(12),
                Constraint::Percentage(32),
                Constraint::Percentage(28),
                Constraint::Percentage(13),
            ],
        )
        .header(
            Row::new(vec!["订阅名", "类型", "URL", "路径", "周期"])
                .style(Style::default().fg(Color::Yellow)),
        )
        .row_highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White))
        .block(Block::bordered().title(" 代理订阅（a 新增，r 刷新 HTTP 订阅） "));
        frame.render_stateful_widget(
            table,
            area,
            &mut TableState::default().with_selected(Some(self.state.selected)),
        );
    }

    fn add_provider_dialog(&self, frame: &mut Frame, dialog: &AddProviderDialog) {
        let area = centered_rect(70, 9, frame.area());
        let name_label = if dialog.editing_url {
            "名称"
        } else {
            "> 名称"
        };
        let url_label = if dialog.editing_url { "> URL" } else { "URL" };
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(format!(
                "{name_label}: {}\n{url_label}: {}\n\nTab 切换字段  回车保存  Esc 取消",
                dialog.name, dialog.url
            ))
            .block(Block::bordered().title(" 新增 HTTP 订阅 "))
            .style(Style::default().fg(Color::White)),
            area,
        );
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width.saturating_sub(2));
    let height = height.min(area.height.saturating_sub(2));
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

fn apply_config(state: &mut AppState, config: &ConfigSnapshot) {
    state.rules = RuleSet {
        rules: config
            .rules
            .iter()
            .map(|rule| {
                Rule::new(
                    &rule.kind,
                    &rule.value,
                    match rule.action.as_str() {
                        "DIRECT" => RuleAction::Direct,
                        "REJECT" | "REJECT-DROP" => RuleAction::Reject,
                        "PROXY" => RuleAction::Proxy,
                        group => RuleAction::Group(group.into()),
                    },
                )
            })
            .collect(),
    };
    state.profiles = config
        .providers
        .iter()
        .map(|provider| Profile {
            name: provider.name.clone(),
            kind: provider.kind.clone(),
            source: provider
                .url
                .clone()
                .or_else(|| provider.path.clone())
                .unwrap_or_else(|| "not configured".into()),
            updated: provider
                .interval
                .map(|seconds| format!("every {seconds}s"))
                .unwrap_or_else(|| "manual".into()),
            enabled: true,
        })
        .collect();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_member_selection_starts_at_the_current_member() {
        let app = App::new(None, None, None);

        assert_eq!(app.selected_proxy_member(), Some("Tokyo-01"));
    }

    #[test]
    fn proxy_member_navigation_does_not_change_the_proxy_group() {
        let mut app = App::new(None, None, None);

        app.open_proxy_members();
        app.move_proxy_member(1);

        assert_eq!(app.state.selected, 0);
        assert_eq!(app.selected_proxy_member(), Some("Singapore-02"));
    }
}
