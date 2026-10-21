use super::*;
use serde_json::json;

fn snapshot(ids: &[&str]) -> ConnectionSnapshot {
    ConnectionSnapshot::parse(&json!({"connections": ids.iter().map(|id| json!({
        "id": id, "metadata": {"host":format!("{id}.example"),"network":"tcp","type":"Tun"},
        "chains":["JP-HY2","Proxy"], "rule":"DomainKeyword", "rulePayload":"github"
    })).collect::<Vec<_>>()}))
    .unwrap()
}

#[test]
fn connection_chain_is_displayed_from_group_to_exit() {
    let snapshot = snapshot(&["a"]);
    assert_eq!(snapshot.connections[0].route(), "Proxy → JP-HY2");
    assert_eq!(snapshot.connections[0].inbound, "Tun");
}

#[test]
fn malformed_snapshots_are_not_mistaken_for_empty_connection_lists() {
    assert!(ConnectionSnapshot::parse(&json!({})).is_err());
    assert!(ConnectionSnapshot::parse(&json!({"connections":"invalid"})).is_err());
    assert!(ConnectionSnapshot::parse(&json!({"connections":[{"id":"a"}]})).is_err());
    assert!(
        ConnectionSnapshot::parse(&json!({"connections":null}))
            .unwrap()
            .connections
            .is_empty()
    );
}

#[test]
fn selection_survives_refresh_reordering_and_filtering() {
    let mut view = ConnectionsView::default();
    view.update(Ok(snapshot(&["a", "b"])));
    view.move_selection(1);
    assert_eq!(view.selected_id.as_deref(), Some("b"));
    view.update(Ok(snapshot(&["b", "a"])));
    assert_eq!(view.selected_id.as_deref(), Some("b"));
    view.filter = "a.example".into();
    assert_eq!(view.filtered().len(), 1);
    view.update(Err("网络暂不可用".into()));
    assert!(view.snapshot.is_some());
    assert!(view.error.is_some());
}

#[test]
fn connection_snapshots_are_bounded_and_controls_are_sanitized() {
    let ids = vec!["a"; MAX_CONNECTIONS + 1];
    let value = snapshot(&ids);
    assert_eq!(value.connections.len(), MAX_CONNECTIONS);
    assert!(value.truncated);
    let value = ConnectionSnapshot::parse(&json!({"connections":[{
        "id":"x","metadata":{"host":"github.com\u{001b}[2J","sourcePort":"12345"}
    }]}))
    .unwrap();
    assert!(!value.connections[0].host.contains('\u{001b}'));
    assert_eq!(value.connections[0].source_port, Some(12345));
}
