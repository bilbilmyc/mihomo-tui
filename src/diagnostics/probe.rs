use super::{Check, Status};
use crate::{mihomo::MihomoClient, observability::connections::Connection};
use std::{
    collections::BTreeMap,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

fn request_target(url: &str) -> Result<reqwest::blocking::Response, String> {
    reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| "无法创建独立 HTTPS 探测客户端".to_string())?
        .get(url)
        .send()
        .map_err(|_| "普通 HTTPS 请求失败或超时；无法仅据此判断 TUN 故障".into())
}

pub(super) fn run(client: Option<&MihomoClient>) -> Vec<Check> {
    let baseline = client.and_then(|client| {
        client
            .connections_with_timeout(Duration::from_millis(500))
            .ok()
    });
    let start = Instant::now();
    let (tx, rx) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let _ = tx.send(request_target("https://github.com/"));
    });
    let mut observed = BTreeMap::new();
    let mut incomplete = baseline.as_ref().is_none_or(|snapshot| snapshot.truncated);
    let response = loop {
        if let Some(client) = client {
            match client.connections_with_timeout(Duration::from_millis(500)) {
                Ok(snapshot) => {
                    incomplete |= snapshot.truncated;
                    for connection in snapshot.connections {
                        let existed = baseline.as_ref().is_some_and(|snapshot| {
                            snapshot
                                .connections
                                .iter()
                                .any(|old| old.id == connection.id)
                        });
                        if !existed && connection.host.eq_ignore_ascii_case("github.com") {
                            observed.insert(connection.id.clone(), connection);
                        }
                    }
                }
                Err(_) => incomplete = true,
            }
        }
        match rx.try_recv() {
            Ok(response) => break Some(response),
            Err(mpsc::TryRecvError::Disconnected) => break None,
            Err(mpsc::TryRecvError::Empty) if start.elapsed() < Duration::from_secs(11) => {
                thread::sleep(Duration::from_millis(100))
            }
            _ => break None,
        }
    };
    let http = match response {
        Some(Ok(response)) => {
            let status = response.status();
            // Retain the response until the final observation; never consume its body.
            if let Some(client) = client {
                match client.connections_with_timeout(Duration::from_millis(500)) {
                    Ok(snapshot) => {
                        incomplete |= snapshot.truncated;
                        for connection in snapshot.connections {
                            if connection.host.eq_ignore_ascii_case("github.com")
                                && !baseline.as_ref().is_some_and(|snapshot| {
                                    snapshot
                                        .connections
                                        .iter()
                                        .any(|old| old.id == connection.id)
                                })
                            {
                                observed.insert(connection.id.clone(), connection);
                            }
                        }
                    }
                    Err(_) => incomplete = true,
                }
            }
            Check::new(
                "https_probe",
                if status.is_success() {
                    Status::Pass
                } else {
                    Status::Warning
                },
                format!(
                    "github.com 普通 HTTPS 返回 HTTP {}，约 {:.2}s；未显式设置代理，未跟随重定向；不证明 TUN 接管",
                    status.as_u16(),
                    start.elapsed().as_secs_f64()
                ),
            )
        }
        Some(Err(error)) => Check::new("https_probe", Status::Unknown, error),
        None => Check::new(
            "https_probe",
            Status::Unknown,
            "HTTPS 探测未在截止时间内完成",
        ),
    };
    let mut observation = observation_check(&observed.values().collect::<Vec<_>>());
    if incomplete {
        observation.status = Status::Unknown;
        observation
            .detail
            .push_str("；连接采样不完整，不能唯一归因探测");
    }
    vec![http, observation]
}

fn observation_check(connections: &[&Connection]) -> Check {
    match connections {
        [connection] => Check::new(
            "probe_observation",
            Status::Warning,
            format!(
                "观测到 github.com 新连接：{} → {} {} → {}；按域名关联，无法排除并发流量，探测接管未唯一确认",
                connection.inbound,
                connection.rule,
                connection.rule_payload,
                connection.route()
            ),
        ),
        [] => Check::new(
            "probe_observation",
            Status::Unknown,
            "未观测到目标新连接；可能是短连接或采样遗漏，不能据此判定未接管",
        ),
        _ => Check::new(
            "probe_observation",
            Status::Unknown,
            "观测到多个目标新连接，无法唯一归因；请在连接页查看入口、规则和链路",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn independent_probe_does_not_send_auth_or_follow_redirects() {
        use std::{
            io::{BufRead, BufReader, Write},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut headers = String::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                headers.push_str(&line);
            }
            assert!(!headers.to_lowercase().contains("authorization:"));
            write!(stream, "HTTP/1.1 302 Found\r\nLocation: /do-not-follow\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        assert_eq!(
            request_target(&format!("http://{address}/"))
                .unwrap()
                .status()
                .as_u16(),
            302
        );
        server.join().unwrap();
    }

    #[test]
    fn missing_or_ambiguous_observations_never_claim_failed_or_verified_takeover() {
        let empty = observation_check(&[]);
        assert_eq!(empty.status, super::super::Status::Unknown);
        let snapshot = crate::observability::ConnectionSnapshot::parse(&json!({"connections":[
            {"id":"a","metadata":{"host":"github.com","type":"Tun"},"chains":["DIRECT"]},
            {"id":"b","metadata":{"host":"github.com","type":"Tun"},"chains":["JP","Proxy"]}
        ]}))
        .unwrap();
        let check = observation_check(&snapshot.connections.iter().collect::<Vec<_>>());
        assert_eq!(check.status, super::super::Status::Unknown);
        let check = observation_check(&[&snapshot.connections[0]]);
        assert!(check.detail.contains("DIRECT"));
        assert_ne!(check.status, super::super::Status::Pass);
    }
}
