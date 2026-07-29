use crate::mihomo::MihomoClient;
use crate::models::{AppState, Page};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{prelude::*, widgets::*};
use std::time::{Duration, Instant};

pub struct App {
    pub state: AppState,
    client: Option<MihomoClient>,
    last_refresh: Instant,
    proxy_members_focused: bool,
    selected_proxy_member_index: usize,
}

impl App {
    pub fn new(controller: Option<String>, secret: Option<String>) -> Self {
        let mut state = AppState::demo();
        let client = controller
            .as_ref()
            .and_then(|url| MihomoClient::new(url, secret).ok());
        if let Some(url) = controller {
            state.controller = url;
            state.status = "Connecting to Mihomo...".into();
        }
        Self {
            state,
            client,
            last_refresh: Instant::now() - Duration::from_secs(10),
            proxy_members_focused: false,
            selected_proxy_member_index: 0,
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
                self.state.status = "Connected - proxies refreshed".into();
            }
            Err(error) => self.state.status = format!("API error: {error}"),
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return true;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => true,
            KeyCode::Tab => {
                self.state.page = match self.state.page {
                    Page::Dashboard => Page::Proxies,
                    Page::Proxies => Page::Rules,
                    Page::Rules => Page::Dashboard,
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
            KeyCode::Char('r') => {
                self.last_refresh = Instant::now() - Duration::from_secs(10);
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
        };
        if len == 0 {
            return;
        }
        self.state.selected =
            ((self.state.selected as isize + delta).rem_euclid(len as isize)) as usize;
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
        let tabs = Tabs::new(vec!["1 Dashboard", "2 Proxies", "3 Rules"])
            .select(match self.state.page {
                Page::Dashboard => 0,
                Page::Proxies => 1,
                Page::Rules => 2,
            })
            .block(Block::bordered().title(" mihomo-tui "))
            .highlight_style(Style::default().fg(Color::Yellow));
        frame.render_widget(tabs, root[0]);
        match self.state.page {
            Page::Dashboard => self.dashboard(frame, root[1]),
            Page::Proxies => self.proxies(frame, root[1]),
            Page::Rules => self.rules(frame, root[1]),
        }
        frame.render_widget(
            Paragraph::new(format!(
                " {} | Tab/1-3 page  j/k move  Right/Enter nodes  Left groups  Enter apply  r refresh  q quit",
                self.state.status
            ))
            .style(Style::default().fg(Color::Gray)),
            root[2],
        );
    }

    fn dashboard(&self, frame: &mut Frame, area: Rect) {
        let profile = self.state.profiles.first();
        let rows = vec![
            ListItem::new(format!("Core       Mihomo API")),
            ListItem::new(format!("Controller {}", self.state.controller)),
            ListItem::new(format!("Proxy groups {}", self.state.proxies.len())),
            ListItem::new(format!(
                "Rules      {} enabled / {} total",
                self.state.rules.rules.iter().filter(|r| r.enabled).count(),
                self.state.rules.rules.len()
            )),
            ListItem::new(format!("Profiles   {}", self.state.profiles.len())),
            ListItem::new(format!(
                "Active     {} ({})",
                profile.map(|p| p.name.as_str()).unwrap_or("none"),
                profile.map(|p| p.kind.as_str()).unwrap_or("-")
            )),
            ListItem::new(format!(
                "Source     {} | updated {} | {}",
                profile.map(|p| p.source.as_str()).unwrap_or("-"),
                profile.map(|p| p.updated.as_str()).unwrap_or("-"),
                if profile.is_some_and(|p| p.enabled) {
                    "enabled"
                } else {
                    "disabled"
                }
            )),
        ];
        frame.render_widget(
            List::new(rows)
                .block(Block::bordered().title(" Overview "))
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
            Row::new(vec!["Group", "Type", "Current", "Delay", "Nodes"])
                .style(Style::default().fg(Color::Yellow)),
        )
        .row_highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White))
        .block(Block::bordered().title(" Proxy groups "));
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
            .map(|proxy| format!(" Nodes: {} ", proxy.name))
            .unwrap_or_else(|| " Nodes ".into());
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
            Row::new(vec!["State", "Type", "Match", "Action"])
                .style(Style::default().fg(Color::Yellow)),
        )
        .row_highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White))
        .block(Block::bordered().title(" Rules (Space toggle, J/K reorder) "));
        frame.render_stateful_widget(
            table,
            area,
            &mut TableState::default().with_selected(Some(self.state.selected)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_member_selection_starts_at_the_current_member() {
        let app = App::new(None, None);

        assert_eq!(app.selected_proxy_member(), Some("Tokyo-01"));
    }

    #[test]
    fn proxy_member_navigation_does_not_change_the_proxy_group() {
        let mut app = App::new(None, None);

        app.open_proxy_members();
        app.move_proxy_member(1);

        assert_eq!(app.state.selected, 0);
        assert_eq!(app.selected_proxy_member(), Some("Singapore-02"));
    }
}
