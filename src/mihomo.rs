use crate::models::ProxySummary;
use reqwest::blocking::Client;
use serde_json::Value;
use std::time::Duration;

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
        let Some(proxies) = body.get("proxies").and_then(Value::as_object) else {
            return Err("response has no proxies object".into());
        };
        let mut result = Vec::new();
        for (name, proxy) in proxies {
            let proxy_type = proxy
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            let now = proxy.get("now").and_then(Value::as_str).map(str::to_string);
            let delay_ms = proxy
                .get("history")
                .and_then(Value::as_array)
                .and_then(|h| h.last())
                .and_then(|item| item.get("delay"))
                .and_then(Value::as_u64);
            let members = proxy
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
            result.push(ProxySummary {
                name: name.clone(),
                proxy_type,
                now,
                delay_ms,
                members,
            });
        }
        result.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(result)
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
}
