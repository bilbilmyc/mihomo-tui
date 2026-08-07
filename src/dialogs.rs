use crate::{
    config::{DnsSettings, TunSettings},
    models::{Rule, RuleAction},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{prelude::*, widgets::*};

pub(crate) enum SettingsDialog {
    Tun(TunSettingsDialog),
    Dns(DnsSettingsDialog),
}

pub(crate) enum NetworkSettings {
    Tun(TunSettings),
    Dns(DnsSettings),
}

pub(crate) struct TunSettingsDialog {
    settings: TunSettings,
    field: usize,
    editing: bool,
    device: String,
    dns_hijack: String,
    mtu: String,
    route_exclude_address: String,
}

pub(crate) struct DnsSettingsDialog {
    settings: DnsSettings,
    field: usize,
    editing: bool,
    listen: String,
    fake_ip_range: String,
    fake_ip_range6: String,
}

pub(crate) struct RuleDialog {
    mode: RuleDialogMode,
    field: usize,
    editing: bool,
    selector: Option<RuleSelector>,
    selector_index: usize,
    kind: String,
    value: String,
    action: String,
    policies: Vec<String>,
    original: Option<Rule>,
}

#[derive(Clone, Copy)]
pub(crate) enum RuleDialogMode {
    InsertFront,
    InsertBack,
    Edit(usize),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RuleSelector {
    Kind,
    Policy,
}

const RULE_KINDS: &[&str] = &[
    "DOMAIN",
    "DOMAIN-SUFFIX",
    "DOMAIN-KEYWORD",
    "DOMAIN-WILDCARD",
    "DOMAIN-REGEX",
    "GEOSITE",
    "IP-CIDR",
    "IP-CIDR6",
    "GEOIP",
    "PROCESS-NAME",
    "PROCESS-PATH",
    "DST-PORT",
    "SRC-PORT",
    "RULE-SET",
    "NETWORK",
    "MATCH",
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DialogAction {
    None,
    Close,
    Save,
}

impl SettingsDialog {
    pub(crate) fn tun(settings: &TunSettings) -> Self {
        Self::Tun(TunSettingsDialog {
            settings: settings.clone(),
            field: 0,
            editing: false,
            device: settings.device.clone().unwrap_or_default(),
            dns_hijack: settings.dns_hijack.join(","),
            mtu: settings
                .mtu
                .map(|value| value.to_string())
                .unwrap_or_default(),
            route_exclude_address: settings.route_exclude_address.join(","),
        })
    }

    pub(crate) fn dns(settings: &DnsSettings) -> Self {
        Self::Dns(DnsSettingsDialog {
            settings: settings.clone(),
            field: 0,
            editing: false,
            listen: settings.listen.clone().unwrap_or_default(),
            fake_ip_range: settings.fake_ip_range.clone().unwrap_or_default(),
            fake_ip_range6: settings.fake_ip_range6.clone().unwrap_or_default(),
        })
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> DialogAction {
        match self {
            Self::Tun(dialog) => dialog.handle_key(key),
            Self::Dns(dialog) => dialog.handle_key(key),
        }
    }

    pub(crate) fn editing(&self) -> bool {
        match self {
            Self::Tun(dialog) => dialog.editing,
            Self::Dns(dialog) => dialog.editing,
        }
    }

    pub(crate) fn value(&self) -> Result<NetworkSettings, String> {
        match self {
            Self::Tun(dialog) => dialog.value().map(NetworkSettings::Tun),
            Self::Dns(dialog) => Ok(NetworkSettings::Dns(dialog.value())),
        }
    }

    fn field(&self) -> usize {
        match self {
            Self::Tun(dialog) => dialog.field,
            Self::Dns(dialog) => dialog.field,
        }
    }

    fn rows(&self) -> Vec<(&'static str, String)> {
        match self {
            Self::Tun(dialog) => dialog.rows(),
            Self::Dns(dialog) => dialog.rows(),
        }
    }

    fn active_input(&self) -> Option<&str> {
        match self {
            Self::Tun(dialog) => dialog.active_input(),
            Self::Dns(dialog) => dialog.active_input(),
        }
    }

    fn title(&self) -> &'static str {
        match self {
            Self::Tun(_) => " TUN 高级设置 ",
            Self::Dns(_) => " DNS 高级设置 ",
        }
    }
}

impl TunSettingsDialog {
    const FIELD_COUNT: usize = 10;

    fn handle_key(&mut self, key: KeyEvent) -> DialogAction {
        if self.editing {
            return self.handle_edit_key(key);
        }
        match key.code {
            KeyCode::Esc => DialogAction::Close,
            KeyCode::Char('s') => DialogAction::Save,
            KeyCode::Tab | KeyCode::Char('j') | KeyCode::Down => {
                self.field = (self.field + 1) % Self::FIELD_COUNT;
                DialogAction::None
            }
            KeyCode::BackTab | KeyCode::Char('k') | KeyCode::Up => {
                self.field = (self.field + Self::FIELD_COUNT - 1) % Self::FIELD_COUNT;
                DialogAction::None
            }
            KeyCode::Left => {
                self.activate(-1);
                DialogAction::None
            }
            KeyCode::Right | KeyCode::Enter | KeyCode::Char(' ') => {
                self.activate(1);
                DialogAction::None
            }
            _ => DialogAction::None,
        }
    }

    fn handle_edit_key(&mut self, key: KeyEvent) -> DialogAction {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => self.editing = false,
            KeyCode::Backspace => {
                if let Some(input) = self.active_input_mut() {
                    input.pop();
                }
            }
            KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(input) = self.active_input_mut()
                    && input.len() < 1_024
                {
                    input.push(character);
                }
            }
            _ => {}
        }
        DialogAction::None
    }

    fn activate(&mut self, delta: isize) {
        match self.field {
            0 => self.settings.enable = !self.settings.enable,
            1 => cycle_option(
                &mut self.settings.stack,
                &["system", "gvisor", "mixed"],
                delta,
            ),
            2 | 7..=9 => self.editing = true,
            3 => self.settings.auto_route = !self.settings.auto_route,
            4 => self.settings.auto_redirect = !self.settings.auto_redirect,
            5 => self.settings.strict_route = !self.settings.strict_route,
            6 => self.settings.auto_detect_interface = !self.settings.auto_detect_interface,
            _ => {}
        }
    }

    fn active_input(&self) -> Option<&str> {
        if !self.editing {
            return None;
        }
        match self.field {
            2 => Some(&self.device),
            7 => Some(&self.dns_hijack),
            8 => Some(&self.mtu),
            9 => Some(&self.route_exclude_address),
            _ => None,
        }
    }

    fn active_input_mut(&mut self) -> Option<&mut String> {
        match self.field {
            2 => Some(&mut self.device),
            7 => Some(&mut self.dns_hijack),
            8 => Some(&mut self.mtu),
            9 => Some(&mut self.route_exclude_address),
            _ => None,
        }
    }

    fn rows(&self) -> Vec<(&'static str, String)> {
        vec![
            ("启用 TUN", toggle_label(self.settings.enable)),
            ("协议栈", format!("< {} >", self.settings.stack)),
            (
                "虚拟网卡名",
                input_label(&self.device, self.editing && self.field == 2),
            ),
            ("自动设置路由", toggle_label(self.settings.auto_route)),
            ("自动重定向", toggle_label(self.settings.auto_redirect)),
            ("严格路由", toggle_label(self.settings.strict_route)),
            (
                "自动识别出口",
                toggle_label(self.settings.auto_detect_interface),
            ),
            (
                "DNS 劫持",
                input_label(&self.dns_hijack, self.editing && self.field == 7),
            ),
            (
                "MTU",
                input_label(&self.mtu, self.editing && self.field == 8),
            ),
            (
                "排除路由网段",
                input_label(&self.route_exclude_address, self.editing && self.field == 9),
            ),
        ]
    }

    fn value(&self) -> Result<TunSettings, String> {
        let mut settings = self.settings.clone();
        settings.device = optional_input(&self.device);
        settings.dns_hijack = split_list(&self.dns_hijack);
        settings.route_exclude_address = split_list(&self.route_exclude_address);
        settings.mtu = if self.mtu.trim().is_empty() {
            None
        } else {
            Some(
                self.mtu
                    .trim()
                    .parse::<u32>()
                    .map_err(|_| "MTU 必须是正整数".to_string())?,
            )
        };
        if settings.mtu == Some(0) {
            return Err("MTU 必须大于 0".into());
        }
        Ok(settings)
    }
}

impl DnsSettingsDialog {
    const FIELD_COUNT: usize = 9;

    fn handle_key(&mut self, key: KeyEvent) -> DialogAction {
        if self.editing {
            return self.handle_edit_key(key);
        }
        match key.code {
            KeyCode::Esc => DialogAction::Close,
            KeyCode::Char('s') => DialogAction::Save,
            KeyCode::Tab | KeyCode::Char('j') | KeyCode::Down => {
                self.field = (self.field + 1) % Self::FIELD_COUNT;
                DialogAction::None
            }
            KeyCode::BackTab | KeyCode::Char('k') | KeyCode::Up => {
                self.field = (self.field + Self::FIELD_COUNT - 1) % Self::FIELD_COUNT;
                DialogAction::None
            }
            KeyCode::Left => {
                self.activate(-1);
                DialogAction::None
            }
            KeyCode::Right | KeyCode::Enter | KeyCode::Char(' ') => {
                self.activate(1);
                DialogAction::None
            }
            _ => DialogAction::None,
        }
    }

    fn handle_edit_key(&mut self, key: KeyEvent) -> DialogAction {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => self.editing = false,
            KeyCode::Backspace => {
                if let Some(input) = self.active_input_mut() {
                    input.pop();
                }
            }
            KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(input) = self.active_input_mut()
                    && input.len() < 1_024
                {
                    input.push(character);
                }
            }
            _ => {}
        }
        DialogAction::None
    }

    fn activate(&mut self, delta: isize) {
        match self.field {
            0 => self.settings.enable = !self.settings.enable,
            1 | 3 | 4 => self.editing = true,
            2 => cycle_option(
                &mut self.settings.enhanced_mode,
                &["redir-host", "fake-ip"],
                delta,
            ),
            5 => cycle_option(
                &mut self.settings.fake_ip_filter_mode,
                &["blacklist", "whitelist", "rule"],
                delta,
            ),
            6 => self.settings.ipv6 = !self.settings.ipv6,
            7 => self.settings.prefer_h3 = !self.settings.prefer_h3,
            8 => self.settings.respect_rules = !self.settings.respect_rules,
            _ => {}
        }
    }

    fn active_input(&self) -> Option<&str> {
        if !self.editing {
            return None;
        }
        match self.field {
            1 => Some(&self.listen),
            3 => Some(&self.fake_ip_range),
            4 => Some(&self.fake_ip_range6),
            _ => None,
        }
    }

    fn active_input_mut(&mut self) -> Option<&mut String> {
        match self.field {
            1 => Some(&mut self.listen),
            3 => Some(&mut self.fake_ip_range),
            4 => Some(&mut self.fake_ip_range6),
            _ => None,
        }
    }

    fn rows(&self) -> Vec<(&'static str, String)> {
        vec![
            ("启用 DNS", toggle_label(self.settings.enable)),
            (
                "监听地址",
                input_label(&self.listen, self.editing && self.field == 1),
            ),
            ("增强模式", format!("< {} >", self.settings.enhanced_mode)),
            (
                "Fake IP 范围",
                input_label(&self.fake_ip_range, self.editing && self.field == 3),
            ),
            (
                "Fake IP IPv6",
                input_label(&self.fake_ip_range6, self.editing && self.field == 4),
            ),
            (
                "Fake IP 过滤",
                format!("< {} >", self.settings.fake_ip_filter_mode),
            ),
            ("IPv6 解析", toggle_label(self.settings.ipv6)),
            ("优先 HTTP/3", toggle_label(self.settings.prefer_h3)),
            ("遵循路由规则", toggle_label(self.settings.respect_rules)),
        ]
    }

    fn value(&self) -> DnsSettings {
        let mut settings = self.settings.clone();
        settings.listen = optional_input(&self.listen);
        settings.fake_ip_range = optional_input(&self.fake_ip_range);
        settings.fake_ip_range6 = optional_input(&self.fake_ip_range6);
        settings
    }
}

impl RuleDialog {
    const FIELD_COUNT: usize = 3;

    pub(crate) fn new(
        mode: RuleDialogMode,
        original: Option<Rule>,
        mut policies: Vec<String>,
    ) -> Self {
        if let Some(rule) = &original {
            let current = rule.action.label();
            if !policies.iter().any(|policy| policy == current) {
                policies.insert(0, current.to_string());
            }
        }
        let kind = original
            .as_ref()
            .map(|rule| rule.kind.clone())
            .unwrap_or_else(|| "DOMAIN-SUFFIX".into());
        let value = original
            .as_ref()
            .map(|rule| rule.value.clone())
            .unwrap_or_default();
        let action = original
            .as_ref()
            .map(|rule| rule.action.label().to_string())
            .unwrap_or_else(|| policies[0].clone());
        Self {
            mode,
            field: 0,
            editing: false,
            selector: None,
            selector_index: 0,
            kind,
            value,
            action,
            policies,
            original,
        }
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> DialogAction {
        if self.selector.is_some() {
            return self.handle_selector_key(key);
        }
        if self.editing {
            return self.handle_edit_key(key);
        }
        match key.code {
            KeyCode::Esc => DialogAction::Close,
            KeyCode::Char('s') => DialogAction::Save,
            KeyCode::Tab | KeyCode::Char('j') | KeyCode::Down => {
                self.field = (self.field + 1) % Self::FIELD_COUNT;
                DialogAction::None
            }
            KeyCode::BackTab | KeyCode::Char('k') | KeyCode::Up => {
                self.field = (self.field + Self::FIELD_COUNT - 1) % Self::FIELD_COUNT;
                DialogAction::None
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                self.activate();
                DialogAction::None
            }
            _ => DialogAction::None,
        }
    }

    pub(crate) fn editing(&self) -> bool {
        self.editing
    }

    pub(crate) fn selecting(&self) -> bool {
        self.selector.is_some()
    }

    pub(crate) fn mode(&self) -> RuleDialogMode {
        self.mode
    }

    pub(crate) fn rule(&self) -> Result<Rule, String> {
        let value = if self.kind == "MATCH" {
            "all".to_string()
        } else {
            let value = self.value.trim();
            if value.is_empty() {
                return Err("规则匹配内容不能为空".into());
            }
            value.to_string()
        };
        let action = rule_action(&self.action);
        let extra = self
            .original
            .as_ref()
            .map(|rule| rule.extra.clone())
            .unwrap_or_default();
        let mut rule = Rule::new(&self.kind, value, action).with_extra(extra);
        if let Some(original) = &self.original {
            rule.enabled = original.enabled;
        }
        if let Some(original) = &self.original
            && original.kind == rule.kind
            && original.value == rule.value
            && original.action == rule.action
            && original.extra == rule.extra
        {
            rule.raw = original.raw.clone();
        }
        Ok(rule)
    }

    fn handle_edit_key(&mut self, key: KeyEvent) -> DialogAction {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => self.editing = false,
            KeyCode::Backspace => {
                self.value.pop();
            }
            KeyCode::Char(character)
                if !key.modifiers.contains(KeyModifiers::CONTROL) && self.value.len() < 4_096 =>
            {
                self.value.push(character);
            }
            _ => {}
        }
        DialogAction::None
    }

    fn handle_selector_key(&mut self, key: KeyEvent) -> DialogAction {
        let Some(selector) = self.selector else {
            return DialogAction::None;
        };
        let option_count = self.selector_option_count(selector);
        match key.code {
            KeyCode::Esc => self.selector = None,
            KeyCode::Char('j') | KeyCode::Down => {
                self.selector_index = (self.selector_index + 1).min(option_count - 1);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.selector_index = self.selector_index.saturating_sub(1);
            }
            KeyCode::Home => self.selector_index = 0,
            KeyCode::End => self.selector_index = option_count - 1,
            KeyCode::Enter | KeyCode::Char(' ') => self.confirm_selector(selector),
            _ => {}
        }
        DialogAction::None
    }

    fn activate(&mut self) {
        match self.field {
            0 => self.open_selector(RuleSelector::Kind),
            1 if self.kind != "MATCH" => self.editing = true,
            2 => self.open_selector(RuleSelector::Policy),
            _ => {}
        }
    }

    fn open_selector(&mut self, selector: RuleSelector) {
        self.selector_index = match selector {
            RuleSelector::Kind => RULE_KINDS
                .iter()
                .position(|option| *option == self.kind)
                .unwrap_or(0),
            RuleSelector::Policy => self
                .policies
                .iter()
                .position(|option| option == &self.action)
                .unwrap_or(0),
        };
        self.selector = Some(selector);
    }

    fn confirm_selector(&mut self, selector: RuleSelector) {
        match selector {
            RuleSelector::Kind => {
                self.kind = RULE_KINDS[self.selector_index].into();
                self.field = if self.kind == "MATCH" { 2 } else { 1 };
            }
            RuleSelector::Policy => {
                self.action = self.policies[self.selector_index].clone();
            }
        }
        self.selector = None;
    }

    fn selector_option_count(&self, selector: RuleSelector) -> usize {
        match selector {
            RuleSelector::Kind => RULE_KINDS.len(),
            RuleSelector::Policy => self.policies.len(),
        }
    }

    fn selector_options(&self) -> Option<(&'static str, Vec<&str>)> {
        match self.selector? {
            RuleSelector::Kind => Some(("选择规则类型", RULE_KINDS.to_vec())),
            RuleSelector::Policy => Some((
                "选择代理策略",
                self.policies.iter().map(String::as_str).collect(),
            )),
        }
    }

    fn rows(&self) -> Vec<(&'static str, String)> {
        vec![
            ("规则类型", format!("< {} >", self.kind)),
            (
                "匹配内容",
                if self.kind == "MATCH" {
                    "(无需匹配项)".into()
                } else {
                    input_label(&self.value, self.editing)
                },
            ),
            ("代理策略", format!("< {} >", self.action)),
        ]
    }

    fn active_input(&self) -> Option<&str> {
        (self.editing && self.field == 1).then_some(self.value.as_str())
    }

    fn title(&self) -> &'static str {
        match self.mode {
            RuleDialogMode::InsertFront => " 前置新增自定义规则 ",
            RuleDialogMode::InsertBack => " 后置新增自定义规则 ",
            RuleDialogMode::Edit(_) => " 编辑自定义规则 ",
        }
    }
}

pub(crate) fn draw_settings_dialog(frame: &mut Frame, dialog: &SettingsDialog) {
    let rows = dialog.rows();
    let height = u16::try_from(rows.len())
        .unwrap_or(u16::MAX)
        .saturating_add(4);
    draw_form_dialog(
        frame,
        dialog.title(),
        rows,
        dialog.field(),
        dialog.editing(),
        dialog.active_input(),
        height,
        "Enter/Space 修改  s 保存  Esc 取消",
    );
}

pub(crate) fn draw_rule_dialog(frame: &mut Frame, dialog: &RuleDialog) {
    draw_form_dialog(
        frame,
        dialog.title(),
        dialog.rows(),
        dialog.field,
        dialog.editing,
        dialog.active_input(),
        9,
        "Enter 打开列表/编辑  s 应用  Esc 取消",
    );
    draw_rule_selector(frame, dialog);
}

fn draw_rule_selector(frame: &mut Frame, dialog: &RuleDialog) {
    let Some((title, options)) = dialog.selector_options() else {
        return;
    };
    let selected = dialog.selector_index.min(options.len() - 1);
    let visible_count = options.len().min(8);
    let start = selected
        .saturating_sub(visible_count / 2)
        .min(options.len() - visible_count);
    let help = "↑/↓ 选择  Space/Enter 确认  Esc 返回";
    let content_width = options
        .iter()
        .map(|option| Line::from(*option).width())
        .chain([Line::from(help).width(), Line::from(title).width()])
        .max()
        .unwrap_or(0)
        .saturating_add(4);
    let width = u16::try_from(content_width)
        .unwrap_or(u16::MAX)
        .clamp(34, 60);
    let height = u16::try_from(visible_count)
        .unwrap_or(u16::MAX)
        .saturating_add(3);
    let area = centered_rect(width, height, frame.area());
    let block = Block::bordered().title(format!(" {title} ({}/{}) ", selected + 1, options.len()));
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);

    for (offset, option) in options.iter().skip(start).take(visible_count).enumerate() {
        let index = start + offset;
        let y = inner
            .y
            .saturating_add(u16::try_from(offset).unwrap_or(u16::MAX));
        let selected_row = index == selected;
        let style = if selected_row {
            Style::default().fg(Color::Black).bg(Color::Yellow)
        } else {
            Style::default().fg(Color::White)
        };
        frame.render_widget(
            Paragraph::new(format!("{} {option}", if selected_row { ">" } else { " " }))
                .style(style),
            Rect::new(inner.x, y, inner.width, 1),
        );
    }

    let help_y = inner
        .y
        .saturating_add(u16::try_from(visible_count).unwrap_or(u16::MAX));
    if help_y < inner.bottom() {
        frame.render_widget(
            Paragraph::new(help).style(Style::default().fg(Color::DarkGray)),
            Rect::new(inner.x, help_y, inner.width, 1),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_form_dialog(
    frame: &mut Frame,
    title: &str,
    rows: Vec<(&'static str, String)>,
    field: usize,
    editing: bool,
    active_input: Option<&str>,
    height: u16,
    idle_help: &str,
) {
    let area = centered_rect(76, height, frame.area());
    let block = Block::bordered().title(title);
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);

    let label_width = 22_u16.min(inner.width);
    let value_x = inner.x.saturating_add(label_width);
    let value_width = inner.width.saturating_sub(label_width);
    for (index, (label, value)) in rows.iter().enumerate() {
        let Ok(row_offset) = u16::try_from(index) else {
            break;
        };
        let y = inner.y.saturating_add(row_offset);
        if y >= inner.bottom() {
            break;
        }
        let selected = index == field;
        let style = if selected {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::White)
        };
        frame.render_widget(
            Paragraph::new(format!("{} {label}", if selected { ">" } else { " " })).style(style),
            Rect::new(inner.x, y, label_width, 1),
        );
        let available = usize::from(value_width.saturating_sub(1));
        let display_width = Line::from(value.as_str()).width();
        let scroll = if selected && editing {
            display_width.saturating_sub(available)
        } else {
            0
        };
        frame.render_widget(
            Paragraph::new(value.as_str())
                .style(style)
                .scroll((0, u16::try_from(scroll).unwrap_or(u16::MAX))),
            Rect::new(value_x, y, value_width, 1),
        );
    }

    let help_y = inner
        .y
        .saturating_add(u16::try_from(rows.len()).unwrap_or(u16::MAX))
        .saturating_add(1);
    if help_y < inner.bottom() {
        frame.render_widget(
            Paragraph::new(if editing {
                "输入内容；Enter 完成，Esc 停止编辑"
            } else {
                idle_help
            })
            .style(Style::default().fg(Color::DarkGray)),
            Rect::new(inner.x, help_y, inner.width, 1),
        );
    }

    if let Some(input) = active_input {
        let input_width = Line::from(input).width();
        let available = usize::from(value_width.saturating_sub(1));
        frame.set_cursor_position(Position::new(
            value_x.saturating_add(u16::try_from(input_width.min(available)).unwrap_or(u16::MAX)),
            inner
                .y
                .saturating_add(u16::try_from(field).unwrap_or(u16::MAX)),
        ));
    }
}

pub(crate) fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width.saturating_sub(2));
    let height = height.min(area.height.saturating_sub(2));
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

fn cycle_option(value: &mut String, options: &[&str], delta: isize) {
    let index = options
        .iter()
        .position(|option| *option == value)
        .unwrap_or(0);
    let next = (index as isize + delta).rem_euclid(options.len() as isize) as usize;
    *value = options[next].into();
}

fn toggle_label(enabled: bool) -> String {
    if enabled {
        "[x] 开启".into()
    } else {
        "[ ] 关闭".into()
    }
}

fn input_label(input: &str, editing: bool) -> String {
    if input.is_empty() && !editing {
        "(未配置)".into()
    } else {
        input.to_string()
    }
}

fn optional_input(input: &str) -> Option<String> {
    let value = input.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn split_list(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn rule_action(label: &str) -> RuleAction {
    match label {
        "PROXY" => RuleAction::Proxy,
        "DIRECT" => RuleAction::Direct,
        "REJECT" => RuleAction::Reject,
        "REJECT-DROP" => RuleAction::RejectDrop,
        group => RuleAction::Group(group.into()),
    }
}
