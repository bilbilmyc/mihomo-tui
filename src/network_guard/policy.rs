pub fn network_only(baseline: &[u8], candidate: &[u8]) -> Result<(), String> {
    fn without_network(bytes: &[u8]) -> Result<serde_yaml::Mapping, String> {
        let value: serde_yaml::Value =
            serde_yaml::from_slice(bytes).map_err(|_| "无法解析保护配置（原文已隐藏）")?;
        let mut root = value
            .as_mapping()
            .cloned()
            .ok_or("保护配置必须是原生 YAML 映射")?;
        if root
            .get("kind")
            .and_then(serde_yaml::Value::as_str)
            .is_some_and(|kind| kind.starts_with("mihomo-tui/"))
        {
            return Err("保护应用仅支持原生 Mihomo 配置".into());
        }
        root.remove("tun");
        root.remove("dns");
        Ok(root)
    }
    if without_network(baseline)? != without_network(candidate)? {
        return Err(
            "首版保护应用只允许修改 TUN/DNS；其他字段与基线不同，请先明确应用并重新建立可工作基线"
                .into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn protected_changes_are_limited_to_tun_and_dns_without_mutating_provider_dependencies() {
        let baseline = b"proxy-providers: {airport: {type: file, path: provider.yaml}}\ntun: {enable: false}\nrules: ['MATCH,DIRECT']\n";
        let network = b"proxy-providers: {airport: {type: file, path: provider.yaml}}\ntun: {enable: true}\ndns: {enable: true}\nrules: ['MATCH,DIRECT']\n";
        assert!(super::network_only(baseline, network).is_ok());
        let changed = b"proxy-providers: {airport: {type: http, path: provider.yaml}}\ntun: {enable: true}\nrules: ['MATCH,DIRECT']\n";
        assert!(super::network_only(baseline, changed).is_err());
        assert!(
            super::network_only(b"rules: ['MATCH,DIRECT']", b"rules: ['MATCH,Proxy']").is_err()
        );
    }
}
