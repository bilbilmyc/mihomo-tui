use crate::{
    core::CoreVersion,
    models::{ProxyDelay, ProxySummary},
};
use reqwest::blocking::Client;
use serde_json::Value;
use std::{collections::BTreeMap, time::Duration};

const DELAY_TEST_URL: &str = "https://www.gstatic.com/generate_204";
const DELAY_TIMEOUT_MS: u64 = 5_000;

#[derive(Debug, Clone)]
pub struct MihomoClient {
    base_url: String,
    secret: Option<String>,
    http: Client,
}

impl MihomoClient {
    pub fn new(
        base_url: impl Into<String>,
        secret: Option<String>,
    ) -> Result<Self, reqwest::Error> {
        Ok(Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            secret,
            http: Client::builder().timeout(Duration::from_secs(5)).build()?,
        })
    }

    fn request(
        &self,
        request: reqwest::blocking::RequestBuilder,
    ) -> reqwest::blocking::RequestBuilder {
        match &self.secret {
            Some(secret) if !secret.is_empty() => {
                request.header("Authorization", format!("Bearer {secret}"))
            }
            _ => request,
        }
    }

    pub fn proxies(&self) -> Result<Vec<ProxySummary>, String> {
        let response = self
            .request(self.http.get(format!("{}/proxies", self.base_url)))
            .send()
            .map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("Mihomo returned {}", response.status()));
        }
        let body: Value = response.json().map_err(|e| e.to_string())?;
        parse_proxies(&body)
    }

    pub fn version(&self) -> Result<CoreVersion, String> {
        let response = self
            .request(self.http.get(format!("{}/version", self.base_url)))
            .send()
            .map_err(|error| error.to_string())?;
        if !response.status().is_success() {
            return Err(format!("Mihomo returned {}", response.status()));
        }
        let body: Value = response.json().map_err(|error| error.to_string())?;
        let version = body
            .get("version")
            .and_then(Value::as_str)
            .ok_or_else(|| "Mihomo version response has no version string".to_string())?;
        CoreVersion::parse(version)
    }

    pub fn select_proxy(&self, group: &str, proxy: &str) -> Result<(), String> {
        let url = format!("{}/proxies/{}", self.base_url, urlencoding::encode(group));
        let response = self
            .request(
                self.http
                    .put(url)
                    .json(&serde_json::json!({ "name": proxy })),
            )
            .send()
            .map_err(|e| e.to_string())?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(format!("Mihomo returned {}", response.status()))
        }
    }

    pub fn refresh_provider(&self, name: &str) -> Result<(), String> {
        let response = self
            .request(self.http.put(self.provider_url(name)))
            .send()
            .map_err(|error| error.to_string())?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(format!("Mihomo returned {}", response.status()))
        }
    }

    pub fn provider_proxy_count(&self, name: &str) -> Result<usize, String> {
        let response = self
            .request(self.http.get(self.provider_url(name)))
            .send()
            .map_err(|error| error.to_string())?;
        if !response.status().is_success() {
            return Err(format!("Mihomo returned {}", response.status()));
        }
        let body: Value = response.json().map_err(|error| error.to_string())?;
        body.get("proxies")
            .and_then(Value::as_array)
            .map(Vec::len)
            .ok_or_else(|| "provider response has no proxies list".into())
    }

    pub fn probe_delay(&self, name: &str) -> Result<ProxyDelay, String> {
        let response = self
            .request(
                self.http
                    .get(self.proxy_delay_url(name))
                    .timeout(Duration::from_millis(DELAY_TIMEOUT_MS + 1_000)),
            )
            .send()
            .map_err(|error| error.to_string())?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return self.probe_provider_delay(name);
        }
        parse_delay_response(response)
    }

    fn probe_provider_delay(&self, name: &str) -> Result<ProxyDelay, String> {
        let Some(provider) = self.provider_for_proxy(name)? else {
            return Ok(ProxyDelay::Failed);
        };
        let response = self
            .request(
                self.http
                    .get(self.provider_proxy_delay_url(&provider, name))
                    .timeout(Duration::from_millis(DELAY_TIMEOUT_MS + 1_000)),
            )
            .send()
            .map_err(|error| error.to_string())?;
        parse_delay_response(response)
    }

    fn provider_for_proxy(&self, name: &str) -> Result<Option<String>, String> {
        let response = self
            .request(
                self.http
                    .get(format!("{}/providers/proxies", self.base_url)),
            )
            .send()
            .map_err(|error| error.to_string())?;
        if !response.status().is_success() {
            return Err(format!("Mihomo returned {}", response.status()));
        }
        let body: Value = response.json().map_err(|error| error.to_string())?;
        let providers = body
            .get("providers")
            .and_then(Value::as_object)
            .ok_or_else(|| "response has no providers object".to_string())?;
        Ok(providers.iter().find_map(|(provider_name, provider)| {
            provider
                .get("proxies")
                .and_then(Value::as_array)
                .is_some_and(|proxies| {
                    proxies
                        .iter()
                        .any(|proxy| proxy.get("name").and_then(Value::as_str) == Some(name))
                })
                .then(|| provider_name.clone())
        }))
    }

    fn provider_proxy_delay_url(&self, provider: &str, name: &str) -> String {
        format!(
            "{}/providers/proxies/{}/{}/healthcheck?url={}&timeout={DELAY_TIMEOUT_MS}",
            self.base_url,
            urlencoding::encode(provider),
            urlencoding::encode(name),
            urlencoding::encode(DELAY_TEST_URL),
        )
    }

    fn provider_url(&self, name: &str) -> String {
        format!(
            "{}/providers/proxies/{}",
            self.base_url,
            urlencoding::encode(name)
        )
    }

    fn proxy_delay_url(&self, name: &str) -> String {
        format!(
            "{}/proxies/{}/delay?url={}&timeout={DELAY_TIMEOUT_MS}&expected=200%2C204",
            self.base_url,
            urlencoding::encode(name),
            urlencoding::encode(DELAY_TEST_URL),
        )
    }
}

fn parse_proxies(body: &Value) -> Result<Vec<ProxySummary>, String> {
    let Some(proxies) = body.get("proxies").and_then(Value::as_object) else {
        return Err("response has no proxies object".into());
    };
    let delays: BTreeMap<_, _> = proxies
        .iter()
        .filter_map(|(name, proxy)| history_delay(proxy).map(|delay| (name.clone(), delay)))
        .collect();
    let mut result = Vec::new();
    for (name, proxy) in proxies {
        let proxy_type = proxy
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let now = proxy.get("now").and_then(Value::as_str).map(str::to_string);
        let delay_ms = history_delay(proxy).and_then(|delay| match delay {
            ProxyDelay::Measured(delay) => Some(delay),
            ProxyDelay::Timeout | ProxyDelay::Failed => None,
        });
        let members: Vec<String> = proxy
            .get("all")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        if members.is_empty() || !is_proxy_group_type(&proxy_type) {
            continue;
        }
        let member_delays = members
            .iter()
            .filter_map(|member| {
                delays
                    .get(member)
                    .cloned()
                    .map(|delay| (member.clone(), delay))
            })
            .collect();
        result.push(ProxySummary {
            name: name.clone(),
            proxy_type,
            now,
            delay_ms,
            members,
            member_delays,
        });
    }
    result.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(result)
}

fn is_proxy_group_type(proxy_type: &str) -> bool {
    matches!(
        proxy_type,
        "Selector" | "URLTest" | "Fallback" | "LoadBalance" | "Relay"
    )
}

fn parse_delay_response(response: reqwest::blocking::Response) -> Result<ProxyDelay, String> {
    if response.status() == reqwest::StatusCode::REQUEST_TIMEOUT {
        return Ok(ProxyDelay::Timeout);
    }
    if !response.status().is_success() {
        return Ok(ProxyDelay::Failed);
    }
    let body: Value = response.json().map_err(|error| error.to_string())?;
    body.get("delay")
        .and_then(Value::as_u64)
        .map(ProxyDelay::Measured)
        .ok_or_else(|| "Mihomo delay response has no delay value".into())
}

fn history_delay(proxy: &Value) -> Option<ProxyDelay> {
    let delay = proxy
        .get("history")
        .and_then(Value::as_array)
        .and_then(|history| history.last())
        .and_then(|item| item.get("delay"))
        .and_then(Value::as_u64)?;
    Some(if delay == 0 {
        ProxyDelay::Timeout
    } else {
        ProxyDelay::Measured(delay)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        thread,
        time::{Duration, Instant},
    };

    #[test]
    fn provider_refresh_url_escapes_provider_names() {
        let client = MihomoClient::new("http://127.0.0.1:9090", None).unwrap();

        assert_eq!(
            client.provider_url("my provider"),
            "http://127.0.0.1:9090/providers/proxies/my%20provider"
        );
    }

    #[test]
    fn provider_proxy_count_reads_the_selected_provider() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request_line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request_line)
                .unwrap();
            assert!(request_line.starts_with("GET /providers/proxies/airport "));
            let body = r#"{"name":"airport","proxies":[{"name":"Node A"},{"name":"Node B"}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        let client = MihomoClient::new(format!("http://{address}"), None).unwrap();

        let count = client.provider_proxy_count("airport").unwrap();

        server.join().unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn version_reads_the_exact_controller_version() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request_line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request_line)
                .unwrap();
            assert!(request_line.starts_with("GET /version "));
            let body = r#"{"meta":true,"version":"v1.19.29"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        let client = MihomoClient::new(format!("http://{address}"), None).unwrap();

        let version = client.version().unwrap();

        server.join().unwrap();
        assert_eq!(version.to_string(), "v1.19.29");
    }

    #[test]
    fn proxy_delay_url_escapes_a_node_name_and_sets_a_timeout() {
        let client = MihomoClient::new("http://127.0.0.1:9090", None).unwrap();

        assert_eq!(
            client.proxy_delay_url("Tokyo node"),
            "http://127.0.0.1:9090/proxies/Tokyo%20node/delay?url=https%3A%2F%2Fwww.gstatic.com%2Fgenerate_204&timeout=5000&expected=200%2C204"
        );
    }

    #[test]
    fn delay_probe_falls_back_to_a_provider_node_healthcheck() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_millis(500);
            let mut request_count = 0;
            while request_count < 3 && Instant::now() < deadline {
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("mock server failed: {error}"),
                };
                let mut request_line = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut request_line)
                    .unwrap();
                let target = request_line.split_whitespace().nth(1).unwrap_or_default();
                let (status, body) = if target.starts_with("/proxies/Tokyo%20node/delay?") {
                    ("404 Not Found", r#"{"message":"Resource not found"}"#)
                } else if target == "/providers/proxies" {
                    (
                        "200 OK",
                        r#"{"providers":{"airport":{"proxies":[{"name":"Tokyo node"}]}}}"#,
                    )
                } else if target.starts_with("/providers/proxies/airport/Tokyo%20node/healthcheck?")
                {
                    ("200 OK", r#"{"delay":128}"#)
                } else {
                    (
                        "500 Internal Server Error",
                        r#"{"message":"unexpected request"}"#,
                    )
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
                request_count += 1;
            }
            request_count
        });
        let client = MihomoClient::new(format!("http://{address}"), None).unwrap();

        let delay = client.probe_delay("Tokyo node").unwrap();
        let request_count = server.join().unwrap();

        assert_eq!(delay, ProxyDelay::Measured(128));
        assert_eq!(request_count, 3);
    }

    #[test]
    fn proxy_response_only_exposes_selectable_groups() {
        let body: Value = serde_json::from_str(
            r#"{
                "proxies": {
                    "DIRECT": {"type":"Direct","history":[]},
                    "REJECT": {"type":"Reject","history":[]},
                    "GLOBAL": {"type":"Selector","all":["Main"],"now":"Main","history":[]},
                    "Main": {"type":"Selector","all":["Node A"],"now":"Node A","history":[]},
                    "Auto": {"type":"URLTest","all":["Node A"],"now":"Node A","history":[]},
                    "Node A": {"type":"Shadowsocks","history":[{"delay":88}]}
                }
            }"#,
        )
        .unwrap();

        let groups = parse_proxies(&body).unwrap();

        assert_eq!(
            groups
                .iter()
                .map(|group| group.name.as_str())
                .collect::<Vec<_>>(),
            ["Auto", "GLOBAL", "Main"]
        );
        assert_eq!(groups[2].member_delays["Node A"], ProxyDelay::Measured(88));
    }
}
