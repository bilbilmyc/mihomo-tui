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
    Group(String),
}

impl RuleAction {
    pub fn label(&self) -> &str {
        match self {
            Self::Proxy => "PROXY",
            Self::Direct => "DIRECT",
            Self::Reject => "REJECT",
            Self::Group(name) => name,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub kind: String,
    pub value: String,
    pub action: RuleAction,
    pub enabled: bool,
}

impl Rule {
    pub fn new(kind: impl Into<String>, value: impl Into<String>, action: RuleAction) -> Self {
        Self {
            kind: kind.into(),
            value: value.into(),
            action,
            enabled: true,
        }
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
                },
                ProxySummary {
                    name: "Auto".into(),
                    proxy_type: "URLTest".into(),
                    now: Some("Singapore-02".into()),
                    delay_ms: Some(112),
                    members: vec!["Tokyo-01".into(), "Singapore-02".into()],
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
    }
}
