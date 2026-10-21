use super::{
    clock,
    engine::{Backend, BindingError, Service},
    model::{PROTOCOL, Stamp, valid_invocation},
    store::{self, Store},
    units,
};
use crate::{
    core::{CoreRelease, CoreVersion},
    core_manager::CorePaths,
    runtime, system,
};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Seek},
    path::{Path, PathBuf},
    process::Stdio,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

const BINARY_LIMIT: usize = 256 * 1024 * 1024;

fn properties(value: &str) -> BTreeMap<&str, &str> {
    value
        .lines()
        .filter_map(|line| line.split_once('='))
        .collect()
}

fn parse_service(value: &str) -> Result<Service, String> {
    let values = properties(value);
    let state = values.get("ActiveState").ok_or("缺少服务状态")?;
    let pid = values
        .get("MainPID")
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or("无法取得核心 PID")?;
    let invocation = values
        .get("InvocationID")
        .filter(|value| valid_invocation(value))
        .map(|value| (*value).to_string());
    Ok(Service {
        active: *state == "active" && pid > 0,
        starting: *state == "activating" && pid == 0,
        enabled: values.get("UnitFileState") == Some(&"enabled"),
        job: values
            .get("Job")
            .is_some_and(|value| !value.is_empty() && *value != "0"),
        invocation,
    })
}

pub fn run(program: &Path, args: &[&str], seconds: u64) -> Result<(bool, String), String> {
    if !system::trusted_root_file(program) {
        return Err("外部命令不是受信任的 root 文件".into());
    }
    let mut child = system::clean_command(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "无法运行保护操作所需命令")?;
    let stdout = child.stdout.take().ok_or("无法读取保护命令输出")?;
    let (tx, rx) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .take(128 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = tx.send(result);
    });
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let bytes = rx
                    .recv_timeout(Duration::from_millis(200))
                    .map_err(|_| "保护命令输出未及时结束")?
                    .map_err(|_| "无法读取保护命令输出")?;
                if bytes.len() > 128 * 1024 {
                    return Err("保护命令输出超过上限".into());
                }
                return Ok((
                    status.success(),
                    String::from_utf8(bytes).map_err(|_| "保护命令输出不是 UTF-8")?,
                ));
            }
            Ok(None) if started.elapsed() < Duration::from_secs(seconds) => {
                thread::sleep(Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("保护命令超时或无法等待；未宣告操作成功".into());
            }
        }
    }
}

fn ctl_raw(args: &[&str]) -> Result<(bool, String), String> {
    let program = ["/usr/bin/systemctl", "/bin/systemctl"]
        .into_iter()
        .map(PathBuf::from)
        .find(|path| system::trusted_root_file(path))
        .ok_or("找不到受信任的 systemctl")?;
    run(&program, args, 2)
}

pub fn ctl(args: &[&str]) -> Result<String, String> {
    let (success, output) = ctl_raw(args)?;
    if !success {
        return Err("systemd 保护操作失败；请检查服务/权限，原始输出未公开".into());
    }
    Ok(output)
}

fn public_file(path: &Path, limit: usize) -> Result<File, String> {
    #[cfg(target_os = "linux")]
    {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(0o400000 | 0o4000)
            .open(path)
            .map_err(|_| "无法安全打开核心/服务文件")?;
        public_metadata(&file, limit)?;
        Ok(file)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (path, limit);
        Err("保护操作仅支持 Linux".into())
    }
}

fn public_metadata(file: &File, limit: usize) -> Result<(), String> {
    let meta = file.metadata().map_err(|_| "无法验证核心文件描述符")?;
    #[cfg(unix)]
    if !meta.is_file()
        || !system::owner_is_trusted(meta.uid())
        || meta.mode() & 0o022 != 0
        || meta.len() > limit as u64
    {
        return Err("核心/服务文件类型、所有者、权限或大小不安全".into());
    }
    Ok(())
}

fn hash_handle(mut file: File) -> Result<String, String> {
    public_metadata(&file, BINARY_LIMIT)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 16 * 1024];
    let mut total = 0usize;
    loop {
        let count = file.read(&mut buffer).map_err(|_| "无法校验可执行文件")?;
        if count == 0 {
            break;
        }
        total += count;
        if total > BINARY_LIMIT {
            return Err("可执行文件超过上限".into());
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub fn self_hash() -> Result<String, String> {
    // This is the running image, not a pathname that an upgrade may have replaced.
    hash_handle(File::open("/proc/self/exe").map_err(|_| "无法打开当前运行映像")?)
}

pub fn worker_hash() -> Result<String, String> {
    hash_handle(store::open_private(
        Path::new(units::WORKER),
        BINARY_LIMIT,
        false,
    )?)
}

pub fn read_public(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    public_file(path, limit)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取服务文件")?;
    if bytes.len() > limit {
        return Err("服务文件超过上限".into());
    }
    Ok(bytes)
}

pub fn validate_hook() -> Result<(), String> {
    if fs::symlink_metadata(units::HOOK_PATH).is_err() {
        return Ok(());
    }
    if read_public(Path::new(units::HOOK_PATH), 64 * 1024)? != units::hook_content().as_bytes() {
        return Err("存在未识别的保护启动 drop-in，拒绝覆盖或重启".into());
    }
    let store = Store::existing(Path::new(units::ROOT))?.ok_or("保护启动 hook 缺少状态目录")?;
    let ledger = store.load()?;
    let hash = worker_hash()?;
    if ledger.worker_sha256.as_deref() != Some(&hash) && (ledger.pending() || self_hash()? != hash)
    {
        return Err("恢复程序身份与账本不匹配，请使用可信构建修复保护设施".into());
    }
    Ok(())
}

fn core_paths() -> Result<(PathBuf, PathBuf), String> {
    let drop_in = read_public(Path::new(runtime::MANAGED_DROP_IN), 64 * 1024)?;
    let target = std::str::from_utf8(&drop_in)
        .ok()
        .and_then(runtime::managed_dropin_binary)
        .ok_or("核心 drop-in 不是受支持的托管内容")?;
    let selected = if target == CorePaths::system().active_binary() {
        let root = Path::new(crate::core_manager::MANAGED_ROOT);
        if !system::trusted_root_directory(root)
            || !system::trusted_root_directory(&root.join("cores"))
        {
            return Err("托管核心目录不安全".into());
        }
        let link = fs::read_link(root.join("current")).map_err(|_| "无法读取当前核心链接")?;
        let version = link
            .strip_prefix("cores")
            .ok()
            .and_then(|value| value.to_str())
            .and_then(|value| CoreVersion::parse(value).ok())
            .ok_or("当前核心链接目标无效")?;
        if link != Path::new("cores").join(version.to_string()) {
            return Err("当前核心链接不是规范相对目标".into());
        }
        CorePaths::system().binary(version)
    } else {
        target.clone()
    };
    if !system::trusted_root_directory(selected.parent().ok_or("核心缺少目录")?) {
        return Err("核心目录不安全".into());
    }
    Ok((target, selected))
}

fn execution_signature(output: &str, target: &Path) -> Result<Vec<u8>, String> {
    let info = properties(output);
    let expected = format!("{} -d /etc/mihomo-tui", target.display());
    let argv = info
        .get("ExecStart")
        .and_then(|value| value.split("argv[]=").nth(1))
        .and_then(|value| value.split(';').next())
        .map(str::trim);
    if argv != Some(expected.as_str())
        || !matches!(info.get("Type"), Some(&"simple" | &"exec"))
        || !matches!(info.get("User"), Some(&"" | &"root"))
        || info
            .get("RootDirectory")
            .is_none_or(|value| !value.is_empty())
        || info.get("RootImage").is_none_or(|value| !value.is_empty())
    {
        return Err("核心服务执行/命名空间配置不符合首版保护边界".into());
    }
    let mut signature = expected.into_bytes();
    signature.push(0);
    // ExecStart also contains invocation timestamps/PIDs; only static properties bind a checkpoint.
    for key in ["FragmentPath", "Type", "User", "RootDirectory", "RootImage"] {
        signature.extend_from_slice(info.get(key).copied().ok_or("缺少静态服务属性")?.as_bytes());
        signature.push(0);
    }
    Ok(signature)
}

fn core_info() -> Result<(PathBuf, String), BindingError> {
    // No inventory/version subprocesses are allowed in the recovery/startup call graph.
    let (target, selected) = core_paths().map_err(BindingError::Conflict)?;
    let output = ctl(&[
        "show",
        "mihomo.service",
        "--property=LoadState",
        "--property=DropInPaths",
        "--property=ExecStart",
        "--property=FragmentPath",
        "--property=Type",
        "--property=User",
        "--property=RootDirectory",
        "--property=RootImage",
    ])?;
    runtime::validate_guard_unit_output(&output).map_err(BindingError::Conflict)?;
    let signature = execution_signature(&output, &target).map_err(BindingError::Conflict)?;
    let selected_hash =
        hash_handle(public_file(&selected, BINARY_LIMIT).map_err(BindingError::Conflict)?)?;
    let drop_in = read_public(Path::new(runtime::MANAGED_DROP_IN), 64 * 1024)?;
    let info = properties(&output);
    let fragment = info.get("FragmentPath").ok_or("无法确定服务 unit")?;
    let mut hash = Sha256::new();
    for bytes in [
        selected.to_string_lossy().as_bytes(),
        selected_hash.as_bytes(),
        drop_in.as_slice(),
        signature.as_slice(),
    ] {
        hash.update(bytes);
        hash.update([0]);
    }
    hash.update(read_public(Path::new(fragment), 256 * 1024)?);
    if Path::new(units::HOOK_PATH).exists() {
        hash.update(read_public(Path::new(units::HOOK_PATH), 64 * 1024)?);
        hash.update(worker_hash()?);
    }
    Ok((selected, format!("{:x}", hash.finalize())))
}

pub struct Host {
    pub store: Store,
    image_hash: String,
}

impl Host {
    pub fn new(store: Store) -> Result<Self, String> {
        Ok(Self {
            store,
            image_hash: self_hash()?,
        })
    }

    pub fn compatible(&self) -> Result<(), String> {
        let ledger = self.store.load()?;
        if ledger.worker_sha256.as_deref() != Some(&self.image_hash)
            || worker_hash()? != self.image_hash
        {
            return Err("保护事务使用不同构建的恢复程序；请使用私有 worker 操作，或在事务结束后重新 checkpoint".into());
        }
        let (success, output) = run(Path::new(units::WORKER), &["network", "_protocol"], 2)?;
        if !success || output.trim() != PROTOCOL.to_string() {
            return Err("恢复程序协议不匹配，未启用试运行".into());
        }
        Ok(())
    }

    fn known_unit(name: &str, path: &str, expected: &str) -> Result<bool, String> {
        if fs::symlink_metadata(path).is_ok()
            && read_public(Path::new(path), 64 * 1024)? != expected.as_bytes()
        {
            return Err("恢复 unit 路径被未知文件占用，未覆盖".into());
        }
        let (_, output) = ctl_raw(&[
            "show",
            name,
            "--property=LoadState",
            "--property=FragmentPath",
            "--property=DropInPaths",
        ])?;
        let info = properties(&output);
        if info.get("LoadState") == Some(&"not-found")
            && info.get("FragmentPath") == Some(&"")
            && info.get("DropInPaths") == Some(&"")
        {
            return Ok(false);
        }
        if info.get("LoadState") != Some(&"loaded")
            || info.get("FragmentPath") != Some(&path)
            || info.get("DropInPaths") != Some(&"")
        {
            return Err("systemd 已加载未知恢复单元，未操作该服务".into());
        }
        Ok(true)
    }

    pub fn bootstrap(&mut self) -> Result<(), String> {
        let mut ledger = self.store.load()?;
        if ledger.pending() {
            return Err("不能在未解决事务期间替换恢复程序".into());
        }
        let (binary, _) = core_info()?;
        let (ok, version) = run(&binary, &["-v"], 5)?;
        if !ok {
            return Err("无法检查现有核心版本".into());
        }
        CoreRelease::embedded()?.require_supported(
            CoreVersion::from_mihomo_output(&version).map_err(|_| "无法解析核心版本")?,
        )?;
        let service = self.service()?;
        if !service.active || !service.enabled {
            return Err("建立基线要求核心已运行且持久启用".into());
        }
        let service_known = Self::known_unit(
            units::SERVICE,
            units::SERVICE_PATH,
            &units::service_content(units::WORKER),
        )?;
        let timer_known =
            Self::known_unit(units::TIMER, units::TIMER_PATH, &units::timer_content())?;
        if fs::symlink_metadata(units::WORKER).is_ok() {
            let old = worker_hash()?;
            if old != self.image_hash && ledger.worker_sha256.as_deref() != Some(&old) {
                return Err("私有恢复路径含未知程序，未替换".into());
            }
        }
        if timer_known {
            ctl(&["disable", "--now", units::TIMER])?;
        }
        if service_known {
            ctl(&["stop", units::SERVICE])?;
        }
        if fs::symlink_metadata(units::WORKER).is_err() || worker_hash()? != self.image_hash {
            let temporary = self
                .store
                .root
                .join(format!(".worker-{}.tmp", store::random_id()?));
            let mut source = File::open("/proc/self/exe").map_err(|_| "无法打开当前程序映像")?;
            public_metadata(&source, BINARY_LIMIT)?;
            let mut bytes = [0u8; 4];
            source
                .read_exact(&mut bytes)
                .map_err(|_| "无法校验当前程序格式")?;
            if bytes != *b"\x7fELF" {
                return Err("保护工作程序必须为原生 ELF".into());
            }
            source.rewind().map_err(|_| "无法读取当前程序映像")?;
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            options.mode(0o700);
            let mut target = options
                .open(&temporary)
                .map_err(|_| "无法创建恢复程序副本")?;
            std::io::copy(&mut source.take(BINARY_LIMIT as u64 + 1), &mut target)
                .map_err(|_| "无法复制恢复程序")?;
            target.sync_all().map_err(|_| "无法同步恢复程序")?;
            if hash_handle(store::open_private(&temporary, BINARY_LIMIT, false)?)?
                != self.image_hash
            {
                return Err("恢复程序副本校验失败".into());
            }
            fs::rename(&temporary, units::WORKER).map_err(|_| "无法安装私有恢复程序")?;
            store::sync_dir(&self.store.root)?;
        }
        ledger.worker_sha256 = Some(self.image_hash.clone());
        self.store.save(&ledger)?;
        install_unit(
            Path::new(units::SERVICE_PATH),
            &units::service_content(units::WORKER),
        )?;
        install_unit(Path::new(units::TIMER_PATH), &units::timer_content())?;
        install_unit(Path::new(units::HOOK_PATH), &units::hook_content())?;
        ctl(&["daemon-reload"])?;
        validate_hook()?;
        self.compatible()
    }
}

fn install_unit(path: &Path, expected: &str) -> Result<(), String> {
    if fs::symlink_metadata(path).is_ok() {
        if read_public(path, 64 * 1024)? != expected.as_bytes() {
            return Err("未知 systemd 文件，拒绝覆盖".into());
        }
        return Ok(());
    }
    let parent = path.parent().ok_or("unit 缺少父目录")?;
    if !system::trusted_root_directory(parent) {
        return Err("systemd unit 目录不安全".into());
    }
    let temporary = parent.join(format!(".mihomo-guard-{}.tmp", store::random_id()?));
    store::write_new(&temporary, expected.as_bytes())?;
    #[cfg(unix)]
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o644))
        .map_err(|_| "无法设置 unit 权限")?;
    File::open(&temporary)
        .and_then(|file| file.sync_all())
        .map_err(|_| "无法同步 unit")?;
    // No-clobber publication: an unexpected file appearing during setup is not overwritten.
    fs::hard_link(&temporary, path).map_err(|_| "无法安装 unit；路径可能已被占用")?;
    fs::remove_file(temporary).map_err(|_| "无法清理 unit 临时文件")?;
    store::sync_dir(parent)
}

impl Backend for Host {
    fn stamp(&mut self) -> Result<Stamp, String> {
        clock::current()
    }
    fn binding(&mut self) -> Result<String, BindingError> {
        core_info().map(|(_, hash)| hash)
    }
    fn service(&mut self) -> Result<Service, String> {
        let output = ctl(&[
            "show",
            "mihomo.service",
            "--property=ActiveState",
            "--property=UnitFileState",
            "--property=InvocationID",
            "--property=MainPID",
            "--property=Job",
        ])?;
        let service = parse_service(&output)?;
        if service.active {
            let info = properties(&output);
            let pid = info.get("MainPID").ok_or("缺少核心 PID")?;
            let (_, core) = core_paths()?;
            let running = hash_handle(
                File::open(format!("/proc/{pid}/exe")).map_err(|_| "无法验证运行中核心映像")?,
            )?;
            if running != hash_handle(public_file(&core, BINARY_LIMIT)?)? {
                return Err("运行中核心与磁盘核心不同，不能建立/确认基线".into());
            }
        }
        Ok(service)
    }
    fn validate(&mut self, snapshot: &Path) -> Result<(), String> {
        let (_, binary) = core_paths()?;
        let snapshot = snapshot.to_str().ok_or("快照路径不是 UTF-8")?;
        if !run(
            &binary,
            &["-t", "-f", snapshot, "-d", "/etc/mihomo-tui"],
            15,
        )?
        .0
        {
            return Err("核心拒绝候选配置；原始诊断已隐藏，请只读排查".into());
        }
        Ok(())
    }
    fn arm(&mut self, new_trial: bool) -> Result<(), String> {
        self.compatible()?;
        Self::known_unit(
            units::SERVICE,
            units::SERVICE_PATH,
            &units::service_content(units::WORKER),
        )?;
        Self::known_unit(units::TIMER, units::TIMER_PATH, &units::timer_content())?;
        if new_trial {
            ctl(&["start", units::SERVICE])?;
            let result = ctl(&[
                "show",
                units::SERVICE,
                "--property=Result",
                "--property=ExecMainStatus",
            ])?;
            let info = properties(&result);
            if info.get("Result") != Some(&"success") || info.get("ExecMainStatus") != Some(&"0") {
                return Err("恢复单元执行验证未通过".into());
            }
        }
        ctl(&["enable", "--now", units::TIMER])?;
        let timer = ctl(&[
            "show",
            units::TIMER,
            "--property=ActiveState",
            "--property=UnitFileState",
        ])?;
        let info = properties(&timer);
        if info.get("ActiveState") != Some(&"active")
            || info.get("UnitFileState") != Some(&"enabled")
        {
            return Err("恢复 timer 未持久启用并运行".into());
        }
        Ok(())
    }
    fn disarm(&mut self) -> Result<(), String> {
        if Self::known_unit(units::TIMER, units::TIMER_PATH, &units::timer_content())? {
            ctl(&["disable", "--now", units::TIMER])?;
        }
        Ok(())
    }
    fn restart(&mut self) -> Result<(), String> {
        ctl(&["reset-failed", "mihomo.service"])?;
        ctl(&["--no-block", "restart", "mihomo.service"]).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn command_waits_are_bounded_without_touching_systemd() {
        let started = std::time::Instant::now();
        assert!(super::run(std::path::Path::new("/usr/bin/sleep"), &["3"], 1).is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(3));
    }
    #[test]
    fn invocation_timestamps_and_pids_cannot_change_the_static_service_signature() {
        let first = "Type=simple\nUser=\nRootDirectory=\nRootImage=\nFragmentPath=/lib/systemd/system/mihomo.service\nExecStart={ path=/usr/bin/mihomo ; argv[]=/usr/bin/mihomo -d /etc/mihomo-tui ; start_time=old ; pid=1 ; }\n";
        let second = first
            .replace("start_time=old", "start_time=new")
            .replace("pid=1", "pid=999");
        assert_eq!(
            super::execution_signature(first, std::path::Path::new("/usr/bin/mihomo")).unwrap(),
            super::execution_signature(&second, std::path::Path::new("/usr/bin/mihomo")).unwrap()
        );
        assert!(
            super::execution_signature(
                &first.replace("RootDirectory=", "RootDirectory=/unknown"),
                std::path::Path::new("/usr/bin/mihomo")
            )
            .is_err()
        );
    }
    #[test]
    fn service_properties_require_a_real_invocation_and_persistent_enablement() {
        let service = super::parse_service("ActiveState=active\nUnitFileState=enabled\nInvocationID=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nMainPID=123\nJob=0\n").unwrap();
        assert!(service.active && service.enabled);
        assert!(service.invocation.is_some());
        let service = super::parse_service("ActiveState=activating\nUnitFileState=enabled-runtime\nInvocationID=\nMainPID=0\nJob=123\n").unwrap();
        assert!(service.starting && service.job);
        assert!(!service.active || !service.enabled);
        assert!(super::parse_service("garbage").is_err());
    }
}
