use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Dashboard,
    Proxies,
    Rules,
    Config,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuleAction {
    Proxy,
    Direct,
    Reject,
    RejectDrop,
    Group(String),
}

impl RuleAction {
    pub fn label(&self) -> &str {
        match self {
            Self::Proxy => "PROXY",
            Self::Direct => "DIRECT",
            Self::Reject => "REJECT",
            Self::RejectDrop => "REJECT-DROP",
            Self::Group(name) => name,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub kind: String,
    pub value: String,
    pub action: RuleAction,
    pub extra: Vec<String>,
    pub enabled: bool,
    pub raw: Option<String>,
}

impl Rule {
    pub fn new(kind: impl Into<String>, value: impl Into<String>, action: RuleAction) -> Self {
        Self {
            kind: kind.into(),
            value: value.into(),
            action,
            extra: Vec::new(),
            enabled: true,
            raw: None,
        }
    }

    pub fn with_raw(mut self, raw: Option<String>) -> Self {
        self.raw = raw;
        self
    }

    pub fn with_extra(mut self, extra: Vec<String>) -> Self {
        self.extra = extra;
        self
    }
}

#[derive(Debug, Default, Clone)]
pub struct RuleSet {
    pub rules: Vec<Rule>,
}

impl RuleSet {
    pub fn move_rule(&mut self, index: usize, delta: isize) -> bool {
        let target = index as isize + delta;
        if !(0..self.rules.len() as isize).contains(&target) {
            return false;
        }
        self.rules.swap(index, target as usize);
        true
    }

    pub fn toggle(&mut self, index: usize) -> bool {
        if let Some(rule) = self.rules.get_mut(index) {
            rule.enabled = !rule.enabled;
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProxySummary {
    pub name: String,
    pub proxy_type: String,
    pub now: Option<String>,
    pub delay_ms: Option<u64>,
    pub members: Vec<String>,
    pub member_delays: std::collections::BTreeMap<String, ProxyDelay>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxyDelay {
    Measured(u64),
    Timeout,
    Failed,
}

impl ProxyDelay {
    pub fn label(&self) -> String {
        match self {
            Self::Measured(delay) => format!("{delay} ms"),
            Self::Timeout => "timeout".into(),
            Self::Failed => "failed".into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub page: Page,
    pub proxies: Vec<ProxySummary>,
    pub rules: RuleSet,
    pub selected: usize,
    pub status: String,
    pub controller: String,
}

impl AppState {
    pub fn live(controller: impl Into<String>) -> Self {
        Self {
            page: Page::Dashboard,
            proxies: Vec::new(),
            rules: RuleSet::default(),
            selected: 0,
            status: "等待连接 Mihomo...".into(),
            controller: controller.into(),
        }
    }

    pub fn demo() -> Self {
        Self {
            page: Page::Dashboard,
            proxies: vec![
                ProxySummary {
                    name: "Proxy".into(),
                    proxy_type: "Selector".into(),
                    now: Some("Tokyo-01".into()),
                    delay_ms: Some(86),
                    members: vec!["Tokyo-01".into(), "Singapore-02".into(), "DIRECT".into()],
                    member_delays: std::collections::BTreeMap::new(),
                },
                ProxySummary {
                    name: "Auto".into(),
                    proxy_type: "URLTest".into(),
                    now: Some("Singapore-02".into()),
                    delay_ms: Some(112),
                    members: vec!["Tokyo-01".into(), "Singapore-02".into()],
                    member_delays: std::collections::BTreeMap::new(),
                },
            ],
            rules: RuleSet {
                rules: vec![
                    Rule::new("DOMAIN-SUFFIX", "github.com", RuleAction::Proxy),
                    Rule::new("DOMAIN-SUFFIX", "baidu.com", RuleAction::Direct),
                    Rule::new("GEOSITE", "cn", RuleAction::Direct),
                    Rule::new("MATCH", "all", RuleAction::Group("Proxy".into())),
                ],
            },
            selected: 0,
            status: "Demo mode - pass --controller to connect Mihomo".into(),
            controller: "demo".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_can_be_reordered_and_disabled() {
        let mut rules = RuleSet {
            rules: vec![
                Rule::new("DOMAIN", "a.example", RuleAction::Proxy),
                Rule::new("DOMAIN", "b.example", RuleAction::Direct),
            ],
        };
        assert!(rules.move_rule(1, -1));
        assert_eq!(rules.rules[0].value, "b.example");
        assert!(rules.toggle(0));
        assert!(!rules.rules[0].enabled);
        assert!(!rules.move_rule(0, -1));
    }

    #[test]
    fn action_labels_are_stable() {
        assert_eq!(RuleAction::Proxy.label(), "PROXY");
        assert_eq!(RuleAction::Group("Work".into()).label(), "Work");
        assert_eq!(RuleAction::RejectDrop.label(), "REJECT-DROP");
    }

    #[test]
    fn proxy_delay_labels_distinguish_timeouts_from_missing_data() {
        assert_eq!(ProxyDelay::Measured(86).label(), "86 ms");
        assert_eq!(ProxyDelay::Timeout.label(), "timeout");
    }

    #[test]
    fn live_state_has_no_example_data() {
        let state = AppState::live("http://127.0.0.1:9090");

        assert!(state.proxies.is_empty());
        assert!(state.rules.rules.is_empty());
        assert_eq!(state.controller, "http://127.0.0.1:9090");
    }
}
