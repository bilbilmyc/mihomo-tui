mod host;
mod probe;
mod view;
pub use view::View;
#[cfg(test)]
mod tests;
use crate::mihomo::MihomoClient;
use serde::Serialize;
use serde_json::Value;
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub struct Options {
    pub path: PathBuf,
    pub client: Option<MihomoClient>,
    pub external: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub schema_version: u8,
    pub captured_at_unix_secs: u64,
    pub scope: &'static str,
    pub checks: Vec<Check>,
}

impl Report {
    pub fn has_failures(&self) -> bool {
        self.checks.iter().any(|check| check.status == Status::Fail)
    }

    pub fn render(&self) -> String {
        let mut output = format!(
            "Mihomo 只读诊断 | {} | Unix {}\n",
            self.scope, self.captured_at_unix_secs
        );
        for check in &self.checks {
            output.push_str(&format!(
                "[{}] {}：{}\n",
                check.status.label(),
                check.id,
                check.detail
            ));
        }
        output.push_str("本报告仅为一次采样；结构检查/API 可用不等于真实 TUN 接管。\n");
        output
    }
}

pub fn collect(options: &Options) -> Report {
    let mut report = Report {
        schema_version: 1,
        captured_at_unix_secs: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        scope: if options.external {
            "external_controller"
        } else {
            "local_host_ipv4"
        },
        checks: Vec::new(),
    };
    let configuration = if options.external {
        report.checks.push(Check::new(
            "local_host",
            Status::NotApplicable,
            "外部 Controller：不检查本机服务、网卡或路由，也不读取本机配置",
        ));
        None
    } else {
        match read_configuration(&options.path) {
            Ok(config) => {
                report.checks.push(Check::new(
                    "configuration",
                    Status::Pass,
                    "配置可只读解析；未创建或修改文件",
                ));
                Some(config)
            }
            Err(error) => {
                report
                    .checks
                    .push(Check::new("configuration", Status::Unknown, error));
                None
            }
        }
    };
    let runtime = if let Some(client) = &options.client {
        match client.version() {
            Ok(version) => report.checks.push(Check::new(
                "controller",
                Status::Pass,
                format!("Controller 返回核心版本 {version}"),
            )),
            Err(error) => report.checks.push(Check::new(
                "controller",
                Status::Unknown,
                client.redact(&error),
            )),
        }
        match client.runtime_config() {
            Ok(config) => {
                report.checks.push(Check::new(
                    "runtime_config",
                    Status::Pass,
                    "已取得运行时基本配置；未修改核心",
                ));
                Some(config)
            }
            Err(error) => {
                report
                    .checks
                    .push(Check::new("runtime_config", Status::Unknown, error));
                None
            }
        }
    } else {
        report.checks.push(Check::new(
            "controller",
            Status::Unknown,
            "未配置可用的 Controller；请检查监听地址和 MIHOMO_SECRET",
        ));
        None
    };
    if !options.external {
        collect_host(&mut report, configuration.as_ref(), runtime.as_ref());
    }
    report.checks.push(Check::new(
        "coverage",
        Status::Warning,
        "未验证 IPv6、DNS 解析路径或持续运行健康；真实请求需显式探测",
    ));
    report
}

pub fn collect_with_probe(options: &Options, probe: bool) -> Report {
    let mut report = collect(options);
    if probe {
        if options.external {
            report.checks.push(Check::new(
                "https_probe",
                Status::NotApplicable,
                "外部 Controller 模式不在本机发请求冒充远端探测；请在对应主机运行 doctor --probe",
            ));
        } else {
            report
                .checks
                .extend(self::probe::run(options.client.as_ref()));
        }
    } else {
        report.checks.push(Check::new(
            "https_probe",
            Status::NotApplicable,
            "未向外部目标发起请求；使用 --probe 显式探测 github.com",
        ));
    }
    report
}

fn collect_host(report: &mut Report, configuration: Option<&Value>, runtime: Option<&Value>) {
    let service = match host::read(
        "systemctl",
        &[
            "show",
            "mihomo.service",
            "--property=ActiveState",
            "--value",
        ],
    ) {
        Ok(state) if state.trim() == "active" => Check::new(
            "service",
            Status::Pass,
            "mihomo.service 为 active（不代表实际接管）",
        ),
        Ok(state) if matches!(state.trim(), "inactive" | "failed") => Check::new(
            "service",
            Status::Fail,
            "mihomo.service 未运行；诊断不会自动启动它",
        ),
        Ok(_) => Check::new("service", Status::Unknown, "服务状态暂不可判定"),
        Err(error) => Check::new("service", Status::Unknown, error),
    };
    report.checks.push(service);
    let tun = runtime
        .and_then(|config| config.get("tun"))
        .or_else(|| configuration.and_then(|config| config.get("tun")));
    let Some(tun) = tun else {
        report.checks.push(Check::new(
            "tun",
            Status::Unknown,
            "无法确定 TUN 配置，跳过网卡和路由判断",
        ));
        return;
    };
    let Some(enabled) = tun.get("enable").and_then(Value::as_bool) else {
        report.checks.push(Check::new(
            "tun",
            Status::Unknown,
            "缺少可识别的运行时 TUN 开关",
        ));
        return;
    };
    if !enabled {
        report.checks.push(Check::new(
            "tun",
            Status::NotApplicable,
            "TUN 未配置开启，不要求存在 TUN 路由",
        ));
        return;
    }
    report.checks.push(Check::new(
        "tun_device_node",
        if Path::new("/dev/net/tun").exists() {
            Status::Pass
        } else {
            Status::Unknown
        },
        "检查 /dev/net/tun 是否存在；存在不代表权限/虚拟化允许创建网卡",
    ));
    if tun.get("auto-route").and_then(Value::as_bool) != Some(true) {
        report.checks.push(Check::new(
            "tun_routes",
            Status::NotApplicable,
            "未启用 auto-route，不判断用户手动路由的完整性",
        ));
        return;
    }
    let configured_tun = configuration.and_then(|config| config.get("tun"));
    let table_value = tun
        .get("iproute2-table-index")
        .or_else(|| configured_tun.and_then(|tun| tun.get("iproute2-table-index")));
    if configured_tun.is_none() && table_value.is_none() {
        report.checks.push(Check::new(
            "tun_routes",
            Status::Unknown,
            "本地配置不可读，API 未提供表编号；不假定使用默认表 2022",
        ));
        return;
    }
    let table = match table_value {
        None => 2022,
        Some(value) => match value
            .as_u64()
            .and_then(|table| u32::try_from(table).ok())
            .filter(|table| *table > 0)
        {
            Some(table) => table,
            None => {
                report.checks.push(Check::new(
                    "tun_routes",
                    Status::Unknown,
                    "无法确定自定义 TUN 路由表编号",
                ));
                return;
            }
        },
    };
    let routes = match host::ip(&["-j", "-4", "route", "show", "table", &table.to_string()]) {
        Ok(routes) => routes,
        Err(error) => {
            report
                .checks
                .push(Check::new("tun_routes", Status::Unknown, error));
            return;
        }
    };
    let device = tun
        .get("device")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .or_else(|| {
            configured_tun
                .and_then(|tun| tun.get("device"))
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
        })
        .or_else(|| {
            routes
                .as_array()?
                .iter()
                .find(|route| route.get("dst").and_then(Value::as_str) == Some("default"))?
                .get("dev")?
                .as_str()
        });
    let Some(device) = device else {
        report.checks.push(Check::new(
            "tun_interface",
            Status::Unknown,
            "无法确定 TUN 设备名；未假定为 Meta",
        ));
        return;
    };
    let link = match host::ip(&["-j", "link", "show"]) {
        Ok(links) => match links.as_array().and_then(|links| {
            links
                .iter()
                .find(|link| link.get("ifname").and_then(Value::as_str) == Some(device))
        }) {
            Some(link)
                if link
                    .get("flags")
                    .and_then(Value::as_array)
                    .is_some_and(|flags| flags.iter().any(|flag| flag == "UP")) =>
            {
                Check::new(
                    "tun_interface",
                    Status::Pass,
                    format!("{} 网卡存在且为 UP", crate::sanitize::text(device, None)),
                )
            }
            Some(_) => Check::new("tun_interface", Status::Fail, "TUN 网卡没有 UP 标志"),
            None => Check::new("tun_interface", Status::Unknown, "未找到可确认的 TUN 网卡"),
        },
        Err(error) => Check::new("tun_interface", Status::Unknown, error),
    };
    report.checks.push(link);
    match host::ip(&["-j", "-4", "rule", "show"]) {
        Ok(rules) => report
            .checks
            .extend(evaluate_routes(device, table, &rules, &routes)),
        Err(error) => report
            .checks
            .push(Check::new("policy_rules", Status::Unknown, error)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pass,
    Warning,
    Fail,
    Unknown,
    NotApplicable,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pass => "通过",
            Self::Warning => "警告",
            Self::Fail => "失败",
            Self::Unknown => "未知",
            Self::NotApplicable => "不适用",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub id: &'static str,
    pub status: Status,
    pub detail: String,
}

impl Check {
    fn new(id: &'static str, status: Status, detail: impl Into<String>) -> Self {
        Self {
            id,
            status,
            detail: detail.into(),
        }
    }
}

fn read_configuration(path: &Path) -> Result<Value, String> {
    let mut content = String::new();
    File::open(path)
        .and_then(|file| file.take(2 * 1024 * 1024 + 1).read_to_string(&mut content))
        .map_err(|_| "无法只读打开配置，请检查路径与权限".to_string())?;
    if content.len() > 2 * 1024 * 1024 {
        return Err("配置超过诊断读取上限 2 MiB".into());
    }
    let document: serde_yaml::Value = serde_yaml::from_str(&content)
        .map_err(|_| "配置 YAML 无法解析（原文已隐藏）".to_string())?;
    let profile = crate::workspace::config_profile(&document)?;
    if !profile.is_mapping() {
        return Err("配置根节点不是映射".into());
    }
    serde_json::to_value(profile).map_err(|_| "配置字段无法转换为诊断数据".into())
}

fn table_matches(value: &Value, table: u32) -> bool {
    value.as_u64() == Some(u64::from(table)) || value.as_str() == Some(&table.to_string())
}

fn evaluate_routes(device: &str, table: u32, rules: &Value, routes: &Value) -> Vec<Check> {
    let policy = match rules.as_array() {
        None => Check::new("policy_rules", Status::Unknown, "无法解析 IPv4 策略规则"),
        Some(rules) => {
            let unresolved = rules.iter().find_map(|rule| {
                let target = rule.get("goto")?.as_u64()?;
                (rule.get("iif").and_then(Value::as_str) == Some(device)
                    && !rules
                        .iter()
                        .any(|item| item.get("priority").and_then(Value::as_u64) == Some(target)))
                .then_some(target)
            });
            if let Some(target) = unresolved {
                Check::new(
                    "policy_rules",
                    Status::Fail,
                    format!(
                        "TUN goto {target} 无目标；请检查网络管理器冲突，确认后可按 p 重建规则"
                    ),
                )
            } else if rules.iter().any(|rule| {
                rule.get("table")
                    .is_some_and(|value| table_matches(value, table))
                    && rule
                        .get("dst")
                        .is_none_or(|value| matches!(value.as_str(), Some("all" | "default")))
            }) {
                Check::new(
                    "policy_rules",
                    Status::Pass,
                    format!("存在进入表 {table} 的 IPv4 策略入口；仅结构检查，不证明真实接管"),
                )
            } else {
                Check::new(
                    "policy_rules",
                    Status::Fail,
                    format!("缺少进入表 {table} 的通用 IPv4 策略入口；路由表存在也不足以接管"),
                )
            }
        }
    };
    let route = match routes.as_array() {
        None => Check::new("route_table", Status::Unknown, "无法解析 TUN IPv4 路由表"),
        Some(routes)
            if routes.iter().any(|route| {
                route.get("dev").and_then(Value::as_str) == Some(device)
                    && route.get("dst").and_then(Value::as_str) == Some("default")
                    && route.get("type").is_none_or(|kind| kind == "unicast")
            }) =>
        {
            Check::new(
                "route_table",
                Status::Pass,
                format!(
                    "表 {table} 的默认路由指向 {}；仅结构检查",
                    crate::sanitize::text(device, None)
                ),
            )
        }
        Some(_) => Check::new(
            "route_table",
            Status::Fail,
            format!("表 {table} 没有指向 TUN 的默认 IPv4 路由；确认配置后可按 p 重建"),
        ),
    };
    vec![policy, route]
}
