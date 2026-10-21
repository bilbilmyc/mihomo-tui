use crate::mihomo::MihomoClient;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{prelude::*, widgets::*};
use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::Duration,
};

const MAX_LOGS: usize = 500;
const QUEUE_SIZE: usize = 128;

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub level: String,
    pub payload: String,
}

impl LogEntry {
    pub fn parse(bytes: &[u8], secret: Option<&str>) -> Result<Self, String> {
        let value: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|_| "日志流含有无效 JSON（原文已隐藏）")?;
        let level = value
            .get("type")
            .and_then(serde_json::Value::as_str)
            .ok_or("日志缺少级别")?;
        if !matches!(level, "info" | "warning" | "error" | "debug") {
            return Err("日志级别不可识别".into());
        }
        let payload = value
            .get("payload")
            .and_then(serde_json::Value::as_str)
            .ok_or("日志缺少字符串内容")?;
        Ok(Self {
            level: level.into(),
            payload: crate::sanitize::text(payload, secret),
        })
    }
}

#[derive(Debug)]
enum LogEvent {
    Entry(LogEntry),
    State(String),
}

#[derive(Default)]
struct Signals {
    active: AtomicBool,
    stop: AtomicBool,
    epoch: AtomicU64,
    dropped: AtomicU64,
}

pub struct LogsView {
    entries: VecDeque<LogEntry>,
    frozen: Option<Vec<LogEntry>>,
    offset: usize,
    state: String,
    active: bool,
    started: bool,
    signals: Arc<Signals>,
    tx: SyncSender<(u64, LogEvent)>,
    rx: Receiver<(u64, LogEvent)>,
}

impl Default for LogsView {
    fn default() -> Self {
        let (tx, rx) = mpsc::sync_channel(QUEUE_SIZE);
        Self {
            entries: VecDeque::new(),
            frozen: None,
            offset: 0,
            state: "尚未订阅；仅接收进入页面后的实时日志".into(),
            active: false,
            started: false,
            signals: Arc::default(),
            tx,
            rx,
        }
    }
}

impl Drop for LogsView {
    fn drop(&mut self) {
        self.signals.stop.store(true, Ordering::Relaxed);
        self.signals.active.store(false, Ordering::Relaxed);
    }
}

impl LogsView {
    pub fn sync(&mut self, client: Option<MihomoClient>, active: bool) {
        if self.active != active {
            self.active = active;
            self.signals.epoch.fetch_add(1, Ordering::Relaxed);
            self.signals.active.store(active, Ordering::Relaxed);
            self.state = if active {
                "正在订阅实时日志；空闲流会周期重连"
            } else {
                "已离开日志页，正在停止订阅；仅保留内存缓冲"
            }
            .into();
        }
        if active && !self.started {
            if let Some(client) = client {
                self.started = true;
                let signals = self.signals.clone();
                let sender = self.tx.clone();
                thread::spawn(move || log_worker(client, signals, sender));
            } else {
                self.state = "未连接 Controller，无法订阅日志".into();
            }
        }
        // A hot producer must not starve terminal input while the queue is drained.
        for _ in 0..QUEUE_SIZE {
            let Ok((epoch, event)) = self.rx.try_recv() else {
                break;
            };
            self.accept(epoch, event);
        }
    }

    fn accept(&mut self, epoch: u64, event: LogEvent) {
        if epoch != self.signals.epoch.load(Ordering::Relaxed) {
            return;
        }
        match event {
            LogEvent::Entry(entry) => {
                self.push(entry);
                self.state = "正在接收实时日志".into();
            }
            LogEvent::State(state) => self.state = state,
        }
    }

    fn push(&mut self, entry: LogEntry) {
        if self.entries.len() >= MAX_LOGS {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    fn visible(&self) -> Vec<&LogEntry> {
        match &self.frozen {
            Some(entries) => entries.iter().collect(),
            None => self.entries.iter().collect(),
        }
    }

    fn toggle_pause(&mut self) {
        self.frozen = if self.frozen.is_some() {
            None
        } else {
            Some(self.entries.iter().cloned().collect())
        };
        self.offset = 0;
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.frozen = None;
        self.offset = 0;
        self.signals.epoch.fetch_add(1, Ordering::Relaxed);
        self.signals.dropped.store(0, Ordering::Relaxed);
        self.state = "已清空本地缓冲；等待新订阅，未清除核心日志".into();
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char(' ') => self.toggle_pause(),
            KeyCode::Char('c') => self.clear(),
            KeyCode::Char('r') => {
                self.signals.epoch.fetch_add(1, Ordering::Relaxed);
                self.state = "正在重新订阅实时日志".into();
            }
            KeyCode::Char('k') | KeyCode::Up => self.offset = (self.offset + 1).min(MAX_LOGS),
            KeyCode::Char('j') | KeyCode::Down => self.offset = self.offset.saturating_sub(1),
            KeyCode::PageUp => self.offset = (self.offset + 10).min(MAX_LOGS),
            KeyCode::PageDown => self.offset = self.offset.saturating_sub(10),
            KeyCode::End => self.offset = 0,
            _ => return false,
        }
        true
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect) {
        let block = Block::bordered().title(" 实时日志 · 已脱敏 · 仅内存缓冲 ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let parts = Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).split(inner);
        frame.render_widget(
            Paragraph::new(format!(
                "{}{}\n仅保留最近500条；非历史日志；队列丢弃 {} 条",
                self.state,
                if self.frozen.is_some() {
                    " · 暂停展示（仍收集）"
                } else {
                    ""
                },
                self.signals.dropped.load(Ordering::Relaxed)
            )),
            parts[0],
        );
        let entries = self.visible();
        let height = usize::from(parts[1].height);
        let offset = self.offset.min(entries.len().saturating_sub(height));
        let end = entries.len().saturating_sub(offset);
        let start = end.saturating_sub(height);
        let lines = entries[start..end]
            .iter()
            .map(|entry| {
                Line::styled(
                    format!("[{}] {}", entry.level, entry.payload),
                    Style::default().fg(match entry.level.as_str() {
                        "error" => Color::Red,
                        "warning" => Color::Yellow,
                        _ => Color::Gray,
                    }),
                )
            })
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(lines), parts[1]);
    }
}

#[derive(Default)]
struct IdleBudget {
    epoch: u64,
    failures: usize,
}

impl IdleBudget {
    fn record(&mut self, epoch: u64, received: bool) {
        if self.epoch != epoch {
            self.epoch = epoch;
            self.failures = 0;
        }
        self.failures = if received { 0 } else { self.failures + 1 };
    }

    fn exhausted(&self, epoch: u64) -> bool {
        self.epoch == epoch && self.failures >= 3
    }
}

fn log_worker(client: MihomoClient, signals: Arc<Signals>, sender: SyncSender<(u64, LogEvent)>) {
    let mut budget = IdleBudget::default();
    while !signals.stop.load(Ordering::Relaxed) {
        if !signals.active.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(50));
            continue;
        }
        let epoch = signals.epoch.load(Ordering::Relaxed);
        if budget.exhausted(epoch) {
            thread::sleep(Duration::from_millis(50));
            continue;
        }
        let cancelled = || {
            signals.stop.load(Ordering::Relaxed)
                || !signals.active.load(Ordering::Relaxed)
                || signals.epoch.load(Ordering::Relaxed) != epoch
        };
        let send = |event| {
            if sender.try_send((epoch, event)).is_err() {
                signals.dropped.fetch_add(1, Ordering::Relaxed);
            }
        };
        let received = std::cell::Cell::new(false);
        let result = client.log_stream(cancelled, |entry| {
            received.set(true);
            send(LogEvent::Entry(entry));
        });
        if !cancelled() {
            budget.record(epoch, received.get());
            if budget.exhausted(epoch) {
                send(LogEvent::State(
                    "连续3次未收到日志或连接失败，已暂停自动重连；按 r 重试".into(),
                ));
                continue;
            }
            send(LogEvent::State(match result {
                Ok(()) => "日志流已结束，稍后重新订阅（不补历史）".into(),
                Err(error) => format!("{error}；稍后重连，期间可能遗漏日志"),
            }));
        }
        for _ in 0..20 {
            if cancelled() {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_payload_is_typed_sanitized_and_bounded() {
        let entry = LogEntry::parse(br#"{"type":"warning","payload":"failed https://sub.example/private?token=hidden fixture-secret"}"#, Some("fixture-secret")).unwrap();
        assert!(!entry.payload.contains("hidden"));
        assert!(!entry.payload.contains("fixture-secret"));
        assert_eq!(entry.level, "warning");
        assert!(LogEntry::parse(br#"{"type":"info","payload":42}"#, None).is_err());
        assert!(LogEntry::parse(b"not-json", None).is_err());
    }

    #[test]
    fn ring_buffer_pause_and_clear_remain_bounded() {
        let mut view = LogsView::default();
        for index in 0..MAX_LOGS + 20 {
            view.push(LogEntry {
                level: "info".into(),
                payload: index.to_string(),
            });
        }
        assert_eq!(view.entries.len(), MAX_LOGS);
        view.toggle_pause();
        let frozen = view.visible().last().unwrap().payload.clone();
        view.push(LogEntry {
            level: "info".into(),
            payload: "new".into(),
        });
        assert_eq!(view.visible().last().unwrap().payload, frozen);
        view.toggle_pause();
        assert_eq!(view.visible().last().unwrap().payload, "new");
        view.clear();
        assert!(view.entries.is_empty());
    }

    #[test]
    fn old_subscription_messages_cannot_repopulate_a_cleared_buffer() {
        let mut view = LogsView::default();
        let epoch = view.signals.epoch.load(Ordering::Relaxed);
        view.clear();
        view.accept(
            epoch,
            LogEvent::Entry(LogEntry {
                level: "info".into(),
                payload: "stale".into(),
            }),
        );
        assert!(view.entries.is_empty());
    }

    #[test]
    fn dropping_the_view_signals_cancellation_without_waiting_for_network_io() {
        let view = LogsView::default();
        let signals = view.signals.clone();
        drop(view);
        assert!(signals.stop.load(Ordering::Relaxed));
    }

    #[test]
    fn the_transport_queue_is_bounded_even_when_the_terminal_does_not_drain_it() {
        let view = LogsView::default();
        for _ in 0..QUEUE_SIZE {
            view.tx
                .try_send((0, LogEvent::State("fixture".into())))
                .unwrap();
        }
        assert!(matches!(
            view.tx.try_send((0, LogEvent::State("overflow".into()))),
            Err(mpsc::TrySendError::Full(_))
        ));
    }

    #[test]
    fn quiet_or_failing_streams_have_a_finite_retry_budget_per_subscription() {
        let mut budget = IdleBudget::default();
        for _ in 0..3 {
            budget.record(1, false);
        }
        assert!(budget.exhausted(1));
        assert!(!budget.exhausted(2));
        budget.record(2, false);
        budget.record(2, true);
        assert!(!budget.exhausted(2));
    }
}
