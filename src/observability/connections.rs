use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{prelude::*, widgets::*};
use serde_json::Value;
use std::time::Instant;

pub const MAX_CONNECTIONS: usize = 512;

#[derive(Debug, Clone)]
pub struct Connection {
    pub id: String,
    pub host: String,
    pub destination: String,
    pub source_port: Option<u16>,
    pub network: String,
    pub inbound: String,
    pub rule: String,
    pub rule_payload: String,
    pub chains: Vec<String>,
    pub upload: u64,
    pub download: u64,
}

impl Connection {
    pub fn target(&self) -> &str {
        if self.host.is_empty() {
            &self.destination
        } else {
            &self.host
        }
    }

    pub fn route(&self) -> String {
        if self.chains.is_empty() {
            "链路未知".into()
        } else {
            self.chains
                .iter()
                .rev()
                .cloned()
                .collect::<Vec<_>>()
                .join(" → ")
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConnectionSnapshot {
    pub connections: Vec<Connection>,
    pub upload_total: u64,
    pub download_total: u64,
    pub truncated: bool,
}

impl ConnectionSnapshot {
    pub fn parse(value: &Value) -> Result<Self, String> {
        let values = match value.get("connections") {
            Some(Value::Array(values)) => values.as_slice(),
            Some(Value::Null) => &[],
            _ => return Err("连接响应缺少有效的 connections 列表".into()),
        };
        let mut connections = Vec::new();
        for value in values.iter().take(MAX_CONNECTIONS) {
            let id = value
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty() && id.len() <= 256)
                .ok_or("连接缺少有效 ID")?;
            let metadata = value
                .get("metadata")
                .filter(|metadata| metadata.is_object())
                .ok_or("连接缺少 metadata 对象")?;
            let text = |object: &Value, field| {
                crate::sanitize::text(
                    object.get(field).and_then(Value::as_str).unwrap_or(""),
                    None,
                )
            };
            let source_port = metadata
                .get("sourcePort")
                .and_then(|port| port.as_u64().or_else(|| port.as_str()?.parse().ok()))
                .and_then(|port| u16::try_from(port).ok());
            let chains = match value.get("chains") {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::Array(chains)) if chains.len() <= 32 => chains
                    .iter()
                    .map(|chain| {
                        chain
                            .as_str()
                            .map(|chain| crate::sanitize::text(chain, None))
                            .ok_or("链路条目不是字符串")
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                _ => return Err("连接链路格式不可识别或过长".into()),
            };
            connections.push(Connection {
                id: id.into(),
                host: text(metadata, "host"),
                destination: text(metadata, "destinationIP"),
                source_port,
                network: text(metadata, "network"),
                inbound: text(metadata, "type"),
                rule: text(value, "rule"),
                rule_payload: text(value, "rulePayload"),
                chains,
                upload: value.get("upload").and_then(Value::as_u64).unwrap_or(0),
                download: value.get("download").and_then(Value::as_u64).unwrap_or(0),
            });
        }
        Ok(Self {
            connections,
            upload_total: value
                .get("uploadTotal")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            download_total: value
                .get("downloadTotal")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            truncated: values.len() > MAX_CONNECTIONS,
        })
    }
}

#[derive(Default)]
pub struct ConnectionsView {
    pub snapshot: Option<ConnectionSnapshot>,
    pub error: Option<String>,
    pub loading: bool,
    pub filter: String,
    pub filtering: bool,
    pub selected_id: Option<String>,
    pub updated: Option<Instant>,
    details: bool,
}

impl ConnectionsView {
    pub fn filtered(&self) -> Vec<&Connection> {
        let filter = self.filter.to_lowercase();
        self.snapshot
            .iter()
            .flat_map(|snapshot| &snapshot.connections)
            .filter(|connection| {
                filter.is_empty()
                    || format!(
                        "{} {} {} {} {}",
                        connection.target(),
                        connection.inbound,
                        connection.rule,
                        connection.rule_payload,
                        connection.route()
                    )
                    .to_lowercase()
                    .contains(&filter)
            })
            .collect()
    }

    pub fn update(&mut self, result: Result<ConnectionSnapshot, String>) {
        self.loading = false;
        match result {
            Ok(snapshot) => {
                self.snapshot = Some(snapshot);
                self.error = None;
                self.updated = Some(Instant::now());
                let rows = self.filtered();
                if !rows
                    .iter()
                    .any(|connection| Some(&connection.id) == self.selected_id.as_ref())
                {
                    self.selected_id = rows.first().map(|connection| connection.id.clone());
                }
            }
            Err(error) => self.error = Some(error),
        }
    }

    pub fn move_selection(&mut self, delta: isize) {
        let rows = self.filtered();
        if rows.is_empty() {
            self.selected_id = None;
            return;
        }
        let index = rows
            .iter()
            .position(|connection| Some(&connection.id) == self.selected_id.as_ref())
            .unwrap_or(0);
        let next = (index as isize + delta).rem_euclid(rows.len() as isize) as usize;
        self.selected_id = Some(rows[next].id.clone());
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        if self.filtering {
            match key.code {
                KeyCode::Enter | KeyCode::Esc => self.filtering = false,
                KeyCode::Backspace => {
                    self.filter.pop();
                }
                KeyCode::Char(character)
                    if !character.is_control() && self.filter.chars().count() < 128 =>
                {
                    self.filter.push(character)
                }
                _ => {}
            }
            self.selected_id = self
                .filtered()
                .first()
                .map(|connection| connection.id.clone());
            return true;
        }
        match key.code {
            KeyCode::Char('/') => self.filtering = true,
            KeyCode::Char('c') => self.filter.clear(),
            KeyCode::Char('j') | KeyCode::Down => self.move_selection(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_selection(-1),
            KeyCode::Enter => self.details = !self.details,
            _ => return false,
        }
        true
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect) {
        let block = Block::bordered().title(" 活跃连接 · 只读快照 ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let parts = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(0),
            Constraint::Length(if self.details { 6 } else { 0 }),
        ])
        .split(inner);
        let mut status = if let Some(error) = &self.error {
            format!("采集失败：{error}；旧快照仅供参考")
        } else if self.loading && self.snapshot.is_none() {
            "正在加载活跃连接…".into()
        } else if let Some(updated) = self.updated {
            format!("{} 秒前采集；不是连接历史", updated.elapsed().as_secs())
        } else {
            "尚未采集；r 刷新".into()
        };
        if self
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.truncated)
        {
            status.push_str(" · 已截断至512条");
        }
        frame.render_widget(
            Paragraph::new(format!(
                "{status}\n{}过滤：{}",
                if self.filtering {
                    "正在输入 · "
                } else {
                    ""
                },
                self.filter
            )),
            parts[0],
        );
        let rows = self.filtered();
        if rows.is_empty() {
            let empty = if self.snapshot.is_some() {
                "没有符合条件的活跃连接"
            } else {
                "暂无连接快照"
            };
            frame.render_widget(Paragraph::new(empty), parts[1]);
            return;
        }
        let selected = rows
            .iter()
            .position(|connection| Some(&connection.id) == self.selected_id.as_ref())
            .unwrap_or(0);
        let entries = rows
            .iter()
            .map(|connection| {
                ListItem::new(format!(
                    "{} | {} {} | {} → {}",
                    connection.target(),
                    connection.network,
                    connection.inbound,
                    connection.rule,
                    connection.route()
                ))
            })
            .collect::<Vec<_>>();
        frame.render_stateful_widget(
            List::new(entries)
                .highlight_style(Style::default().fg(Color::Yellow))
                .highlight_symbol("› "),
            parts[1],
            &mut ListState::default().with_selected(Some(selected)),
        );
        if self.details {
            let connection = rows[selected];
            let snapshot = self.snapshot.as_ref().unwrap();
            frame.render_widget(Paragraph::new(format!("目标 {} ({})\n入口 {} / {} | 源端口 {:?}\n规则 {} {}\n链路 {}\n本连接 ↑{} ↓{} B | 核心累计 ↑{} ↓{} B{}",
                connection.target(), connection.destination, connection.inbound, connection.network, connection.source_port,
                connection.rule, connection.rule_payload, connection.route(), connection.upload, connection.download,
                snapshot.upload_total, snapshot.download_total, if snapshot.truncated { " · 已截断至512条" } else { "" }))
                .wrap(Wrap { trim: true }), parts[2]);
        }
    }
}
