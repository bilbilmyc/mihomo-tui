use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Output},
    time::Duration,
};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

const MIHOMO_VERSION: &str = "v1.19.29";
const MIHOMO_DEB_VERSION: &str = "1.19.29";
const MAX_PACKAGE_BYTES: u64 = 128 * 1024 * 1024;
const BINARY_PATHS: [&str; 2] = ["/usr/bin/mihomo", "/usr/local/bin/mihomo"];
const UNIT_PATHS: [&str; 3] = [
    "/etc/systemd/system/mihomo.service",
    "/usr/lib/systemd/system/mihomo.service",
    "/lib/systemd/system/mihomo.service",
];
const MANAGED_DROP_IN: &str = "/etc/systemd/system/mihomo.service.d/10-mihomo-tui.conf";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeMode {
    External,
    ManagedLocal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inventory {
    pub binary: bool,
    pub unit: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    Prepare,
    Install,
    RejectPartialInstall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Package {
    asset: &'static str,
    sha256: &'static str,
    deb_arch: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnitExpectation {
    Absent,
    Packaged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SystemdUnitState {
    load_state: String,
    fragment_path: Option<PathBuf>,
    drop_in_paths: Vec<PathBuf>,
}

struct RuntimeLock {
    _file: File,
}

pub fn plan(mode: RuntimeMode, inventory: Inventory, auto_install: bool) -> Action {
    if mode == RuntimeMode::External {
        return Action::None;
    }
    match (inventory.binary, inventory.unit) {
        (true, true) => Action::Prepare,
        (false, false) if auto_install => Action::Install,
        (false, false) => Action::None,
        _ => Action::RejectPartialInstall,
    }
}

pub fn ensure(mode: RuntimeMode, auto_install: bool) -> Result<(), String> {
    if mode == RuntimeMode::External {
        return Ok(());
    }
    let inventory = inspect();
    match plan(mode, inventory, auto_install) {
        Action::None => Ok(()),
        Action::RejectPartialInstall => Err(partial_install_error(inventory)),
        Action::Prepare | Action::Install => {
            if !effective_root() {
                return Err(root_required_message().into());
            }
            let _lock = acquire_runtime_lock()?;
            let inventory = inspect();
            match plan(mode, inventory, auto_install) {
                Action::None => Ok(()),
                Action::Prepare => prepare_existing_for_apply(),
                Action::Install => install_for_apply(),
                Action::RejectPartialInstall => Err(partial_install_error(inventory)),
            }
        }
    }
}

pub(crate) fn root_required_message() -> &'static str {
    "权限不足：自动安装或管理本机 Mihomo 需要 root 权限。\n请在原命令前添加 sudo 重新运行，例如：sudo mihomo-tui\n如果只连接已有实例，请使用 --controller。"
}

fn partial_install_error(inventory: Inventory) -> String {
    format!(
        "检测到不完整的 Mihomo 安装（binary={}，service={}），为避免覆盖现有文件已停止；请修复后重试或使用 --controller",
        inventory.binary, inventory.unit
    )
}

fn acquire_runtime_lock() -> Result<RuntimeLock, String> {
    let path = Path::new("/run/mihomo-tui.lock");
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options
        .open(path)
        .map_err(|error| format!("无法打开 Mihomo 运行时锁：{error}"))?;
    #[cfg(unix)]
    {
        let metadata = file
            .metadata()
            .map_err(|error| format!("无法检查 Mihomo 运行时锁：{error}"))?;
        if !metadata.file_type().is_file() || metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
            return Err("/run/mihomo-tui.lock 的所有者或权限不安全".into());
        }
    }
    file.try_lock()
        .map_err(|error| format!("另一个 mihomo-tui 正在管理本机服务：{error}"))?;
    Ok(RuntimeLock { _file: file })
}

fn prepare_existing_for_apply() -> Result<(), String> {
    validate_existing_install()?;
    validate_systemd_unit(UnitExpectation::Packaged)
}

pub fn reload_service() -> Result<(), String> {
    ensure_systemd()?;
    validate_systemd_unit(UnitExpectation::Packaged)?;
    let systemctl = systemctl_path()?;
    let (description, arguments) = config_apply_command();
    let output = clean_command(&systemctl)
        .args(arguments)
        .output()
        .map_err(|error| format!("无法执行 {description}：{error}"))?;
    checked_output(description, output).map(|_| ())
}

fn config_apply_command() -> (&'static str, [&'static str; 2]) {
    (
        "systemctl reload-or-restart mihomo.service",
        ["reload-or-restart", "mihomo.service"],
    )
}

pub fn validate_config(candidate: &Path, data_dir: &Path) -> Result<Output, String> {
    let binary = mihomo_binary_path()
        .ok_or_else(|| "未在受信任路径中找到 Mihomo（/usr/bin 或 /usr/local/bin）".to_string())?;
    clean_command(&binary)
        .args(["-t", "-f"])
        .arg(candidate)
        .arg("-d")
        .arg(data_dir)
        .output()
        .map_err(|error| format!("无法执行 Mihomo 配置校验：{error}"))
}

pub fn configure_workspace_service(config_path: &Path) -> Result<(), String> {
    if config_path != Path::new(crate::workspace::DEFAULT_SOURCE_PATH) {
        return Err(format!(
            "本机托管模式只支持 {}",
            crate::workspace::DEFAULT_SOURCE_PATH
        ));
    }
    if !effective_root() {
        return Err(root_required_message().into());
    }
    validate_systemd_unit(UnitExpectation::Packaged)?;
    let binary = mihomo_binary_path().ok_or_else(|| "找不到受信任的 Mihomo".to_string())?;
    let drop_in = Path::new(MANAGED_DROP_IN);
    let directory = drop_in
        .parent()
        .ok_or_else(|| "systemd drop-in 路径没有父目录".to_string())?;
    let existed = fs::symlink_metadata(directory).is_ok();
    fs::create_dir_all(directory)
        .map_err(|error| format!("无法创建 Mihomo systemd drop-in 目录：{error}"))?;
    #[cfg(unix)]
    if !existed {
        fs::set_permissions(directory, fs::Permissions::from_mode(0o755))
            .map_err(|error| format!("无法设置 Mihomo systemd drop-in 目录权限：{error}"))?;
    }
    let metadata = fs::symlink_metadata(directory)
        .map_err(|error| format!("无法检查 Mihomo systemd drop-in 目录：{error}"))?;
    if !metadata.file_type().is_dir() {
        return Err(format!("{} 不是普通目录", directory.display()));
    }
    #[cfg(unix)]
    {
        if metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
            return Err(format!("{} 的所有者或权限不安全", directory.display()));
        }
    }
    let content = workspace_drop_in_content(&binary);
    if fs::symlink_metadata(drop_in).is_ok() && !trusted_root_file(drop_in) {
        return Err(format!("{} 不是安全的 root 普通文件", drop_in.display()));
    }
    if fs::read_to_string(drop_in).ok().as_deref() == Some(content.as_str()) {
        return daemon_reload();
    }
    let candidate = directory.join(format!(".10-mihomo-tui-{}.conf", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o644);
    let mut file = options
        .open(&candidate)
        .map_err(|error| format!("无法创建 Mihomo systemd drop-in 候选：{error}"))?;
    if let Err(error) = file
        .write_all(content.as_bytes())
        .and_then(|()| file.sync_all())
    {
        let _ = fs::remove_file(&candidate);
        return Err(format!("无法写入 Mihomo systemd drop-in：{error}"));
    }
    if let Err(error) = fs::rename(&candidate, drop_in) {
        let _ = fs::remove_file(&candidate);
        return Err(format!("无法安装 Mihomo systemd drop-in：{error}"));
    }
    fs::File::open(directory)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("无法同步 Mihomo systemd drop-in 目录：{error}"))?;
    daemon_reload()
}

fn workspace_drop_in_content(binary: &Path) -> String {
    format!(
        "[Service]\nExecStart=\nExecStart={} -d /etc/mihomo-tui\n",
        binary.display()
    )
}

fn inspect() -> Inventory {
    Inventory {
        binary: BINARY_PATHS.iter().any(path_present),
        unit: UNIT_PATHS.iter().any(path_present),
    }
}

fn path_present(path: impl AsRef<Path>) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn validate_existing_install() -> Result<(), String> {
    for (kind, paths) in [
        ("binary", BINARY_PATHS.as_slice()),
        ("service", UNIT_PATHS.as_slice()),
    ] {
        for path in paths.iter().filter(|path| path_present(path)) {
            if !trusted_root_file(Path::new(path)) {
                return Err(format!(
                    "Mihomo {kind} 文件 {path} 不是安全的 root 普通文件，拒绝启动"
                ));
            }
        }
    }
    Ok(())
}

fn mihomo_binary_path() -> Option<PathBuf> {
    BINARY_PATHS
        .iter()
        .map(PathBuf::from)
        .find(|path| trusted_root_file(path))
}

fn systemctl_path() -> Result<PathBuf, String> {
    ["/usr/bin/systemctl", "/bin/systemctl"]
        .into_iter()
        .map(PathBuf::from)
        .find(|path| trusted_root_file(path))
        .ok_or_else(|| "找不到受信任的 systemctl 可执行文件".to_string())
}

fn parse_systemd_unit(output: &str) -> Result<SystemdUnitState, String> {
    let mut load_state = None;
    let mut fragment_path = None;
    let mut drop_in_paths = None;
    for line in output.lines() {
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        match name {
            "LoadState" => load_state = Some(value.trim().to_string()),
            "FragmentPath" => {
                fragment_path =
                    Some((!value.trim().is_empty()).then(|| PathBuf::from(value.trim())))
            }
            "DropInPaths" => {
                drop_in_paths = Some(value.split_whitespace().map(PathBuf::from).collect())
            }
            _ => {}
        }
    }
    Ok(SystemdUnitState {
        load_state: load_state
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "systemd 未返回 Mihomo 服务的 LoadState".to_string())?,
        fragment_path: fragment_path
            .ok_or_else(|| "systemd 未返回 Mihomo 服务的 FragmentPath".to_string())?,
        drop_in_paths: drop_in_paths
            .ok_or_else(|| "systemd 未返回 Mihomo 服务的 DropInPaths".to_string())?,
    })
}

fn validate_systemd_unit_state(
    state: &SystemdUnitState,
    expectation: UnitExpectation,
) -> Result<(), String> {
    let absent = state.load_state == "not-found"
        && state.fragment_path.is_none()
        && state.drop_in_paths.is_empty();
    let managed_drop_ins = state.drop_in_paths.is_empty()
        || state.drop_in_paths.as_slice() == [PathBuf::from(MANAGED_DROP_IN)];
    let packaged = state.load_state == "loaded"
        && managed_drop_ins
        && state.fragment_path.as_deref().is_some_and(|path| {
            UNIT_PATHS
                .iter()
                .any(|expected| path == Path::new(expected))
        });
    let valid = match expectation {
        UnitExpectation::Absent => absent,
        UnitExpectation::Packaged => packaged,
    };
    if valid {
        return Ok(());
    }
    Err(format!(
        "systemd 中存在未受管理的 mihomo.service（LoadState={}，FragmentPath={}，DropIns={}），拒绝继续",
        state.load_state,
        state
            .fragment_path
            .as_deref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "<none>".into()),
        state
            .drop_in_paths
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(" ")
    ))
}

fn validate_systemd_unit(expectation: UnitExpectation) -> Result<(), String> {
    ensure_systemd()?;
    let systemctl = systemctl_path()?;
    let output = clean_command(&systemctl)
        .args([
            "show",
            "mihomo.service",
            "--property=LoadState",
            "--property=FragmentPath",
            "--property=DropInPaths",
            "--no-pager",
        ])
        .output()
        .map_err(|error| format!("无法查询 systemd 中的 Mihomo 服务：{error}"))?;
    let output = checked_output("systemctl show mihomo.service", output)?;
    let output = String::from_utf8(output.stdout)
        .map_err(|_| "systemd 返回的 Mihomo 服务信息不是 UTF-8".to_string())?;
    let state = parse_systemd_unit(&output)?;
    validate_systemd_unit_state(&state, expectation)?;
    if let Some(path) = state.fragment_path.as_deref()
        && !trusted_root_file(path)
    {
        return Err(format!(
            "systemd 加载的 Mihomo 服务文件 {} 不是安全的 root 普通文件",
            path.display()
        ));
    }
    for path in &state.drop_in_paths {
        if !trusted_root_file(path) {
            return Err(format!(
                "systemd 加载的 Mihomo drop-in {} 不是安全的 root 普通文件",
                path.display()
            ));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn trusted_root_file(path: &Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    metadata.file_type().is_file() && metadata.uid() == 0 && metadata.mode() & 0o022 == 0
}

#[cfg(not(unix))]
fn trusted_root_file(_path: &Path) -> bool {
    false
}

fn ensure_systemd() -> Result<(), String> {
    systemctl_path()?;
    if !Path::new("/run/systemd/system").is_dir() {
        return Err("当前系统没有可用的 systemd；请使用 --controller 连接外部 Mihomo".into());
    }
    Ok(())
}

fn clean_command(program: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("LANG", "C");
    command
}

fn run_privileged(program: &Path, args: &[&str]) -> Result<Output, String> {
    if !effective_root() {
        return Err("该操作需要 root 权限".into());
    }
    let mut command = clean_command(program);
    command
        .args(args)
        .output()
        .map_err(|error| format!("无法执行 {}：{error}", program.display()))
}

#[cfg(unix)]
fn effective_root() -> bool {
    fs::metadata("/proc/self")
        .map(|metadata| metadata.uid() == 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn effective_root() -> bool {
    false
}

fn checked_output(action: &str, output: Output) -> Result<Output, String> {
    if output.status.success() {
        return Ok(output);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = if !stderr.trim().is_empty() {
        stderr.trim()
    } else if !stdout.trim().is_empty() {
        stdout.trim()
    } else {
        "无诊断输出"
    };
    Err(format!("{action} 失败（{}）：{detail}", output.status))
}

fn install_for_apply() -> Result<(), String> {
    ensure_systemd()?;
    ensure_debian_host()?;
    validate_systemd_unit(UnitExpectation::Absent)?;
    let package = package_for(std::env::consts::OS, std::env::consts::ARCH)?;
    eprintln!("未检测到 Mihomo，正在下载官方 {MIHOMO_VERSION} 安装包...");
    let artifact = download_package(package)?;
    eprintln!("安装包校验通过，正在安装 Mihomo 服务...");
    install_deb(&artifact.path)?;
    verify_installed_version()?;
    daemon_reload()?;
    eprintln!("Mihomo {MIHOMO_VERSION} 已安装，正在加载唯一配置。");
    Ok(())
}

fn package_for(os: &str, arch: &str) -> Result<Package, String> {
    if os != "linux" {
        return Err(format!("不支持在 {os} 上自动安装 Mihomo"));
    }
    match arch {
        "x86_64" => Ok(Package {
            asset: "mihomo-linux-amd64-v1-v1.19.29.deb",
            sha256: "6919c50b403a60c3956d07e776c06e1b11bd466e6b05341c1605ce450f79a591",
            deb_arch: "amd64",
        }),
        "aarch64" => Ok(Package {
            asset: "mihomo-linux-arm64-v1.19.29.deb",
            sha256: "a14e694a2bac6ca3848e05f4ef27596c5982dab812c23743823e7e5c35f7cfc9",
            deb_arch: "arm64",
        }),
        _ => Err(format!("不支持为 {arch} 自动选择 Mihomo 安装包")),
    }
}

fn package_url(package: Package) -> String {
    format!(
        "https://github.com/MetaCubeX/mihomo/releases/download/{MIHOMO_VERSION}/{}",
        package.asset
    )
}

fn allowed_release_url(url: &reqwest::Url) -> bool {
    if url.scheme() != "https" {
        return false;
    }
    match url.host_str() {
        Some("github.com") => url
            .path()
            .starts_with("/MetaCubeX/mihomo/releases/download/"),
        Some(
            "release-assets.githubusercontent.com"
            | "objects.githubusercontent.com"
            | "github-releases.githubusercontent.com",
        ) => true,
        _ => false,
    }
}

struct TempArtifact {
    path: PathBuf,
}

impl TempArtifact {
    fn create(extension: &str) -> Result<(Self, File), String> {
        let name = format!("mihomo-tui-{}.{}", random_hex(16)?, extension);
        let path = secure_temp_dir()?.join(name);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let file = options
            .open(&path)
            .map_err(|error| format!("无法创建临时文件 {}：{error}", path.display()))?;
        Ok((Self { path }, file))
    }
}

#[cfg(unix)]
fn secure_temp_dir() -> Result<&'static Path, String> {
    let path = Path::new("/tmp");
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("无法检查系统临时目录 /tmp：{error}"))?;
    let mode = metadata.mode();
    if !metadata.file_type().is_dir()
        || metadata.uid() != 0
        || (mode & 0o022 != 0 && mode & 0o1000 == 0)
    {
        return Err("/tmp 的所有者或权限不安全，拒绝创建特权临时文件".into());
    }
    Ok(path)
}

#[cfg(not(unix))]
fn secure_temp_dir() -> Result<&'static Path, String> {
    Err("自动安装仅支持 Unix 系统".into())
}

impl Drop for TempArtifact {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn random_hex(bytes: usize) -> Result<String, String> {
    let mut random = vec![0_u8; bytes];
    File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut random))
        .map_err(|error| format!("无法从系统随机源读取数据：{error}"))?;
    let mut encoded = String::with_capacity(bytes * 2);
    for byte in random {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").map_err(|error| error.to_string())?;
    }
    Ok(encoded)
}

fn download_package(package: Package) -> Result<TempArtifact, String> {
    let policy = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 5 {
            attempt.error("Mihomo 安装包重定向次数过多")
        } else if allowed_release_url(attempt.url()) {
            attempt.follow()
        } else {
            attempt.error("Mihomo 安装包被重定向到非 GitHub 域名")
        }
    });
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(300))
        .redirect(policy)
        .user_agent(concat!("mihomo-tui/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| format!("无法创建下载客户端：{error}"))?;
    let mut response = client
        .get(package_url(package))
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| format!("下载 Mihomo 安装包失败：{error}"))?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_PACKAGE_BYTES)
    {
        return Err(format!(
            "Mihomo 安装包过大（上限 {MAX_PACKAGE_BYTES} 字节）"
        ));
    }
    let (artifact, mut file) = TempArtifact::create("deb")?;
    copy_verified(&mut response, &mut file, MAX_PACKAGE_BYTES, package.sha256)?;
    file.sync_all()
        .map_err(|error| format!("无法同步临时安装包：{error}"))?;
    drop(file);
    validate_deb_file(&artifact.path, package)?;
    Ok(artifact)
}

fn validate_deb_file(path: &Path, package: Package) -> Result<(), String> {
    let dpkg_deb = Path::new("/usr/bin/dpkg-deb");
    if !trusted_root_file(dpkg_deb) {
        return Err("找不到受信任的 /usr/bin/dpkg-deb".into());
    }
    let output = clean_command(dpkg_deb)
        .arg("--field")
        .arg(path)
        .args(["Package", "Version", "Architecture"])
        .output()
        .map_err(|error| format!("无法检查 Mihomo deb 元数据：{error}"))?;
    let output = checked_output("dpkg-deb --field", output)?;
    let metadata =
        String::from_utf8(output.stdout).map_err(|_| "Mihomo deb 元数据不是 UTF-8".to_string())?;
    validate_deb_metadata(&metadata, package)
}

fn ensure_debian_host() -> Result<(), String> {
    if !Path::new("/etc/debian_version").is_file() {
        return Err("自动安装目前仅支持 Debian/Ubuntu；请使用 --controller 连接外部 Mihomo".into());
    }
    for tool in ["/usr/bin/dpkg", "/usr/bin/dpkg-deb", "/usr/bin/install"] {
        if !trusted_root_file(Path::new(tool)) {
            return Err(format!("找不到受信任的系统工具 {tool}"));
        }
    }
    Ok(())
}

fn install_deb(path: &Path) -> Result<(), String> {
    let path = path
        .to_str()
        .ok_or_else(|| "临时安装包路径不是 UTF-8".to_string())?;
    let output = run_privileged(
        Path::new("/usr/bin/dpkg"),
        &["--force-confold", "--install", path],
    )?;
    checked_output("dpkg 安装 Mihomo", output).map(|_| ())
}

fn verify_installed_version() -> Result<(), String> {
    let binary = Path::new("/usr/bin/mihomo");
    if !trusted_root_file(binary) {
        return Err("安装完成后未找到受信任的 /usr/bin/mihomo".into());
    }
    let output = clean_command(binary)
        .arg("-v")
        .output()
        .map_err(|error| format!("无法检查 Mihomo 版本：{error}"))?;
    let output = checked_output("mihomo -v", output)?;
    let version = String::from_utf8_lossy(&output.stdout);
    if !version.contains(MIHOMO_VERSION) {
        return Err(format!(
            "安装后的 Mihomo 版本不匹配：期望 {MIHOMO_VERSION}，实际 {}",
            version.trim()
        ));
    }
    Ok(())
}

fn daemon_reload() -> Result<(), String> {
    let systemctl = systemctl_path()?;
    let output = run_privileged(&systemctl, &["daemon-reload"])?;
    checked_output("systemctl daemon-reload", output).map(|_| ())
}

fn validate_deb_metadata(metadata: &str, package: Package) -> Result<(), String> {
    let field = |name: &str| {
        metadata.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            (key.trim() == name).then(|| value.trim())
        })
    };
    let expected = [
        ("Package", "mihomo"),
        ("Version", MIHOMO_DEB_VERSION),
        ("Architecture", package.deb_arch),
    ];
    for (name, expected_value) in expected {
        let actual = field(name).ok_or_else(|| format!("安装包缺少 {name} 元数据"))?;
        if actual != expected_value {
            return Err(format!(
                "安装包 {name} 不匹配：期望 {expected_value}，实际 {actual}"
            ));
        }
    }
    Ok(())
}

fn copy_verified(
    mut reader: impl Read,
    mut writer: impl Write,
    max_bytes: u64,
    expected_sha256: &str,
) -> Result<u64, String> {
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .ok_or_else(|| "Mihomo 安装包大小溢出".to_string())?;
        if total > max_bytes {
            return Err(format!("Mihomo 安装包过大（上限 {max_bytes} 字节）"));
        }
        hasher.update(&buffer[..read]);
        writer
            .write_all(&buffer[..read])
            .map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|error| error.to_string())?;
    let actual = format!("{:x}", hasher.finalize());
    if actual != expected_sha256 {
        return Err(format!(
            "Mihomo 安装包 SHA-256 校验失败：期望 {expected_sha256}，实际 {actual}"
        ));
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_controller_never_manages_the_local_runtime() {
        let inventory = Inventory {
            binary: false,
            unit: false,
        };

        assert_eq!(plan(RuntimeMode::External, inventory, true), Action::None);
    }

    #[test]
    fn clean_local_machine_is_installed_when_auto_install_is_enabled() {
        let inventory = Inventory {
            binary: false,
            unit: false,
        };

        assert_eq!(
            plan(RuntimeMode::ManagedLocal, inventory, true),
            Action::Install
        );
    }

    #[test]
    fn clean_local_machine_stays_untouched_when_auto_install_is_disabled() {
        let inventory = Inventory {
            binary: false,
            unit: false,
        };

        assert_eq!(
            plan(RuntimeMode::ManagedLocal, inventory, false),
            Action::None
        );
    }

    #[test]
    fn complete_local_install_is_prepared_without_downloading() {
        let inventory = Inventory {
            binary: true,
            unit: true,
        };

        assert_eq!(
            plan(RuntimeMode::ManagedLocal, inventory, true),
            Action::Prepare
        );
    }

    #[test]
    fn partial_install_is_not_overwritten() {
        for inventory in [
            Inventory {
                binary: true,
                unit: false,
            },
            Inventory {
                binary: false,
                unit: true,
            },
        ] {
            assert_eq!(
                plan(RuntimeMode::ManagedLocal, inventory, true),
                Action::RejectPartialInstall
            );
        }
    }

    #[test]
    fn supported_debian_architectures_use_pinned_packages() {
        let amd64 = package_for("linux", "x86_64").unwrap();
        assert_eq!(amd64.asset, "mihomo-linux-amd64-v1-v1.19.29.deb");
        assert_eq!(
            amd64.sha256,
            "6919c50b403a60c3956d07e776c06e1b11bd466e6b05341c1605ce450f79a591"
        );

        let arm64 = package_for("linux", "aarch64").unwrap();
        assert_eq!(arm64.asset, "mihomo-linux-arm64-v1.19.29.deb");
        assert_eq!(
            arm64.sha256,
            "a14e694a2bac6ca3848e05f4ef27596c5982dab812c23743823e7e5c35f7cfc9"
        );
        assert!(package_for("linux", "mips").is_err());
        assert!(package_for("macos", "x86_64").is_err());
    }

    #[test]
    fn package_bytes_are_limited_and_sha256_verified() {
        let mut output = Vec::new();
        copy_verified(
            std::io::Cursor::new(b"abc"),
            &mut output,
            3,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        )
        .unwrap();
        assert_eq!(output, b"abc");

        let too_large = copy_verified(
            std::io::Cursor::new(b"abcd"),
            Vec::new(),
            3,
            "88d4266fd4e6338d13b845fcf289579d209c897823b9217da3e161936f031589",
        )
        .unwrap_err();
        assert!(too_large.contains("过大"));

        let mismatch = copy_verified(
            std::io::Cursor::new(b"abc"),
            Vec::new(),
            3,
            "0000000000000000000000000000000000000000000000000000000000000000",
        )
        .unwrap_err();
        assert!(mismatch.contains("SHA-256"));
    }

    #[test]
    fn deb_metadata_must_match_the_pinned_package() {
        let package = package_for("linux", "x86_64").unwrap();
        validate_deb_metadata(
            "Package: mihomo\nVersion: 1.19.29\nArchitecture: amd64\n",
            package,
        )
        .unwrap();

        assert!(
            validate_deb_metadata(
                "Package: other\nVersion: 1.19.29\nArchitecture: amd64\n",
                package,
            )
            .is_err()
        );
        assert!(
            validate_deb_metadata(
                "Package: mihomo\nVersion: 1.19.29\nArchitecture: arm64\n",
                package,
            )
            .is_err()
        );
    }

    #[test]
    fn download_url_and_redirects_are_restricted_to_github_release_hosts() {
        let package = package_for("linux", "x86_64").unwrap();
        assert_eq!(
            package_url(package),
            "https://github.com/MetaCubeX/mihomo/releases/download/v1.19.29/mihomo-linux-amd64-v1-v1.19.29.deb"
        );

        assert!(allowed_release_url(
            &reqwest::Url::parse("https://github.com/MetaCubeX/mihomo/releases/download/v/file")
                .unwrap()
        ));
        assert!(allowed_release_url(
            &reqwest::Url::parse("https://release-assets.githubusercontent.com/file").unwrap()
        ));
        assert!(!allowed_release_url(
            &reqwest::Url::parse("http://github.com/file").unwrap()
        ));
        assert!(!allowed_release_url(
            &reqwest::Url::parse("https://example.com/file").unwrap()
        ));
    }

    #[cfg(unix)]
    #[test]
    fn world_writable_runtime_files_are_not_trusted() {
        use std::os::unix::fs::PermissionsExt;

        let path = Path::new("/tmp").join(format!(
            "mihomo-tui-untrusted-{}-{}",
            std::process::id(),
            random_hex(8).unwrap()
        ));
        fs::write(&path, b"not executable").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();

        assert!(!trusted_root_file(&path));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn applying_an_edited_config_starts_an_inactive_service() {
        let (description, arguments) = config_apply_command();

        assert_eq!(description, "systemctl reload-or-restart mihomo.service");
        assert_eq!(arguments, ["reload-or-restart", "mihomo.service"]);
    }

    #[test]
    fn systemd_units_outside_the_managed_paths_are_rejected() {
        let hidden = parse_systemd_unit(
            "LoadState=loaded\nFragmentPath=/run/systemd/system/mihomo.service\nDropInPaths=\n",
        )
        .unwrap();
        assert!(validate_systemd_unit_state(&hidden, UnitExpectation::Packaged).is_err());

        let packaged = parse_systemd_unit(
            "LoadState=loaded\nFragmentPath=/usr/lib/systemd/system/mihomo.service\nDropInPaths=\n",
        )
        .unwrap();
        assert!(validate_systemd_unit_state(&packaged, UnitExpectation::Packaged).is_ok());

        let overridden = parse_systemd_unit(
            "LoadState=loaded\nFragmentPath=/usr/lib/systemd/system/mihomo.service\nDropInPaths=/run/systemd/system.control/mihomo.service.d/50-CPUQuota.conf\n",
        )
        .unwrap();
        assert!(validate_systemd_unit_state(&overridden, UnitExpectation::Packaged).is_err());

        let managed = parse_systemd_unit(&format!(
            "LoadState=loaded\nFragmentPath=/usr/lib/systemd/system/mihomo.service\nDropInPaths={MANAGED_DROP_IN}\n"
        ))
        .unwrap();
        assert!(validate_systemd_unit_state(&managed, UnitExpectation::Packaged).is_ok());

        let absent =
            parse_systemd_unit("LoadState=not-found\nFragmentPath=\nDropInPaths=\n").unwrap();
        assert!(validate_systemd_unit_state(&absent, UnitExpectation::Absent).is_ok());
    }

    #[test]
    fn managed_drop_in_points_mihomo_at_the_single_config_directory() {
        assert_eq!(
            workspace_drop_in_content(Path::new("/usr/bin/mihomo")),
            "[Service]\nExecStart=\nExecStart=/usr/bin/mihomo -d /etc/mihomo-tui\n"
        );
    }
}
