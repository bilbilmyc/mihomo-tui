use crate::config::{self, ConfigSnapshot};
use crate::dialogs::{
    DialogAction, NetworkSettings, RuleDialog, RuleDialogMode, SettingsDialog, centered_rect,
    draw_rule_dialog, draw_settings_dialog,
};
use crate::mihomo::MihomoClient;
use crate::models::{AppState, Page, ProxyDelay, ProxySummary, Rule, RuleAction, RuleSet};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{prelude::*, widgets::*};
use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread,
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
    config_reload: ConfigReload,
    add_provider: Option<AddProviderDialog>,
    settings_dialog: Option<SettingsDialog>,
    rule_dialog: Option<RuleDialog>,
    worker_tx: Sender<WorkerResult>,
    worker_rx: Receiver<WorkerResult>,
    refresh_in_flight: bool,
    provider_refresh_in_flight: bool,
    delay_probe_in_flight: bool,
    proxy_selection_in_flight: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigReload {
    None,
    LocalSystemd,
}

#[derive(Default)]
struct TextInput {
    value: String,
    cursor: usize,
}

impl TextInput {
    fn with_value(value: impl Into<String>) -> Self {
        let value = value.into();
        let cursor = value.len();
        Self { value, cursor }
    }

    fn value(&self) -> &str {
        &self.value
    }

    fn before_cursor(&self) -> &str {
        &self.value[..self.cursor]
    }

    fn insert(&mut self, character: char) {
        self.value.insert(self.cursor, character);
        self.cursor += character.len_utf8();
    }

    fn insert_text(&mut self, text: &str) -> bool {
        let mut changed = false;
        for character in text.chars().filter(|character| !character.is_control()) {
            self.insert(character);
            changed = true;
        }
        changed
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return match key.code {
                KeyCode::Char('u') => self.clear(),
                KeyCode::Char('w') => self.delete_previous_word(),
                _ => false,
            };
        }
        if key.modifiers.contains(KeyModifiers::ALT) {
            return false;
        }
        match key.code {
            KeyCode::Left => {
                self.move_left();
                false
            }
            KeyCode::Right => {
                self.move_right();
                false
            }
            KeyCode::Home => {
                self.cursor = 0;
                false
            }
            KeyCode::End => {
                self.cursor = self.value.len();
                false
            }
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            KeyCode::Char(character)
                if !character.is_control()
                    && matches!(key.modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT) =>
            {
                self.insert(character);
                true
            }
            _ => false,
        }
    }

    fn move_left(&mut self) {
        if let Some((index, _)) = self.value[..self.cursor].char_indices().next_back() {
            self.cursor = index;
        }
    }

    fn move_right(&mut self) {
        if let Some(character) = self.value[self.cursor..].chars().next() {
            self.cursor += character.len_utf8();
        }
    }

    fn backspace(&mut self) -> bool {
        let Some((previous, _)) = self.value[..self.cursor].char_indices().next_back() else {
            return false;
        };
        self.value.drain(previous..self.cursor);
        self.cursor = previous;
        true
    }

    fn delete(&mut self) -> bool {
        let Some(character) = self.value[self.cursor..].chars().next() else {
            return false;
        };
        self.value
            .drain(self.cursor..self.cursor + character.len_utf8());
        true
    }

    fn clear(&mut self) -> bool {
        if self.value.is_empty() {
            return false;
        }
        self.value.clear();
        self.cursor = 0;
        true
    }

    fn delete_previous_word(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        let before_cursor = &self.value[..self.cursor];
        let word_end = before_cursor.trim_end_matches(char::is_whitespace).len();
        let word_start = before_cursor[..word_end]
            .char_indices()
            .rev()
            .find(|(_, character)| character.is_whitespace())
            .map(|(index, character)| index + character.len_utf8())
            .unwrap_or(0);
        self.value.drain(word_start..self.cursor);
        self.cursor = word_start;
        true
    }
}

struct AddProviderDialog {
    name: TextInput,
    url: TextInput,
    editing_url: bool,
    error: Option<String>,
}

impl AddProviderDialog {
    fn active_input(&self) -> &TextInput {
        if self.editing_url {
            &self.url
        } else {
            &self.name
        }
    }

    fn active_input_mut(&mut self) -> &mut TextInput {
        if self.editing_url {
            &mut self.url
        } else {
            &mut self.name
        }
    }

    fn insert_text(&mut self, text: &str) {
        if self.active_input_mut().insert_text(text) {
            self.error = None;
        }
    }
}

enum WorkerResult {
    Proxies(Result<Vec<ProxySummary>, String>),
    ProxySelected {
        group: String,
        target: String,
        result: Result<(), String>,
    },
    DelayMeasured {
        target: String,
        result: Result<ProxyDelay, String>,
    },
    ProviderRefreshed {
        name: String,
        result: Result<Vec<ProxySummary>, String>,
    },
}

impl App {
    #[cfg(test)]
    pub fn new(
        controller: Option<String>,
        secret: Option<String>,
        config_path: Option<PathBuf>,
    ) -> Self {
        Self::with_config_reload(controller, secret, config_path, ConfigReload::None)
    }

    pub fn with_config_reload(
        controller: Option<String>,
        secret: Option<String>,
        config_path: Option<PathBuf>,
        config_reload: ConfigReload,
    ) -> Self {
        let demo_mode = controller.is_none() && config_path.is_none();
        let mut state = if demo_mode {
            AppState::demo()
        } else {
            AppState::live(controller.as_deref().unwrap_or("未连接"))
        };
        let config = if let Some(path) = config_path.as_deref() {
            match config::load(path) {
                Ok(snapshot) => {
                    apply_config(&mut state, &snapshot);
                    snapshot
                }
                Err(error) => {
                    state.status = format!("Config read error: {error}");
                    ConfigSnapshot::default()
                }
            }
        } else {
            ConfigSnapshot::default()
        };
        let client = controller
            .as_ref()
            .and_then(|url| MihomoClient::new(url, secret).ok());
        if let Some(url) = controller {
            state.controller = url;
            state.status = "正在连接 Mihomo...".into();
        }
        let (worker_tx, worker_rx) = mpsc::channel();
        Self {
            state,
            client,
            last_refresh: Instant::now() - Duration::from_secs(10),
            proxy_members_focused: false,
            selected_proxy_member_index: 0,
            config,
            config_path,
            config_reload,
            add_provider: None,
            settings_dialog: None,
            rule_dialog: None,
            worker_tx,
            worker_rx,
            refresh_in_flight: false,
            provider_refresh_in_flight: false,
            delay_probe_in_flight: false,
            proxy_selection_in_flight: false,
        }
    }

    pub fn run(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        loop {
            self.process_worker_results();
            self.refresh();
            terminal.draw(|frame| self.draw(frame))?;
            if event::poll(Duration::from_millis(250))? {
                match event::read()? {
                    Event::Key(key) if self.handle_key(key) => break,
                    Event::Paste(text) => self.handle_paste(&text),
                    _ => {}
                }
            }
        }
        Ok(())
    }

    fn refresh(&mut self) {
        if self.refresh_in_flight || self.last_refresh.elapsed() < Duration::from_secs(5) {
            return;
        }
        let Some(client) = self.client.clone() else {
            return;
        };
        self.last_refresh = Instant::now();
        self.refresh_in_flight = true;
        let sender = self.worker_tx.clone();
        thread::spawn(move || {
            let _ = sender.send(WorkerResult::Proxies(client.proxies()));
        });
    }

    fn process_worker_results(&mut self) {
        while let Ok(message) = self.worker_rx.try_recv() {
            match message {
                WorkerResult::Proxies(result) => {
                    self.refresh_in_flight = false;
                    match result {
                        Ok(proxies) => apply_proxy_refresh(&mut self.state, proxies),
                        Err(error) => self.state.status = format!("API 错误：{error}"),
                    }
                }
                WorkerResult::ProxySelected {
                    group,
                    target,
                    result,
                } => {
                    self.proxy_selection_in_flight = false;
                    match result {
                        Ok(()) => {
                            if let Some(proxy) = self
                                .state
                                .proxies
                                .iter_mut()
                                .find(|proxy| proxy.name == group)
                            {
                                proxy.now = Some(target.clone());
                            }
                            self.state.status = format!("{group} -> {target}");
                            self.last_refresh = Instant::now() - Duration::from_secs(10);
                        }
                        Err(error) => self.state.status = format!("节点切换失败：{error}"),
                    }
                }
                WorkerResult::DelayMeasured { target, result } => {
                    self.delay_probe_in_flight = false;
                    match result {
                        Ok(delay) => apply_delay_result(&mut self.state, &target, delay),
                        Err(error) => self.state.status = format!("延迟测试失败：{error}"),
                    }
                }
                WorkerResult::ProviderRefreshed { name, result } => {
                    self.provider_refresh_in_flight = false;
                    match result {
                        Ok(proxies) => {
                            apply_proxy_refresh(&mut self.state, proxies);
                            self.last_refresh = Instant::now();
                            self.state.status = format!("{name} 订阅及代理列表已刷新");
                        }
                        Err(error) => self.state.status = format!("订阅更新失败：{error}"),
                    }
                }
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return true;
        }
        if self.settings_dialog.is_some() {
            return self.handle_settings_key(key);
        }
        if self.rule_dialog.is_some() {
            return self.handle_rule_dialog_key(key);
        }
        if self.add_provider.is_some() {
            return self.handle_add_provider_key(key);
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
                    name: TextInput::default(),
                    url: TextInput::default(),
                    editing_url: false,
                    error: None,
                });
                false
            }
            KeyCode::Char('e') if self.state.page == Page::Config => {
                let Some(provider) = self.config.providers.get(self.state.selected) else {
                    self.state.status = "未选择订阅".into();
                    return false;
                };
                if provider.kind != "http" {
                    self.state.status = format!("{} 不是 HTTP 订阅，无法修改 URL", provider.name);
                    return false;
                }
                self.add_provider = Some(AddProviderDialog {
                    name: TextInput::with_value(provider.name.clone()),
                    url: TextInput::default(),
                    editing_url: true,
                    error: None,
                });
                false
            }
            KeyCode::Char('t') if self.state.page == Page::Dashboard => {
                self.settings_dialog = Some(SettingsDialog::tun(&self.config.tun));
                false
            }
            KeyCode::Char('d') if self.state.page == Page::Dashboard => {
                self.settings_dialog = Some(SettingsDialog::dns(&self.config.dns));
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
            KeyCode::Right if self.state.page == Page::Config => {
                self.open_provider_proxy_group();
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
            KeyCode::Char('x') if self.state.page == Page::Rules => {
                self.state.rules.toggle(self.state.selected);
                false
            }
            KeyCode::Char('a') if self.state.page == Page::Rules => {
                self.open_rule_dialog(RuleDialogMode::InsertFront);
                false
            }
            KeyCode::Char('A') if self.state.page == Page::Rules => {
                self.open_rule_dialog(RuleDialogMode::InsertBack);
                false
            }
            KeyCode::Char('e') if self.state.page == Page::Rules => {
                self.open_rule_dialog(RuleDialogMode::Edit(self.state.selected));
                false
            }
            KeyCode::Char('J') if self.state.page == Page::Rules => {
                if self.state.rules.move_rule(self.state.selected, 1) {
                    self.state.selected += 1;
                }
                false
            }
            KeyCode::Char('K') if self.state.page == Page::Rules => {
                if self.state.rules.move_rule(self.state.selected, -1) {
                    self.state.selected -= 1;
                }
                false
            }
            KeyCode::Char('r') if self.state.page == Page::Config => {
                self.refresh_provider();
                false
            }
            KeyCode::Char('r') => {
                self.last_refresh = Instant::now() - Duration::from_secs(10);
                self.state.status = "正在刷新代理列表...".into();
                false
            }
            KeyCode::Char('l')
                if self.state.page == Page::Proxies && self.proxy_members_focused =>
            {
                self.measure_proxy_delay();
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
            KeyCode::Enter if self.state.page == Page::Config => {
                self.open_provider_proxy_group();
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
            KeyCode::Enter if dialog.editing_url => self.commit_add_provider(),
            KeyCode::Enter => dialog.editing_url = true,
            _ => {
                if dialog.active_input_mut().handle_key(key) {
                    dialog.error = None;
                }
            }
        }
        false
    }

    fn handle_paste(&mut self, text: &str) {
        if let Some(dialog) = self.add_provider.as_mut() {
            dialog.insert_text(text);
        }
    }

    fn handle_settings_key(&mut self, key: KeyEvent) -> bool {
        let action = self
            .settings_dialog
            .as_mut()
            .map(|dialog| dialog.handle_key(key))
            .unwrap_or(DialogAction::None);
        match action {
            DialogAction::None => {}
            DialogAction::Close => self.settings_dialog = None,
            DialogAction::Save => self.commit_settings_dialog(),
        }
        false
    }

    fn commit_settings_dialog(&mut self) {
        let Some(path) = self.config_path.clone() else {
            self.state.status = "未发现 Mihomo 配置路径".into();
            return;
        };
        let pending = match self.settings_dialog.as_ref().map(SettingsDialog::value) {
            Some(Ok(settings)) => settings,
            Some(Err(error)) => {
                self.state.status = format!("高级设置未保存：{error}");
                return;
            }
            None => return,
        };
        let tun_enabling = matches!(
            &pending,
            NetworkSettings::Tun(settings) if settings.enable && !self.config.tun.enable
        );
        let result = match pending {
            NetworkSettings::Tun(settings) => config::save_tun_settings(&path, &settings),
            NetworkSettings::Dns(settings) => config::save_dns_settings(&path, &settings),
        };
        match result {
            Ok(backup) => match self.finalize_config_change(&backup) {
                Ok(()) => {
                    self.settings_dialog = None;
                    let mut status = format!("高级设置已保存；备份 {}", backup.display());
                    if tun_enabling {
                        let direct_rules = self.config.direct_rule_count();
                        if direct_rules > 0 {
                            status.push_str(&format!(
                                "；仍有 {direct_rules} 条 DIRECT 规则，请检查直连可达性"
                            ));
                        }
                    }
                    self.state.status = status;
                }
                Err(error) => self.state.status = format!("高级设置未应用：{error}"),
            },
            Err(error) => self.state.status = format!("高级设置未保存：{error}"),
        }
    }

    fn open_rule_dialog(&mut self, mode: RuleDialogMode) {
        let original = match mode {
            RuleDialogMode::Edit(index) => {
                let Some(rule) = self.state.rules.rules.get(index).cloned() else {
                    self.state.status = "未选择可编辑的规则".into();
                    return;
                };
                Some(rule)
            }
            RuleDialogMode::InsertFront | RuleDialogMode::InsertBack => None,
        };
        self.rule_dialog = Some(RuleDialog::new(mode, original, self.rule_policy_options()));
    }

    fn rule_policy_options(&self) -> Vec<String> {
        let mut policies = Vec::new();
        for policy in self
            .config
            .proxy_groups
            .iter()
            .chain(self.state.proxies.iter().map(|proxy| &proxy.name))
            .map(String::as_str)
            .chain(["DIRECT", "REJECT", "REJECT-DROP"])
        {
            if !policies.iter().any(|existing| existing == policy) {
                policies.push(policy.to_string());
            }
        }
        policies
    }

    fn handle_rule_dialog_key(&mut self, key: KeyEvent) -> bool {
        let action = self
            .rule_dialog
            .as_mut()
            .map(|dialog| dialog.handle_key(key))
            .unwrap_or(DialogAction::None);
        match action {
            DialogAction::None => {}
            DialogAction::Close => self.rule_dialog = None,
            DialogAction::Save => self.commit_rule_dialog(),
        }
        false
    }

    fn commit_rule_dialog(&mut self) {
        let Some(dialog) = self.rule_dialog.as_ref() else {
            return;
        };
        let mode = dialog.mode();
        let rule = match dialog.rule() {
            Ok(rule) => rule,
            Err(error) => {
                self.state.status = format!("规则未应用：{error}");
                return;
            }
        };
        let status = match mode {
            RuleDialogMode::InsertFront => {
                self.state.rules.rules.insert(0, rule);
                self.state.selected = 0;
                "规则已新增到顶部；按 s 保存配置"
            }
            RuleDialogMode::InsertBack => {
                self.state.rules.rules.push(rule);
                self.state.selected = self.state.rules.rules.len().saturating_sub(1);
                "规则已追加到底部；按 s 保存配置"
            }
            RuleDialogMode::Edit(index) => {
                let Some(existing) = self.state.rules.rules.get_mut(index) else {
                    self.state.status = "原规则已不存在".into();
                    return;
                };
                *existing = rule;
                self.state.selected = index;
                "规则已更新；按 s 保存配置"
            }
        };
        self.rule_dialog = None;
        self.state.status = status.into();
    }

    fn commit_add_provider(&mut self) {
        let Some(dialog) = self.add_provider.as_ref() else {
            return;
        };
        let name = dialog.name.value().to_string();
        let url = dialog.url.value().to_string();
        let Some(path) = self.config_path.clone() else {
            self.set_add_provider_error("未发现 Mihomo 配置路径".into());
            return;
        };
        let updating = self
            .config
            .providers
            .iter()
            .any(|provider| provider.name == name);
        match config::add_http_provider(&path, &name, &url) {
            Ok(backup) => match self.finalize_config_change(&backup) {
                Ok(()) => {
                    let action = if updating { "已更新" } else { "已新增" };
                    self.add_provider = None;
                    self.state.status = format!("{action} {name}；备份 {}", backup.display())
                }
                Err(error) => self.set_add_provider_error(format!("订阅未应用：{error}")),
            },
            Err(error) => {
                let error = provider_form_error(&error);
                self.set_add_provider_error(format!("订阅未保存：{error}"));
            }
        }
    }

    fn set_add_provider_error(&mut self, error: String) {
        if let Some(dialog) = self.add_provider.as_mut() {
            dialog.error = Some(error.clone());
        }
        self.state.status = error;
    }

    fn finalize_config_change(&mut self, backup: &Path) -> Result<(), String> {
        let path = self
            .config_path
            .clone()
            .ok_or_else(|| "未发现 Mihomo 配置路径".to_string())?;
        let reload_result =
            apply_config_reload(self.config_reload, &path, backup, reload_mihomo_service);
        let state_result = config::load(&path).map(|snapshot| {
            apply_config(&mut self.state, &snapshot);
            self.config = snapshot;
        });
        match (reload_result, state_result) {
            (Ok(()), Ok(())) => Ok(()),
            (Ok(()), Err(error)) if self.config_reload == ConfigReload::LocalSystemd => {
                Err(format!("核心已重载，但重新读取配置失败：{error}"))
            }
            (Ok(()), Err(error)) => Err(format!("配置已保存，但重新读取失败：{error}")),
            (Err(error), Ok(())) => Err(error),
            (Err(reload_error), Err(read_error)) => Err(format!(
                "{reload_error}；重新读取恢复后的配置也失败：{read_error}"
            )),
        }
    }

    fn select_proxy(&mut self) {
        if self.proxy_selection_in_flight {
            self.state.status = "正在切换节点...".into();
            return;
        }
        let Some(client) = self.client.clone() else {
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
        self.proxy_selection_in_flight = true;
        self.state.status = format!("正在切换 {group} -> {target}...");
        let sender = self.worker_tx.clone();
        thread::spawn(move || {
            let result = client.select_proxy(&group, &target);
            let _ = sender.send(WorkerResult::ProxySelected {
                group,
                target,
                result,
            });
        });
    }

    fn open_provider_proxy_group(&mut self) {
        let Some(provider) = self.config.providers.get(self.state.selected) else {
            self.state.status = "No proxy provider selected".into();
            return;
        };
        let provider_name = provider.name.clone();
        let configured_groups = self.config.provider_groups.get(&provider_name);
        let group_index = configured_groups
            .into_iter()
            .flatten()
            .find_map(|group_name| {
                self.state
                    .proxies
                    .iter()
                    .position(|proxy| proxy.name == *group_name && !proxy.members.is_empty())
            })
            .or_else(|| {
                self.state
                    .proxies
                    .iter()
                    .position(|proxy| proxy.name == provider_name && !proxy.members.is_empty())
            });
        let Some(group_index) = group_index else {
            self.state.status =
                "No selectable proxy group found; refresh the subscription first".into();
            return;
        };
        self.state.page = Page::Proxies;
        self.state.selected = group_index;
        self.open_proxy_members();
        self.state.status = format!("{provider_name}：请选择节点并按 Enter 应用");
    }

    fn measure_proxy_delay(&mut self) {
        if self.delay_probe_in_flight {
            self.state.status = "延迟测试正在进行...".into();
            return;
        }
        let Some(client) = self.client.clone() else {
            self.state.status = "No Mihomo controller available".into();
            return;
        };
        let Some(target) = self.selected_proxy_member().map(str::to_owned) else {
            self.state.status = "No proxy node selected".into();
            return;
        };
        self.delay_probe_in_flight = true;
        self.state.status = format!("正在测试 {target} 延迟...");
        let sender = self.worker_tx.clone();
        thread::spawn(move || {
            let result = client.probe_delay(&target);
            let _ = sender.send(WorkerResult::DelayMeasured { target, result });
        });
    }

    fn draw(&self, frame: &mut Frame) {
        let area = frame.area();
        let root = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(3),
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
        if let Some(dialog) = &self.settings_dialog {
            draw_settings_dialog(frame, dialog);
        }
        if let Some(dialog) = &self.rule_dialog {
            draw_rule_dialog(frame, dialog);
        }
        let footer =
            Layout::vertical([Constraint::Length(2), Constraint::Length(1)]).split(root[2]);
        frame.render_widget(
            Paragraph::new(format!(" {}", self.state.status))
                .wrap(Wrap { trim: true })
                .style(Style::default().fg(Color::Gray)),
            footer[0],
        );
        frame.render_widget(
            Paragraph::new(format!(" {}", self.shortcut_help()))
                .style(Style::default().fg(Color::DarkGray)),
            footer[1],
        );
    }

    fn shortcut_help(&self) -> &'static str {
        if self.add_provider.is_some() {
            return "Tab 切换字段  Enter 保存  Esc 取消  Ctrl+C 退出";
        }
        if let Some(dialog) = &self.settings_dialog {
            return if dialog.editing() {
                "输入内容  Enter 完成  Esc 停止编辑  Ctrl+C 退出"
            } else {
                "j/k 字段  Enter/Space 修改  s 保存  Esc 取消"
            };
        }
        if let Some(dialog) = &self.rule_dialog {
            return if dialog.selecting() {
                "↑/↓ 选择  Space/Enter 确认  Esc 返回  Ctrl+C 退出"
            } else if dialog.editing() {
                "输入匹配内容  Enter 完成  Esc 停止编辑  Ctrl+C 退出"
            } else {
                "j/k 字段  Enter 打开列表/编辑  s 应用  Esc 取消"
            };
        }
        match (self.state.page, self.proxy_members_focused) {
            (Page::Dashboard, _) => "1-4/Tab 页面  t TUN  d DNS  r 刷新  q 退出",
            (Page::Proxies, false) => "j/k 代理组  Right/Enter 节点  r 刷新  q 退出",
            (Page::Proxies, true) => "j/k 节点  Enter 应用  l 延迟  Left 返回  q 退出",
            (Page::Rules, _) => "j/k 规则  a/A 前/后新增  e 编辑  x 删除  J/K 排序  s 保存",
            (Page::Config, _) => "j/k 订阅  Enter 打开组  a 新增  e 改址  r 刷新  q 退出",
        }
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
                "TUN        {} (t 设置)",
                if self.config.tun.enable {
                    "已启用"
                } else {
                    "已关闭"
                }
            )),
            ListItem::new(format!(
                "DNS        {} ({}, d 设置)",
                if self.config.dns.enable {
                    "已启用"
                } else {
                    "已关闭"
                },
                self.config.dns.enhanced_mode
            )),
            ListItem::new(format!("代理组     {}", self.state.proxies.len())),
            ListItem::new(format!(
                "规则       {} 条保留 / 共 {} 条",
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
                Constraint::Min(10),
                Constraint::Length(9),
                Constraint::Min(10),
                Constraint::Length(8),
                Constraint::Length(6),
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

        let member_index = self.selected_proxy_member_index();
        let members = self
            .state
            .proxies
            .get(self.state.selected)
            .map(|proxy| {
                proxy
                    .members
                    .iter()
                    .map(|member| {
                        let marker = if proxy.now.as_deref() == Some(member.as_str()) {
                            "* "
                        } else {
                            "  "
                        };
                        let delay = proxy
                            .member_delays
                            .get(member)
                            .map(crate::models::ProxyDelay::label)
                            .unwrap_or_else(|| "-".into());
                        ListItem::new(format!("{marker}{member}  {delay}"))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let title = self
            .state
            .proxies
            .get(self.state.selected)
            .map(|proxy| format!(" 节点：{}（Enter 应用，l 测延迟） ", proxy.name))
            .unwrap_or_else(|| " 节点 ".into());
        let mut member_state = ListState::default();
        member_state.select((!members.is_empty()).then_some(member_index));
        frame.render_stateful_widget(
            List::new(members)
                .block(Block::bordered().title(title))
                .highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White)),
            columns[1],
            &mut member_state,
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
                extra: rule.extra.clone(),
                raw: rule.raw.clone(),
            })
            .collect::<Vec<_>>();
        match config::save_rules(path, &rules) {
            Ok(backup) => match self.finalize_config_change(&backup) {
                Ok(()) => self.state.status = format!("规则已保存；备份 {}", backup.display()),
                Err(error) => self.state.status = format!("规则未应用：{error}"),
            },
            Err(error) => self.state.status = format!("Rules not saved: {error}"),
        }
    }

    fn refresh_provider(&mut self) {
        if self.provider_refresh_in_flight {
            self.state.status = "订阅更新正在进行...".into();
            return;
        }
        let Some(provider) = self.config.providers.get(self.state.selected) else {
            self.state.status = "未选择订阅".into();
            return;
        };
        if provider.kind != "http" {
            self.state.status = format!(
                "{} 是 {} provider，没有远程 URL；请更新源文件或改为 HTTP provider",
                provider.name, provider.kind
            );
            return;
        }
        let Some(client) = self.client.clone() else {
            self.state.status = "未连接 Mihomo 控制器".into();
            return;
        };
        let name = provider.name.clone();
        self.provider_refresh_in_flight = true;
        self.state.status = format!("正在更新 {name} 订阅...");
        let sender = self.worker_tx.clone();
        thread::spawn(move || {
            let result = client
                .refresh_provider(&name)
                .and_then(|()| client.proxies());
            let _ = sender.send(WorkerResult::ProviderRefreshed { name, result });
        });
    }

    fn rules(&self, frame: &mut Frame, area: Rect) {
        let rows: Vec<Row> = self
            .state
            .rules
            .rules
            .iter()
            .map(|rule| {
                Row::new(vec![
                    if rule.enabled { "保留" } else { "删除" }.to_string(),
                    rule.kind.clone(),
                    rule.value.clone(),
                    rule.action.label().to_string(),
                ])
            })
            .collect();
        let table = Table::new(
            rows,
            [
                Constraint::Length(6),
                Constraint::Length(16),
                Constraint::Min(20),
                Constraint::Length(18),
            ],
        )
        .header(
            Row::new(vec!["状态", "类型", "匹配项", "动作"])
                .style(Style::default().fg(Color::Yellow)),
        )
        .row_highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White))
        .block(Block::bordered().title(" 规则（Space 标记/撤销删除，J/K 排序，s 保存） "));
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
                    provider
                        .url
                        .as_deref()
                        .map(provider_url_label)
                        .unwrap_or_else(|| "-".into()),
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
                Constraint::Length(12),
                Constraint::Length(8),
                Constraint::Min(18),
                Constraint::Length(22),
                Constraint::Length(9),
            ],
        )
        .header(
            Row::new(vec!["订阅名", "类型", "URL", "路径", "周期"])
                .style(Style::default().fg(Color::Yellow)),
        )
        .row_highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White))
        .block(Block::bordered().title(" 代理订阅（a 新增，e 改址，r 刷新） "));
        frame.render_stateful_widget(
            table,
            area,
            &mut TableState::default().with_selected(Some(self.state.selected)),
        );
    }

    fn add_provider_dialog(&self, frame: &mut Frame, dialog: &AddProviderDialog) {
        let area = centered_rect(70, 9, frame.area());
        frame.render_widget(Clear, area);
        frame.render_widget(Block::bordered().title(" 新增或更新 HTTP 订阅 "), area);
        let inner = area.inner(Margin::new(1, 1));
        let active_label = if dialog.editing_url { "URL" } else { "名称" };
        let label_width = add_provider_label_width(active_label);
        let input_width = inner.width.saturating_sub(label_width);
        let name_row = Rect::new(inner.x, inner.y, inner.width, 1);
        let url_row = Rect::new(inner.x, inner.y.saturating_add(1), inner.width, 1);
        self.add_provider_field(frame, name_row, "名称", &dialog.name, !dialog.editing_url);
        self.add_provider_field(frame, url_row, "URL", &dialog.url, dialog.editing_url);
        if let Some(error) = &dialog.error {
            frame.render_widget(
                Paragraph::new(error.as_str())
                    .wrap(Wrap { trim: true })
                    .style(Style::default().fg(Color::Red)),
                Rect::new(
                    inner.x,
                    inner.y.saturating_add(3),
                    inner.width,
                    inner.height.saturating_sub(3),
                ),
            );
        }
        let input = dialog.active_input();
        let cursor_width = Line::from(input.before_cursor()).width();
        let scroll = cursor_width.saturating_sub(usize::from(input_width.saturating_sub(1)));
        let cursor_offset = u16::try_from(cursor_width.saturating_sub(scroll)).unwrap_or(u16::MAX);
        let cursor_x = inner
            .x
            .saturating_add(label_width)
            .saturating_add(cursor_offset)
            .min(inner.right().saturating_sub(1));
        let cursor_y = inner
            .y
            .saturating_add(u16::from(dialog.editing_url))
            .min(inner.bottom().saturating_sub(1));
        frame.set_cursor_position(Position::new(cursor_x, cursor_y));
    }

    fn add_provider_field(
        &self,
        frame: &mut Frame,
        area: Rect,
        label: &str,
        input: &TextInput,
        active: bool,
    ) {
        let label_width = add_provider_label_width(label);
        let [label_area, input_area] =
            Layout::horizontal([Constraint::Length(label_width), Constraint::Min(1)]).areas(area);
        let marker = if active { ">" } else { " " };
        frame.render_widget(
            Paragraph::new(format!("{marker} {label}: ")).style(Style::default().fg(if active {
                Color::Yellow
            } else {
                Color::Gray
            })),
            label_area,
        );
        let cursor_width = Line::from(input.before_cursor()).width();
        let scroll = cursor_width.saturating_sub(usize::from(input_area.width.saturating_sub(1)));
        frame.render_widget(
            Paragraph::new(input.value()).scroll((0, u16::try_from(scroll).unwrap_or(u16::MAX))),
            input_area,
        );
    }
}

fn reload_mihomo_service() -> Result<(), String> {
    crate::runtime::reload_service()
}

fn apply_config_reload<F>(
    policy: ConfigReload,
    path: &Path,
    backup: &Path,
    reload: F,
) -> Result<(), String>
where
    F: FnMut() -> Result<(), String>,
{
    match policy {
        ConfigReload::None => Ok(()),
        ConfigReload::LocalSystemd => reload_or_rollback(path, backup, reload),
    }
}

fn reload_or_rollback<F>(path: &Path, backup: &Path, mut reload: F) -> Result<(), String>
where
    F: FnMut() -> Result<(), String>,
{
    let reload_error = match reload() {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };
    config::restore_backup(path, backup)
        .map_err(|error| format!("重载失败（{reload_error}），回滚配置也失败：{error}"))?;
    match reload() {
        Ok(()) => Err(format!(
            "重载失败（{reload_error}）；配置已回滚到修改前版本"
        )),
        Err(rollback_error) => Err(format!(
            "重载失败（{reload_error}）；配置已在磁盘回滚，但恢复版本重载失败（{rollback_error}）"
        )),
    }
}

fn provider_url_label(raw: &str) -> String {
    let Ok(url) = reqwest::Url::parse(raw) else {
        return "<invalid URL>".into();
    };
    let Some(host) = url.host_str() else {
        return "<invalid URL>".into();
    };
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    let port = url
        .port()
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    format!("{}://{host}{port}/...", url.scheme())
}

fn provider_form_error(error: &str) -> String {
    match error {
        "Provider name must use letters, digits, - or _, up to 48 characters" => {
            "名称只能包含字母、数字、- 或 _，最多 48 个字符".into()
        }
        "Subscription must be a valid HTTP URL" => {
            "订阅地址必须是有效的 http:// 或 https:// URL".into()
        }
        _ => error.into(),
    }
}

fn add_provider_label_width(label: &str) -> u16 {
    u16::try_from(Line::from(format!("> {label}: ")).width()).unwrap_or(u16::MAX)
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
                        "REJECT" => RuleAction::Reject,
                        "REJECT-DROP" => RuleAction::RejectDrop,
                        "PROXY" => RuleAction::Proxy,
                        group => RuleAction::Group(group.into()),
                    },
                )
                .with_extra(rule.extra.clone())
                .with_raw(rule.raw.clone())
            })
            .collect(),
    };
}

fn apply_proxy_refresh(state: &mut AppState, mut proxies: Vec<crate::models::ProxySummary>) {
    let existing_delays: std::collections::BTreeMap<_, _> = state
        .proxies
        .iter()
        .flat_map(|proxy| proxy.member_delays.iter())
        .map(|(name, delay)| (name.clone(), delay.clone()))
        .collect();
    for proxy in &mut proxies {
        for member in &proxy.members {
            if let Some(delay) = existing_delays.get(member) {
                proxy
                    .member_delays
                    .entry(member.clone())
                    .or_insert_with(|| delay.clone());
            }
        }
    }
    let report_connection = state.status == "正在连接 Mihomo..."
        || state.status == "正在刷新代理列表..."
        || state.status.starts_with("API 错误：");
    state.proxies = proxies;
    if report_connection {
        state.status = "已连接，代理列表已刷新".into();
    }
}

fn apply_delay_result(state: &mut AppState, target: &str, delay: ProxyDelay) {
    for proxy in &mut state.proxies {
        if proxy.members.iter().any(|member| member == target) {
            proxy
                .member_delays
                .insert(target.to_string(), delay.clone());
        }
        if proxy.name == target {
            proxy.delay_ms = match delay {
                ProxyDelay::Measured(value) => Some(value),
                ProxyDelay::Timeout | ProxyDelay::Failed => None,
            };
        }
    }
    state.status = format!("{target}: {}", delay.label());
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use std::{
        fs,
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        thread,
        time::SystemTime,
    };

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

    #[test]
    fn provider_selection_enters_a_matching_proxy_group() {
        let mut app = App::new(None, None, None);
        app.state.page = Page::Config;
        app.config.providers = vec![crate::config::Provider {
            name: "Proxy".into(),
            kind: "http".into(),
            url: None,
            path: None,
            interval: None,
        }];

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(app.state.page, Page::Proxies);
        assert_eq!(app.state.selected, 0);
        assert!(app.proxy_members_focused);
    }

    #[test]
    fn provider_selection_uses_the_group_that_references_the_provider() {
        let mut app = App::new(None, None, None);
        app.state.page = Page::Config;
        app.config.providers = vec![crate::config::Provider {
            name: "sbyun".into(),
            kind: "http".into(),
            url: None,
            path: None,
            interval: None,
        }];
        app.config
            .provider_groups
            .insert("sbyun".into(), vec!["赛博云".into()]);
        app.state.proxies = AppState::demo().proxies;
        app.state.proxies[0].name = "GLOBAL".into();
        app.state.proxies[1].name = "赛博云".into();

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(app.state.page, Page::Proxies);
        assert_eq!(app.state.selected, 1);
        assert!(app.proxy_members_focused);
    }

    #[test]
    fn demo_mode_keeps_demo_rules_without_a_config_file() {
        let app = App::new(None, None, None);

        assert!(!app.state.rules.rules.is_empty());
    }

    #[test]
    fn configured_controller_never_shows_demo_proxies_or_rules() {
        let app = App::new(Some("http://127.0.0.1:9090".into()), None, None);

        assert!(app.state.proxies.is_empty());
        assert!(app.state.rules.rules.is_empty());
        assert_eq!(app.state.controller, "http://127.0.0.1:9090");
    }

    #[test]
    fn proxy_refresh_preserves_measured_member_delays_and_status() {
        let mut state = AppState::demo();
        state.status = "Tokyo-01: 128 ms".into();
        state.proxies[0]
            .member_delays
            .insert("Tokyo-01".into(), crate::models::ProxyDelay::Measured(128));
        let mut refreshed = state.proxies.clone();
        for proxy in &mut refreshed {
            proxy.member_delays.clear();
        }

        apply_proxy_refresh(&mut state, refreshed);

        assert_eq!(
            state.proxies[0].member_delays.get("Tokyo-01"),
            Some(&crate::models::ProxyDelay::Measured(128))
        );
        assert_eq!(state.status, "Tokyo-01: 128 ms");
    }

    #[test]
    fn provider_urls_hide_subscription_credentials() {
        assert_eq!(
            provider_url_label("https://subscriptions.example.com/user/private-token?format=yaml"),
            "https://subscriptions.example.com/..."
        );
        assert_eq!(provider_url_label("not a url"), "<invalid URL>");
    }

    #[test]
    fn reload_failure_restores_the_previous_config() {
        let unique = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "mihomo-tui-rollback-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("config.yaml");
        let backup = directory.join("config.yaml.bak");
        fs::write(&path, "new config").unwrap();
        fs::write(&backup, "old config").unwrap();
        let mut attempts = 0;

        let error = reload_or_rollback(&path, &backup, || {
            attempts += 1;
            if attempts == 1 {
                Err("reload rejected".into())
            } else {
                Ok(())
            }
        })
        .unwrap_err();

        assert_eq!(fs::read_to_string(&path).unwrap(), "old config");
        assert_eq!(attempts, 2);
        assert!(error.contains("回滚"));
        fs::remove_file(path).unwrap();
        fs::remove_file(backup).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn external_config_changes_never_reload_the_local_service() {
        let mut reloads = 0;
        apply_config_reload(
            ConfigReload::None,
            Path::new("/unused/config.yaml"),
            Path::new("/unused/config.yaml.bak"),
            || {
                reloads += 1;
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(reloads, 0);
    }

    #[test]
    fn moving_a_rule_keeps_the_moved_rule_selected() {
        let mut app = App::new(None, None, None);
        app.state.page = Page::Rules;
        app.state.rules = AppState::demo().rules;
        app.state.selected = 0;
        let moved_value = app.state.rules.rules[0].value.clone();

        app.handle_key(KeyEvent::new(KeyCode::Char('J'), KeyModifiers::SHIFT));

        assert_eq!(app.state.selected, 1);
        assert_eq!(app.state.rules.rules[1].value, moved_value);
    }

    #[test]
    fn control_c_quits_even_when_the_add_provider_dialog_is_open() {
        let mut app = App::new(None, None, None);
        app.state.page = Page::Config;
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));

        let should_quit = app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));

        assert!(should_quit);
    }

    #[test]
    fn edit_provider_opens_the_dialog_for_a_replacement_url() {
        let mut app = App::new(None, None, None);
        app.state.page = Page::Config;
        app.config.providers = vec![crate::config::Provider {
            name: "airport".into(),
            kind: "http".into(),
            url: Some("https://old.example.com/sub".into()),
            path: None,
            interval: None,
        }];

        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));

        let dialog = app.add_provider.as_ref().unwrap();
        assert_eq!(dialog.name.value(), "airport");
        assert!(dialog.url.value().is_empty());
        assert!(dialog.editing_url);
    }

    #[test]
    fn add_provider_input_supports_editing_in_the_middle() {
        let mut app = App::new(None, None, None);
        app.state.page = Page::Config;
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));

        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));

        assert_eq!(app.add_provider.as_ref().unwrap().name.value(), "abc");
    }

    #[test]
    fn add_provider_paste_inserts_at_the_cursor() {
        let mut app = App::new(None, None, None);
        app.state.page = Page::Config;
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        app.handle_paste("ac");
        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));

        app.handle_paste("b");

        assert_eq!(app.add_provider.as_ref().unwrap().name.value(), "abc");
    }

    #[test]
    fn add_provider_input_supports_home_end_and_delete() {
        let mut app = App::new(None, None, None);
        app.state.page = Page::Config;
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        for character in "abcd".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }

        app.handle_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));

        assert_eq!(app.add_provider.as_ref().unwrap().name.value(), "bc");
    }

    #[test]
    fn add_provider_input_supports_clear_and_delete_previous_word() {
        let mut app = App::new(None, None, None);
        app.state.page = Page::Config;
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        for character in "airport old".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }

        app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!(app.add_provider.as_ref().unwrap().name.value(), "airport ");

        app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert!(app.add_provider.as_ref().unwrap().name.value().is_empty());
    }

    #[test]
    fn invalid_provider_input_keeps_the_dialog_and_original_values() {
        let (directory, path) = provider_test_config("provider-invalid");
        let mut app = App::new(None, None, Some(path));
        app.state.page = Page::Config;
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        for character in "bad name".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        for character in "https://subscriptions.example.com/sub".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        let dialog = app.add_provider.as_ref().expect("dialog should stay open");
        assert_eq!(dialog.name.value(), "bad name");
        assert_eq!(dialog.url.value(), "https://subscriptions.example.com/sub");
        assert!(
            dialog
                .error
                .as_deref()
                .is_some_and(|error| error.contains("名称"))
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn successful_provider_save_closes_the_dialog() {
        let (directory, path) = provider_test_config("provider-success");
        let mut app = App::new(None, None, Some(path));
        app.state.page = Page::Config;
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        for character in "airport".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        for character in "https://subscriptions.example.com/sub".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert!(app.add_provider.is_none());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn add_provider_dialog_places_the_cursor_at_the_active_field() {
        let mut app = App::new(None, None, None);
        app.state.page = Page::Config;
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('机'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('场'), KeyModifiers::NONE));
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();

        terminal.draw(|frame| app.draw(frame)).unwrap();
        terminal
            .backend_mut()
            .assert_cursor_position(Position::new(18, 8));

        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        terminal.draw(|frame| app.draw(frame)).unwrap();
        terminal
            .backend_mut()
            .assert_cursor_position(Position::new(13, 9));
    }

    #[test]
    fn dashboard_t_opens_tun_settings_with_current_values() {
        let mut app = App::new(None, None, None);
        app.config.tun.stack = "system".into();
        app.config.tun.mtu = Some(9000);
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();

        app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
        terminal.draw(|frame| app.draw(frame)).unwrap();

        let screen = terminal.backend().to_string();
        assert!(screen.contains("TUN 高级设置"));
        assert!(screen.contains("system"));
        assert!(screen.contains("9000"));
    }

    #[test]
    fn tun_settings_show_a_cursor_while_editing_the_device() {
        let mut app = App::new(None, None, None);
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        terminal.draw(|frame| app.draw(frame)).unwrap();

        terminal
            .backend_mut()
            .assert_cursor_position(Position::new(25, 8));
    }

    #[test]
    fn dashboard_d_opens_dns_settings_with_current_values() {
        let mut app = App::new(None, None, None);
        app.config.dns.enhanced_mode = "fake-ip".into();
        app.config.dns.fake_ip_range = Some("198.18.0.1/16".into());
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();

        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        terminal.draw(|frame| app.draw(frame)).unwrap();

        let screen = terminal.backend().to_string();
        assert!(screen.contains("DNS 高级设置"));
        assert!(screen.contains("fake-ip"));
        assert!(screen.contains("198.18.0.1/16"));
    }

    #[test]
    fn rule_editor_adds_rules_to_the_requested_end_of_the_list() {
        let mut app = App::new(None, None, None);
        app.state.page = Page::Rules;

        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        for character in "front.example".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));

        assert_eq!(app.state.rules.rules[0].value, "front.example");

        app.handle_key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT));
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        for character in "back.example".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));

        assert_eq!(app.state.rules.rules.last().unwrap().value, "back.example");
    }

    #[test]
    fn editing_a_rule_shows_the_cursor_at_the_match_value() {
        let mut app = App::new(None, None, None);
        app.state.page = Page::Rules;
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();

        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        terminal.draw(|frame| app.draw(frame)).unwrap();

        let screen = terminal.backend().to_string();
        assert!(screen.contains("编辑自定义规则"));
        terminal
            .backend_mut()
            .assert_cursor_position(Position::new(35, 9));
    }

    #[test]
    fn rule_editor_offers_configured_proxy_groups_as_policies() {
        let mut app = App::new(None, None, None);
        app.state.page = Page::Rules;
        app.config.proxy_groups = vec!["赛博云".into()];
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();

        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        terminal.draw(|frame| app.draw(frame)).unwrap();

        assert!(terminal.backend().to_string().contains("赛博云"));
    }

    #[test]
    fn rule_kind_enter_opens_a_selectable_list_without_changing_the_value() {
        let mut app = App::new(None, None, None);
        app.state.page = Page::Rules;
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();

        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        terminal.draw(|frame| app.draw(frame)).unwrap();

        let screen = terminal.backend().to_string();
        assert!(screen.contains("选择规则类型"));
        assert!(screen.contains("DOMAIN-SUFFIX"));
        assert!(screen.contains("DOMAIN-KEYWORD"));
    }

    #[test]
    fn rule_kind_list_uses_space_to_confirm_and_advances_to_match_input() {
        let mut app = App::new(None, None, None);
        app.state.page = Page::Rules;
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();

        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        terminal.draw(|frame| app.draw(frame)).unwrap();

        let screen = terminal.backend().to_string();
        assert!(screen.contains("DOMAIN-KEYWORD"));
        assert!(screen.contains('x'));
        terminal
            .backend_mut()
            .assert_cursor_position(Position::new(26, 9));
    }

    #[test]
    fn rule_policy_enter_opens_a_list_and_space_confirms_the_selection() {
        let mut app = App::new(None, None, None);
        app.state.page = Page::Rules;
        app.config.proxy_groups = vec!["赛博云".into()];
        app.state.proxies.clear();
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();

        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        terminal.draw(|frame| app.draw(frame)).unwrap();

        let open_screen = terminal.backend().to_string();
        assert!(open_screen.contains("选择代理策略"));
        assert!(open_screen.contains("赛博云"));
        assert!(open_screen.contains("DIRECT"));

        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        terminal.draw(|frame| app.draw(frame)).unwrap();

        let confirmed_screen = terminal.backend().to_string();
        assert!(!confirmed_screen.contains("选择代理策略"));
        assert!(confirmed_screen.contains("< DIRECT >"));
    }

    #[test]
    fn refresh_returns_before_a_slow_api_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request_line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request_line)
                .unwrap();
            assert!(request_line.starts_with("GET /proxies "));
            thread::sleep(Duration::from_millis(300));
            let body = r#"{"proxies":{"Main":{"type":"Selector","all":["Node"],"now":"Node"},"Node":{"type":"Shadowsocks","history":[]}}}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        let mut app = App::new(Some(format!("http://{address}")), None, None);

        let started = Instant::now();
        app.refresh();
        let elapsed = started.elapsed();
        server.join().unwrap();
        for _ in 0..50 {
            app.process_worker_results();
            if !app.refresh_in_flight {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }

        assert!(
            elapsed < Duration::from_millis(100),
            "refresh blocked the UI for {elapsed:?}"
        );
        assert!(!app.refresh_in_flight);
        assert_eq!(app.state.proxies[0].name, "Main");
    }

    #[test]
    fn context_shortcuts_fit_an_eighty_column_terminal() {
        let mut app = App::new(None, None, None);
        for page in [Page::Dashboard, Page::Proxies, Page::Rules, Page::Config] {
            app.state.page = page;
            app.proxy_members_focused = false;
            assert!(Line::from(app.shortcut_help()).width() <= 78);
        }
        app.state.page = Page::Proxies;
        app.proxy_members_focused = true;
        assert!(Line::from(app.shortcut_help()).width() <= 78);
    }

    fn provider_test_config(label: &str) -> (PathBuf, PathBuf) {
        let unique = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "mihomo-tui-{label}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("config.yaml");
        fs::write(
            &path,
            r#"
mixed-port: 7890
mode: rule
proxy-providers: {}
proxy-groups:
  - name: existing
    type: select
    proxies: [DIRECT]
rules:
  - MATCH,DIRECT
"#,
        )
        .unwrap();
        (directory, path)
    }
}
