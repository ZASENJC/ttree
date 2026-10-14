//! 腾讯混元 `HY-MT1.5-1.8B` 内置翻译模型管理器。
//!
//! 职责：
//! 1. **一键环境与硬件兼容性检测** (`check_compatibility`)：检测 CPU 架构、型号、GPU 加速能力（Metal / Vulkan）、系统内存、可用磁盘空间、推荐量化精度，以及当前已下载的精度档位与服务运行状态；
//! 2. **运行时无感准备** (`ensure_llama_server_runtime`)：优先复用应用托管目录或本机已有的 `llama-server`，若缺失则自动从 `ggml-org/llama.cpp` 官方 Release 下载对应平台的轻量预编译包（~11 MB）并自动解压赋予权限；
//! 3. **Hugging Face 全精度 GGUF 断点续传下载** (`download_model_quant`)：支持全部 8 个精度档位（`Q2_K` ~ `F16`），支持 `.part` 断点续传、`GGUF` 魔数校验、实时进度与网速回调、`CancelToken` 随时取消，并在直连 Hugging Face 超时时自动回退镜像；
//! 4. **后台推理进程托管** (`ensure_server_running` / `stop_server`)：静默拉起 `llama-server` 守护进程，轮询 `/health` 就绪后通过本地回环接口提供毫秒级流式翻译，切换精度或退出应用时自动回收子进程。

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use ttree_core::{CancelToken, HyMtQuant};

pub const APP_BUNDLE_ID: &str = "com.samwstu.ttree";
const LLAMA_CPP_PINNED_TAG: &str = "b8792";
const PREFERRED_PORT: u16 = 28190;
const GGUF_MAGIC: &[u8; 4] = b"GGUF";

/// 下载阶段与实时进度信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HyMtDownloadProgress {
    /// 当前阶段描述（如 `"正在准备推理引擎"` / `"正在下载 HY-MT1.5-1.8B (Q8_0)"`）
    pub stage: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub speed_mbps: f64,
}

impl HyMtDownloadProgress {
    pub fn percent(&self) -> f64 {
        if self.total_bytes == 0 {
            return 0.0;
        }
        ((self.downloaded_bytes as f64 / self.total_bytes as f64) * 100.0).clamp(0.0, 100.0)
    }

    pub fn format_status(&self) -> String {
        if self.total_bytes > 0 {
            let dl_mb = self.downloaded_bytes as f64 / (1024.0 * 1024.0);
            let tot_mb = self.total_bytes as f64 / (1024.0 * 1024.0);
            format!(
                "{}: {:.1}% ({:.0} MB / {:.0} MB · {:.1} MB/s)",
                self.stage,
                self.percent(),
                dl_mb,
                tot_mb,
                self.speed_mbps
            )
        } else {
            format!("{}...", self.stage)
        }
    }
}

/// 一键检测硬件与运行环境的完整报告。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HyMtCompatibilityReport {
    pub supported: bool,
    pub os_arch: String,
    pub cpu_brand: String,
    pub gpu_backend: String,
    pub total_ram_gb: f64,
    pub free_disk_gb: f64,
    pub recommended_quant: HyMtQuant,
    pub runtime_installed: bool,
    pub runtime_path: Option<String>,
    pub models_dir: String,
    pub downloaded_quants: Vec<HyMtQuant>,
    pub running_quant: Option<HyMtQuant>,
    pub running_port: Option<u16>,
}

impl HyMtCompatibilityReport {
    /// 生成面向设置面板展示的两行紧凑状态摘要。
    pub fn summary_text(&self, selected: HyMtQuant) -> String {
        let env_icon = if self.supported { "✓" } else { "⚠" };
        let env_line = format!(
            "{} {} · 内存 {:.0}GB · 磁盘 {:.0}GB · 推荐 {}",
            env_icon,
            self.cpu_brand,
            self.total_ram_gb,
            self.free_disk_gb,
            self.recommended_quant.code()
        );

        let model_line = if self.running_quant == Some(selected) {
            let port = self.running_port.unwrap_or(PREFERRED_PORT);
            format!(
                "● 运行中: HY-MT1.5 ({}) · 127.0.0.1:{}",
                selected.code(),
                port
            )
        } else if self.downloaded_quants.contains(&selected) {
            format!(
                "✓ 已下载: HY-MT1.5 ({}) · 点击「启动服务」运行",
                selected.code()
            )
        } else {
            let size_gb = selected.expected_bytes() as f64 / (1024.0 * 1024.0 * 1024.0);
            let size_label = if size_gb >= 1.0 {
                format!("{size_gb:.2} GB")
            } else {
                format!(
                    "{:.0} MB",
                    selected.expected_bytes() as f64 / (1024.0 * 1024.0)
                )
            };
            format!(
                "○ 未下载: {} (约 {}) · 点击「下载并运行」获取",
                selected.code(),
                size_label
            )
        };

        format!("{env_line}\n{model_line}")
    }
}

struct RunningHyMtServer {
    child: Child,
    quant: HyMtQuant,
    port: u16,
    base_url: String,
}

static RUNNING_SERVER: OnceLock<Mutex<Option<RunningHyMtServer>>> = OnceLock::new();
static STARTUP_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
static CUSTOM_MODELS_DIR: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

fn server_slot() -> &'static Mutex<Option<RunningHyMtServer>> {
    RUNNING_SERVER.get_or_init(|| Mutex::new(None))
}

fn startup_lock() -> &'static tokio::sync::Mutex<()> {
    STARTUP_LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

fn custom_models_dir_slot() -> &'static Mutex<Option<PathBuf>> {
    CUSTOM_MODELS_DIR.get_or_init(|| {
        let marker = local_llm_root_dir().join("models-dir.txt");
        let loaded = fs::read_to_string(&marker)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
        Mutex::new(loaded)
    })
}

fn download_http_client() -> &'static Client {
    static DL_CLIENT: OnceLock<Client> = OnceLock::new();
    DL_CLIENT.get_or_init(|| {
        Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .tcp_keepalive(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::limited(10))
            .user_agent("ttree-local-llm/0.2.0")
            .build()
            .expect("failed to build download HTTP client")
    })
}

/// 本地模型与推理引擎的根存储目录：`<config_dir>/com.samwstu.ttree/local-llm`
pub fn local_llm_root_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(APP_BUNDLE_ID)
        .join("local-llm")
}

pub fn runtime_dir() -> PathBuf {
    local_llm_root_dir().join("runtime")
}

/// 获取当前配置的模型存储文件夹（若用户通过弹窗选择过自定义文件夹则优先使用）。
pub fn models_dir() -> PathBuf {
    if let Ok(guard) = custom_models_dir_slot().lock() {
        if let Some(custom) = guard.as_ref() {
            return custom.clone();
        }
    }
    local_llm_root_dir().join("models")
}

/// 设置并持久化用户选择的模型下载保存文件夹。
pub fn set_custom_models_dir(dir: PathBuf) -> Result<PathBuf, String> {
    fs::create_dir_all(&dir).map_err(|e| format!("无法创建所选模型目录 {}: {e}", dir.display()))?;
    let root = local_llm_root_dir();
    let _ = fs::create_dir_all(&root);
    let marker = root.join("models-dir.txt");
    fs::write(&marker, dir.to_string_lossy().as_bytes())
        .map_err(|e| format!("保存模型目录配置失败: {e}"))?;
    if let Ok(mut guard) = custom_models_dir_slot().lock() {
        *guard = Some(dir.clone());
    }
    Ok(dir)
}

pub fn model_file_path(quant: HyMtQuant) -> PathBuf {
    models_dir().join(quant.file_name())
}

pub fn model_part_path(quant: HyMtQuant) -> PathBuf {
    models_dir().join(format!("{}.part", quant.file_name()))
}

/// 检查指定精度的 GGUF 模型是否已完整下载并通过 `GGUF` 魔数校验。
pub fn is_model_downloaded(quant: HyMtQuant) -> bool {
    let path = model_file_path(quant);
    validate_gguf_file(&path, quant.expected_bytes())
}

fn validate_gguf_file(path: &Path, expected_bytes: u64) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    let min_valid = expected_bytes.saturating_mul(95) / 100;
    if meta.len() < min_valid.max(1024 * 1024) {
        return false;
    }
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let mut magic = [0u8; 4];
    if file.read_exact(&mut magic).is_err() {
        return false;
    }
    &magic == GGUF_MAGIC
}

/// 列出本地已完整下载的所有 `HyMtQuant` 档位。
pub fn list_downloaded_quants() -> Vec<HyMtQuant> {
    HyMtQuant::ALL
        .into_iter()
        .filter(|&q| is_model_downloaded(q))
        .collect()
}

/// 删除指定精度的已下载模型文件（及残留 `.part` 文件），若该精度正在运行则先停止服务。
pub fn delete_model_quant(quant: HyMtQuant) -> Result<(), String> {
    if daemon_status().0 == Some(quant) {
        stop_server();
    }
    let path = model_file_path(quant);
    if path.exists() {
        fs::remove_file(&path).map_err(|e| format!("删除模型文件失败: {e}"))?;
    }
    let part = model_part_path(quant);
    if part.exists() {
        let _ = fs::remove_file(&part);
    }
    Ok(())
}

/// 查找本机已存在的 `llama-server` 可执行文件路径。
pub fn resolve_existing_llama_server() -> Option<PathBuf> {
    let bin_name = if cfg!(target_os = "windows") {
        "llama-server.exe"
    } else {
        "llama-server"
    };

    // 1. 应用托管的 runtime 目录
    let managed = runtime_dir().join(bin_name);
    if is_executable_file(&managed) {
        return Some(managed);
    }

    // 2. 应用包内置同级目录（若打包时附带）
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let bundled = dir.join(bin_name);
            if is_executable_file(&bundled) {
                return Some(bundled);
            }
        }
    }

    // 3. 常见本机路径（Homebrew / 本地编译）
    let candidates = [
        "/opt/homebrew/bin/llama-server",
        "/usr/local/bin/llama-server",
        "/Volumes/STU/Ai/llama.cpp/build/bin/llama-server",
    ];
    for c in candidates {
        let p = PathBuf::from(c);
        if is_executable_file(&p) {
            return Some(p);
        }
    }

    // 4. 系统 PATH
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join(bin_name);
            if is_executable_file(&candidate) {
                return Some(candidate);
            }
        }
    }

    None
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() || meta.len() < 4096 {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// 查询当前后台 `llama-server` 守护进程状态：`(Some(quant), Some(port))` 或 `(None, None)`。
pub fn daemon_status() -> (Option<HyMtQuant>, Option<u16>) {
    let mut guard = server_slot().lock().expect("server lock poisoned");
    if let Some(srv) = guard.as_mut() {
        match srv.child.try_wait() {
            Ok(None) => return (Some(srv.quant), Some(srv.port)),
            _ => {
                *guard = None;
            }
        }
    }
    (None, None)
}

/// 停止后台 `llama-server` 守护进程。
pub fn stop_server() {
    let mut guard = server_slot().lock().expect("server lock poisoned");
    if let Some(mut srv) = guard.take() {
        let _ = srv.child.kill();
        let _ = srv.child.wait();
    }
}

/// 执行一键硬件与环境兼容性检测。
pub fn check_compatibility() -> HyMtCompatibilityReport {
    let root = local_llm_root_dir();
    let _ = fs::create_dir_all(&root);
    let mdir = models_dir();
    let _ = fs::create_dir_all(&mdir);

    let os_arch = format!("{} ({})", std::env::consts::OS, std::env::consts::ARCH);
    let cpu_brand = detect_cpu_brand();
    let gpu_backend = detect_gpu_backend();
    let total_ram_gb = detect_total_ram_gb();
    let free_disk_gb = detect_free_disk_gb(&mdir);
    let recommended_quant = HyMtQuant::recommended_for_ram_gb(total_ram_gb.round() as u64);

    let existing_rt = resolve_existing_llama_server();
    let runtime_installed = existing_rt.is_some();
    let runtime_path = existing_rt.map(|p| p.to_string_lossy().to_string());
    let models_dir_str = mdir.to_string_lossy().to_string();

    let downloaded_quants = list_downloaded_quants();
    let (running_quant, running_port) = daemon_status();

    let supported = total_ram_gb >= 4.0 && free_disk_gb >= 1.2;

    HyMtCompatibilityReport {
        supported,
        os_arch,
        cpu_brand,
        gpu_backend,
        total_ram_gb,
        free_disk_gb,
        recommended_quant,
        runtime_installed,
        runtime_path,
        models_dir: models_dir_str,
        downloaded_quants,
        running_quant,
        running_port,
    }
}

fn detect_cpu_brand() -> String {
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = Command::new("sysctl")
            .args(["-n", "machdep.cpu.brand_string"])
            .output()
        {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return s;
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(cpuinfo) = fs::read_to_string("/proc/cpuinfo") {
            for line in cpuinfo.lines() {
                if let Some((k, v)) = line.split_once(':') {
                    if k.trim() == "model name" {
                        return v.trim().to_string();
                    }
                }
            }
        }
    }
    format!("{} CPU", std::env::consts::ARCH)
}

fn detect_gpu_backend() -> String {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "Apple Metal GPU 加速".to_string()
    } else if cfg!(target_os = "macos") {
        "macOS Accelerate / CPU".to_string()
    } else if cfg!(target_os = "windows") {
        "Vulkan / CPU 加速".to_string()
    } else {
        "CPU / Vulkan 加速".to_string()
    }
}

fn detect_total_ram_gb() -> f64 {
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = Command::new("sysctl").args(["-n", "hw.memsize"]).output() {
            if let Ok(bytes) = String::from_utf8_lossy(&out.stdout).trim().parse::<u64>() {
                return (bytes as f64) / (1024.0 * 1024.0 * 1024.0);
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(meminfo) = fs::read_to_string("/proc/meminfo") {
            for line in meminfo.lines() {
                if let Some(rest) = line.strip_prefix("MemTotal:") {
                    let kb_str = rest.trim().trim_end_matches("kB").trim();
                    if let Ok(kb) = kb_str.parse::<u64>() {
                        return (kb as f64) / (1024.0 * 1024.0);
                    }
                }
            }
        }
    }
    16.0
}

fn detect_free_disk_gb(path: &Path) -> f64 {
    let check_dir = if path.exists() {
        path.to_path_buf()
    } else {
        std::env::temp_dir()
    };
    #[cfg(unix)]
    {
        if let Ok(out) = Command::new("df").arg("-k").arg(&check_dir).output() {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if let Some(line) = stdout.lines().nth(1) {
                let cols: Vec<&str> = line.split_whitespace().collect();
                if cols.len() >= 4 {
                    if let Ok(avail_kb) = cols[3].parse::<u64>() {
                        return (avail_kb as f64) / (1024.0 * 1024.0);
                    }
                }
            }
        }
    }
    50.0
}

/// 平台对应的 `llama.cpp` 官方预编译包下载链接。
fn platform_llama_cpp_asset_url(tag: &str) -> Result<(&'static str, String), String> {
    let asset_suffix = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => ("tar.gz", format!("llama-{tag}-bin-macos-arm64.tar.gz")),
        ("macos", "x86_64") => ("tar.gz", format!("llama-{tag}-bin-macos-x64.tar.gz")),
        ("linux", "x86_64") => ("tar.gz", format!("llama-{tag}-bin-ubuntu-x64.tar.gz")),
        ("linux", "aarch64") => ("tar.gz", format!("llama-{tag}-bin-ubuntu-arm64.tar.gz")),
        ("windows", "x86_64") => ("zip", format!("llama-{tag}-bin-win-vulkan-x64.zip")),
        (os, arch) => {
            return Err(format!(
                "暂不支持自动下载 {os}/{arch} 平台的 llama-server 预编译包"
            ))
        }
    };
    let url = format!(
        "https://github.com/ggml-org/llama.cpp/releases/download/{tag}/{}",
        asset_suffix.1
    );
    Ok((asset_suffix.0, url))
}

/// 确保本机存在可用的 `llama-server` 推理引擎；若不存在则自动从 GitHub Release 下载解压。
pub async fn ensure_llama_server_runtime<F>(
    cancel: &CancelToken,
    on_progress: &mut F,
) -> Result<PathBuf, String>
where
    F: FnMut(HyMtDownloadProgress) + Send,
{
    if let Some(existing) = resolve_existing_llama_server() {
        return Ok(existing);
    }

    let rt_dir = runtime_dir();
    fs::create_dir_all(&rt_dir).map_err(|e| format!("创建运行时目录失败: {e}"))?;

    let (ext, url) = platform_llama_cpp_asset_url(LLAMA_CPP_PINNED_TAG)?;
    let archive_path = rt_dir.join(format!("llama-runtime.{ext}"));

    download_file_with_progress(
        &url,
        None,
        &archive_path,
        None,
        "正在下载轻量推理引擎 (llama-server)",
        cancel,
        on_progress,
    )
    .await?;

    on_progress(HyMtDownloadProgress {
        stage: "正在解压并配置推理引擎".to_string(),
        downloaded_bytes: 1,
        total_bytes: 1,
        speed_mbps: 0.0,
    });

    if ext == "tar.gz" {
        let status = Command::new("tar")
            .arg("-xzf")
            .arg(&archive_path)
            .arg("-C")
            .arg(&rt_dir)
            .arg("--strip-components=1")
            .status()
            .map_err(|e| format!("解压推理引擎失败: {e}"))?;
        if !status.success() {
            return Err("解压 llama-server 归档失败".to_string());
        }
    } else if ext == "zip" {
        let status = Command::new("tar")
            .arg("-xf")
            .arg(&archive_path)
            .arg("-C")
            .arg(&rt_dir)
            .status()
            .map_err(|e| format!("解压推理引擎 zip 失败: {e}"))?;
        if !status.success() {
            return Err("解压 llama-server zip 归档失败".to_string());
        }
    }

    let _ = fs::remove_file(&archive_path);

    let bin_name = if cfg!(target_os = "windows") {
        "llama-server.exe"
    } else {
        "llama-server"
    };
    let server_bin = rt_dir.join(bin_name);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if server_bin.exists() {
            let _ = fs::set_permissions(&server_bin, fs::Permissions::from_mode(0o755));
        }
    }

    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("xattr").arg("-cr").arg(&rt_dir).status();
    }

    if is_executable_file(&server_bin) {
        Ok(server_bin)
    } else {
        Err("解压完成但未找到可执行的 llama-server 二进制文件".to_string())
    }
}

/// 从 Hugging Face 下载指定精度的 `HY-MT1.5-1.8B` GGUF 模型文件（支持断点续传与魔数校验）。
pub async fn download_model_quant<F>(
    quant: HyMtQuant,
    cancel: &CancelToken,
    on_progress: &mut F,
) -> Result<PathBuf, String>
where
    F: FnMut(HyMtDownloadProgress) + Send,
{
    let target_path = model_file_path(quant);
    if is_model_downloaded(quant) {
        on_progress(HyMtDownloadProgress {
            stage: format!("模型 {} 已就绪", quant.code()),
            downloaded_bytes: quant.expected_bytes(),
            total_bytes: quant.expected_bytes(),
            speed_mbps: 0.0,
        });
        return Ok(target_path);
    }

    let dir = models_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("创建模型目录失败: {e}"))?;

    let primary_url = if let Ok(custom_ep) = std::env::var("HF_ENDPOINT") {
        let ep = custom_ep.trim().trim_end_matches('/');
        quant.hf_url().replace("https://huggingface.co", ep)
    } else {
        quant.hf_url().to_string()
    };
    let mirror_url = quant
        .hf_url()
        .replace("https://huggingface.co", "https://hf-mirror.com");

    let part_path = model_part_path(quant);
    let stage_title = format!("正在下载 HY-MT1.5-1.8B ({})", quant.code());

    download_file_with_progress(
        &primary_url,
        Some(&mirror_url),
        &part_path,
        Some(quant.expected_bytes()),
        &stage_title,
        cancel,
        on_progress,
    )
    .await?;

    if !validate_gguf_file(&part_path, quant.expected_bytes()) {
        let _ = fs::remove_file(&part_path);
        return Err(format!(
            "下载的 {} 模型文件校验未通过（非有效 GGUF 或文件不完整）",
            quant.code()
        ));
    }

    fs::rename(&part_path, &target_path).map_err(|e| format!("保存模型文件失败: {e}"))?;
    Ok(target_path)
}

async fn download_file_with_progress<F>(
    primary_url: &str,
    fallback_url: Option<&str>,
    dest_path: &Path,
    expected_total: Option<u64>,
    stage_label: &str,
    cancel: &CancelToken,
    on_progress: &mut F,
) -> Result<(), String>
where
    F: FnMut(HyMtDownloadProgress) + Send,
{
    let mut existing_len = fs::metadata(dest_path).map(|m| m.len()).unwrap_or(0);
    if let Some(exp) = expected_total {
        if existing_len > exp {
            let _ = fs::remove_file(dest_path);
            existing_len = 0;
        }
    }

    let send_req = |url: &str, range_start: u64| {
        let mut req = download_http_client().get(url);
        if range_start > 0 {
            req = req.header(reqwest::header::RANGE, format!("bytes={range_start}-"));
        }
        req
    };

    let resp = match tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err("下载已取消".to_string()),
        r = send_req(primary_url, existing_len).send() => r,
    } {
        Ok(r) if r.status().is_success() || r.status().as_u16() == 206 => r,
        Ok(r) if r.status().as_u16() == 416 => {
            // Range 越界，重置文件重下
            let _ = fs::remove_file(dest_path);
            existing_len = 0;
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err("下载已取消".to_string()),
                r2 = send_req(primary_url, 0).send() => r2.map_err(|e| format!("下载请求失败: {e}"))?,
            }
        }
        Ok(r) => {
            if let Some(fb) = fallback_url {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return Err("下载已取消".to_string()),
                    r2 = send_req(fb, existing_len).send() => r2.map_err(|e| format!("Hugging Face 下载失败 ({}): {e}", r.status()))?,
                }
            } else {
                return Err(format!("下载服务器返回状态 {}", r.status()));
            }
        }
        Err(err) => {
            if let Some(fb) = fallback_url {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return Err("下载已取消".to_string()),
                    r2 = send_req(fb, existing_len).send() => r2.map_err(|e| format!("连接 Hugging Face 失败 ({err})，备用节点亦失败: {e}"))?,
                }
            } else {
                return Err(format!("网络连接失败: {err}"));
            }
        }
    };

    if !resp.status().is_success() && resp.status().as_u16() != 206 {
        return Err(format!("下载返回 HTTP {}", resp.status()));
    }

    let is_partial = resp.status().as_u16() == 206;
    if !is_partial {
        existing_len = 0;
    }

    let content_len = resp.content_length().unwrap_or(0);
    let total_bytes = expected_total.unwrap_or_else(|| {
        if content_len > 0 {
            existing_len + content_len
        } else {
            0
        }
    });

    let mut file = if is_partial && existing_len > 0 {
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(dest_path)
            .map_err(|e| format!("打开断点续传文件失败: {e}"))?;
        let _ = f.seek(SeekFrom::End(0));
        f
    } else {
        File::create(dest_path).map_err(|e| format!("创建下载文件失败: {e}"))?
    };

    let mut downloaded = existing_len;
    let start_instant = Instant::now();
    let start_bytes = existing_len;
    let mut last_report = Instant::now() - Duration::from_secs(1);

    on_progress(HyMtDownloadProgress {
        stage: stage_label.to_string(),
        downloaded_bytes: downloaded,
        total_bytes,
        speed_mbps: 0.0,
    });

    let mut stream = resp.bytes_stream();
    loop {
        let next = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                let _ = file.flush();
                return Err("下载已取消（已保存断点进度，下次可继续下载）".to_string());
            }
            chunk = stream.next() => chunk,
        };
        let Some(chunk) = next else {
            break;
        };
        let bytes = chunk.map_err(|e| format!("下载数据流中断: {e}（可再次点击继续断点续传）"))?;
        file.write_all(&bytes)
            .map_err(|e| format!("写入磁盘失败: {e}"))?;
        downloaded += bytes.len() as u64;

        let now = Instant::now();
        if now.duration_since(last_report) >= Duration::from_millis(120) {
            last_report = now;
            let elapsed = now.duration_since(start_instant).as_secs_f64().max(0.05);
            let delta_mb = (downloaded.saturating_sub(start_bytes)) as f64 / (1024.0 * 1024.0);
            on_progress(HyMtDownloadProgress {
                stage: stage_label.to_string(),
                downloaded_bytes: downloaded,
                total_bytes,
                speed_mbps: delta_mb / elapsed,
            });
        }
    }

    file.flush().map_err(|e| format!("刷新磁盘缓存失败: {e}"))?;
    on_progress(HyMtDownloadProgress {
        stage: stage_label.to_string(),
        downloaded_bytes: downloaded,
        total_bytes: total_bytes.max(downloaded),
        speed_mbps: 0.0,
    });
    Ok(())
}

fn pick_available_port() -> Result<u16, String> {
    if TcpListener::bind(("127.0.0.1", PREFERRED_PORT)).is_ok() {
        return Ok(PREFERRED_PORT);
    }
    let listener =
        TcpListener::bind(("127.0.0.1", 0)).map_err(|e| format!("分配本地端口失败: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("读取本地端口失败: {e}"))?
        .port();
    Ok(port)
}

/// 一键下载（若尚未下载）并启动指定精度的 `HY-MT1.5-1.8B` 服务。
pub async fn download_and_start<F>(
    quant: HyMtQuant,
    cancel: &CancelToken,
    mut on_progress: F,
) -> Result<String, String>
where
    F: FnMut(HyMtDownloadProgress) + Send,
{
    let _server_bin = ensure_llama_server_runtime(cancel, &mut on_progress).await?;
    let _model_path = download_model_quant(quant, cancel, &mut on_progress).await?;
    on_progress(HyMtDownloadProgress {
        stage: format!("正在启动 HY-MT1.5-1.8B ({}) 本地推理服务", quant.code()),
        downloaded_bytes: 1,
        total_bytes: 1,
        speed_mbps: 0.0,
    });
    ensure_server_running(quant, cancel).await
}

/// 确保指定精度的 `llama-server` 守护进程正在运行，返回其 OpenAI 兼容 `base_url`（如 `http://127.0.0.1:28190/v1`）。
pub async fn ensure_server_running(
    quant: HyMtQuant,
    cancel: &CancelToken,
) -> Result<String, String> {
    // 快路径：检查是否已在运行相同精度
    {
        let mut guard = server_slot().lock().expect("server lock poisoned");
        if let Some(srv) = guard.as_mut() {
            if srv.quant == quant && matches!(srv.child.try_wait(), Ok(None)) {
                return Ok(srv.base_url.clone());
            }
        }
    }

    let _guard = startup_lock().lock().await;

    // 二次检查
    {
        let mut guard = server_slot().lock().expect("server lock poisoned");
        if let Some(srv) = guard.as_mut() {
            if srv.quant == quant && matches!(srv.child.try_wait(), Ok(None)) {
                return Ok(srv.base_url.clone());
            }
            // 精度不同或已退出，清理旧进程
            if let Some(mut old) = guard.take() {
                let _ = old.child.kill();
                let _ = old.child.wait();
            }
        }
    }

    if !is_model_downloaded(quant) {
        return Err(format!(
            "内置模型 HY-MT1.5-1.8B ({}) 尚未下载，请先在「设置 → AI 翻译」点击「一键下载并运行」",
            quant.code()
        ));
    }

    let mut noop_progress = |_: HyMtDownloadProgress| {};
    let server_bin = ensure_llama_server_runtime(cancel, &mut noop_progress).await?;
    let model_path = model_file_path(quant);
    let port = pick_available_port()?;

    let root = local_llm_root_dir();
    let _ = fs::create_dir_all(&root);
    let log_path = root.join("llama-server.log");
    let log_file = File::create(&log_path).unwrap_or_else(|_| File::open("/dev/null").unwrap());
    let log_err = log_file
        .try_clone()
        .unwrap_or_else(|_| File::open("/dev/null").unwrap());

    let mut cmd = Command::new(&server_bin);
    cmd.arg("-m")
        .arg(&model_path)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .arg("--ctx-size")
        .arg("4096")
        .arg("-ngl")
        .arg("auto")
        .arg("--temp")
        .arg("0.7")
        .arg("--top-k")
        .arg("20")
        .arg("--top-p")
        .arg("0.6")
        .arg("--repeat-penalty")
        .arg("1.05")
        .arg("--no-webui")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_err));

    if let Some(bin_dir) = server_bin.parent() {
        #[cfg(target_os = "macos")]
        cmd.env("DYLD_LIBRARY_PATH", bin_dir);
        #[cfg(target_os = "linux")]
        cmd.env("LD_LIBRARY_PATH", bin_dir);
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let child = cmd
        .spawn()
        .map_err(|e| format!("启动 llama-server 失败 ({}): {e}", server_bin.display()))?;

    let base_url = format!("http://127.0.0.1:{port}/v1");
    let health_url = format!("http://127.0.0.1:{port}/health");

    {
        let mut slot = server_slot().lock().expect("server lock poisoned");
        *slot = Some(RunningHyMtServer {
            child,
            quant,
            port,
            base_url: base_url.clone(),
        });
    }

    // 轮询 /health 等待模型加载就绪
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        if cancel.is_cancelled() {
            stop_server();
            return Err("启动已取消".to_string());
        }

        // 检查子进程是否异常退出
        {
            let mut slot = server_slot().lock().expect("server lock poisoned");
            if let Some(srv) = slot.as_mut() {
                if let Ok(Some(status)) = srv.child.try_wait() {
                    *slot = None;
                    let tail = fs::read_to_string(&log_path)
                        .unwrap_or_default()
                        .lines()
                        .rev()
                        .take(5)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect::<Vec<_>>()
                        .join(" | ");
                    return Err(format!("llama-server 意外退出 ({status}): {tail}"));
                }
            }
        }

        if let Ok(resp) = download_http_client()
            .get(&health_url)
            .timeout(Duration::from_millis(800))
            .send()
            .await
        {
            if resp.status().is_success() {
                return Ok(base_url);
            }
        }

        if Instant::now() >= deadline {
            stop_server();
            return Err("等待内置 HY-MT1.5 模型加载超时（45s）".to_string());
        }

        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compatibility_report_detects_system_resources() {
        let report = check_compatibility();
        assert!(!report.os_arch.is_empty());
        assert!(!report.cpu_brand.is_empty());
        assert!(report.total_ram_gb >= 1.0);
        assert!(report.free_disk_gb >= 0.1);
        let summary = report.summary_text(HyMtQuant::Q8_0);
        assert!(summary.contains("内存"));
        assert!(summary.contains("Q8_0"));
    }

    #[test]
    fn validates_gguf_magic_and_size() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.gguf");
        {
            let mut f = File::create(&file_path).unwrap();
            f.write_all(b"GGUF").unwrap();
            f.write_all(&vec![0u8; 2 * 1024 * 1024]).unwrap();
        }
        assert!(validate_gguf_file(&file_path, 2 * 1024 * 1024));
        assert!(!validate_gguf_file(&file_path, 100 * 1024 * 1024));
    }
}
