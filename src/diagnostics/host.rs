use std::{
    io::Read,
    path::Path,
    process::Stdio,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

const OUTPUT_LIMIT: u64 = 256 * 1024;

pub(super) fn read(program: &str, args: &[&str]) -> Result<String, String> {
    let path = ["/usr/bin", "/usr/sbin", "/bin", "/sbin"]
        .into_iter()
        .map(|directory| Path::new(directory).join(program))
        .find(|path| path.is_file())
        .ok_or_else(|| format!("缺少诊断命令 {program}"))?;
    let mut child = crate::system::clean_command(&path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| format!("无法执行只读命令 {program}"))?;
    let stdout = child.stdout.take().ok_or("无法读取诊断输出")?;
    let (tx, rx) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .take(OUTPUT_LIMIT + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = tx.send(result);
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return Err(format!("{program} 检查不可用，请检查权限与系统支持"));
                }
                let bytes = rx
                    .recv_timeout(Duration::from_millis(200))
                    .map_err(|_| "诊断输出未及时结束".to_string())?
                    .map_err(|_| "无法读取诊断输出".to_string())?;
                if bytes.len() as u64 > OUTPUT_LIMIT {
                    return Err("诊断输出超过 256 KiB 上限".into());
                }
                return String::from_utf8(bytes).map_err(|_| "诊断输出不是 UTF-8".into());
            }
            Ok(None) if start.elapsed() < Duration::from_secs(2) => {
                thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{program} 检查超时或无法等待进程"));
            }
        }
    }
}

pub(super) fn ip(args: &[&str]) -> Result<serde_json::Value, String> {
    serde_json::from_str(&read("ip", args)?).map_err(|_| "ip 命令未返回可识别的 JSON".into())
}
