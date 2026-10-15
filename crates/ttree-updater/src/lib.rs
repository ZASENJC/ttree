//! Shared stable-release updater. Only signed checksums authorize installation.
use anyhow::{ensure, Context, Result};
use base64::Engine;
use futures_util::StreamExt;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

const REPO: &str = "https://github.com/ZASENJC/ttree/releases/download";
const MAX_PACKAGE: u64 = 512 * 1024 * 1024;
const MAX_EXPANDED: u64 = 1536 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub exit_for_install: bool,
    pub status: String,
    pub button: &'static str,
    pub busy: bool,
}
impl Default for View {
    fn default() -> Self {
        Self {
            exit_for_install: false,
            status: format!("当前版本 {} · 正式版通道", env!("CARGO_PKG_VERSION")),
            button: "检查更新",
            busy: false,
        }
    }
}
#[derive(Clone, Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
}
struct Download {
    _dir: tempfile::TempDir,
    package: PathBuf,
}
#[derive(Default)]
struct State {
    view: View,
    release: Option<Release>,
    download: Option<Download>,
}
#[derive(Clone, Default)]
pub struct Updater(Arc<Mutex<State>>);
impl Updater {
    pub fn view(&self) -> View {
        self.0.lock().unwrap().view.clone()
    }
    /// Starts work without blocking the UI. Poll `view` to display progress and exit for installation.
    pub fn activate(&self, runtime: &tokio::runtime::Handle) {
        let mut s = self.0.lock().unwrap();
        if s.view.busy {
            return;
        }
        if s.download.is_some() {
            s.view.busy = true;
            s.view.status = "正在准备安装更新…".into();
            let this = self.clone();
            drop(s);
            runtime.spawn_blocking(move || {
                let (download, version) = {
                    let mut s = this.0.lock().unwrap();
                    (
                        s.download.take().unwrap(),
                        parse_tag(&s.release.as_ref().unwrap().tag_name)
                            .unwrap()
                            .to_string(),
                    )
                };
                let result = install(&download.package, &version);
                let mut s = this.0.lock().unwrap();
                s.view.busy = false;
                match result {
                    Ok(()) => {
                        s.view.exit_for_install = true;
                        s.view.busy = true;
                    }
                    Err(e) => {
                        s.download = Some(download);
                        s.view.status = format!("无法安装：{e:#}");
                    }
                }
            });
            return;
        }
        let release = s.release.clone();
        s.view.busy = true;
        s.view.status = if release.is_some() {
            "正在下载更新…"
        } else {
            "正在检查更新…"
        }
        .into();
        drop(s);
        let this = self.clone();
        runtime.spawn(async move {
            let result = async {
                let client = reqwest::Client::builder()
                    .user_agent("TTREE-Updater")
                    .https_only(true)
                    .connect_timeout(Duration::from_secs(15))
                    .timeout(Duration::from_secs(600))
                    .build()?;
                if let Some(release) = release {
                    let download = this.download(&client, &release).await?;
                    let mut s = this.0.lock().unwrap();
                    s.download = Some(download);
                    s.view.status = format!(
                        "{} 已下载并通过签名校验，安装将退出并重新打开 TTREE",
                        release.tag_name
                    );
                    s.view.button = "安装并重启";
                } else {
                    let response = client
                        .get("https://api.github.com/repos/ZASENJC/ttree/releases/latest")
                        .send()
                        .await?;
                    if response.status() == reqwest::StatusCode::NOT_FOUND {
                        this.0.lock().unwrap().view.status = "尚无可用的正式版".into();
                        return Ok::<_, anyhow::Error>(());
                    }
                    let data = fetch_response(response, 1024 * 1024).await?;
                    let release: Release = serde_json_decode(&data)?;
                    ensure!(
                        !release.draft && !release.prerelease,
                        "更新源没有返回正式版"
                    );
                    let version = parse_tag(&release.tag_name)?;
                    let mut s = this.0.lock().unwrap();
                    if version > semver::Version::parse(env!("CARGO_PKG_VERSION"))? {
                        s.view.status = format!("发现新版本 {}", release.tag_name);
                        s.view.button = "下载更新";
                        s.release = Some(release);
                    } else {
                        s.view.status = "当前已是最新正式版".into();
                    }
                }
                Ok(())
            }
            .await;
            let mut s = this.0.lock().unwrap();
            s.view.busy = false;
            if let Err(e) = result {
                s.view.status = format!("更新失败：{e:#}。请重试");
            }
        });
    }
    async fn download(&self, client: &reqwest::Client, release: &Release) -> Result<Download> {
        let version = parse_tag(&release.tag_name)?;
        let name = if cfg!(target_os = "windows") {
            format!("TTREE_{version}_windows_x64.zip")
        } else {
            "TTREE_universal.app.tar.gz".into()
        };
        let base = format!("{REPO}/{}", release.tag_name);
        let checksums = fetch(client, &format!("{base}/SHA256SUMS"), 64 * 1024).await?;
        let signature = fetch(client, &format!("{base}/SHA256SUMS.sig"), 16 * 1024)
            .await
            .context("此版本缺少安全更新签名，请手动安装或等待新版发布")?;
        verify_signature(&checksums, &signature)?;
        let hash = expected_hash(std::str::from_utf8(&checksums)?, &name)?;
        let dir = tempfile::Builder::new()
            .prefix("ttree-download-")
            .tempdir()?;
        let package = dir.path().join(&name);
        let mut file = std::fs::File::create(&package)?;
        let response = client
            .get(format!("{base}/{name}"))
            .send()
            .await?
            .error_for_status()?;
        let total = response.content_length().filter(|n| *n > 0);
        ensure!(total.unwrap_or(0) <= MAX_PACKAGE, "更新包过大");
        let mut stream = response.bytes_stream();
        let mut received = 0u64;
        let mut digest = Sha256::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            received += chunk.len() as u64;
            ensure!(received <= MAX_PACKAGE, "更新包过大");
            digest.update(&chunk);
            file.write_all(&chunk)?;
            let mut s = self.0.lock().unwrap();
            s.view.status = match total {
                Some(n) => format!("正在下载更新… {}%", received.saturating_mul(100) / n),
                None => format!("正在下载更新… {} MB", received / 1024 / 1024),
            };
        }
        file.sync_all()?;
        ensure!(
            received > 0 && total.is_none_or(|n| n == received),
            "更新包下载不完整"
        );
        ensure!(format!("{:x}", digest.finalize()) == hash, "更新包校验失败");
        Ok(Download { _dir: dir, package })
    }
}
fn serde_json_decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    Ok(serde_json::from_slice(bytes)?)
}
fn parse_tag(tag: &str) -> Result<semver::Version> {
    let version = semver::Version::parse(tag.strip_prefix('v').context("无效版本标签")?)?;
    ensure!(
        version.pre.is_empty() && version.build.is_empty(),
        "只支持正式版更新"
    );
    Ok(version)
}
async fn fetch(client: &reqwest::Client, url: &str, limit: u64) -> Result<Vec<u8>> {
    fetch_response(client.get(url).send().await?, limit).await
}
async fn fetch_response(response: reqwest::Response, limit: u64) -> Result<Vec<u8>> {
    let response = response.error_for_status()?;
    ensure!(
        response.content_length().unwrap_or(0) <= limit,
        "更新元数据过大"
    );
    let mut data = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        ensure!(
            data.len() as u64 + chunk.len() as u64 <= limit,
            "更新元数据过大"
        );
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}
fn verify_signature(data: &[u8], encoded: &[u8]) -> Result<()> {
    verify_with_key(
        data,
        encoded,
        include_str!("../../../scripts/release/updater.pub"),
    )
}
fn verify_with_key(data: &[u8], encoded: &[u8], public_key: &str) -> Result<()> {
    let signature =
        base64::engine::general_purpose::STANDARD.decode(std::str::from_utf8(encoded)?.trim())?;
    let key = minisign_verify::PublicKey::decode(public_key)?;
    let signature = minisign_verify::Signature::decode(std::str::from_utf8(&signature)?)?;
    key.verify(data, &signature, true).context("更新签名无效")
}
fn expected_hash(manifest: &str, name: &str) -> Result<String> {
    let entries: Vec<_> = manifest
        .lines()
        .filter_map(|l| l.split_once("  "))
        .filter(|(_, n)| *n == name)
        .collect();
    ensure!(entries.len() == 1, "更新清单缺少文件或存在重复项");
    let hash = entries[0].0;
    ensure!(
        hash.len() == 64
            && hash
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
        "无效校验值"
    );
    Ok(hash.into())
}

fn install(package: &Path, version: &str) -> Result<()> {
    let exe = std::env::current_exe()?;
    #[cfg(target_os = "macos")]
    return install_mac(package, &exe.canonicalize()?, version);
    #[cfg(target_os = "windows")]
    {
        let _ = version;
        return install_windows(package, &exe);
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (package, exe, version);
        anyhow::bail!("不支持的平台")
    }
}

#[cfg(target_os = "macos")]
fn install_mac(package: &Path, exe: &Path, version: &str) -> Result<()> {
    let app = exe
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .context("请从已安装的 TTREE.app 中更新")?;
    ensure!(
        app.extension().is_some_and(|e| e == "app") && app.join("Contents/MacOS/ttree") == exe,
        "请从已安装的 TTREE.app 中更新"
    );
    ensure!(
        !app.starts_with("/Volumes"),
        "请先将 TTREE 拖入应用程序文件夹，再安装更新"
    );
    let stage = tempfile::Builder::new()
        .prefix(".ttree-update-")
        .tempdir_in(app.parent().unwrap())
        .context("应用目录不可写，请将 TTREE 安装到可写的应用程序文件夹")?;
    let mut archive =
        tar::Archive::new(flate2::read::GzDecoder::new(std::fs::File::open(package)?));
    let mut total = 0u64;
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        ensure!(
            safe_path(&path) && path.starts_with("TTREE.app"),
            "更新归档路径无效"
        );
        let kind = entry.header().entry_type();
        ensure!(
            kind.is_file() || kind.is_dir(),
            "更新归档包含不支持的链接或设备"
        );
        total = total.checked_add(entry.size()).context("更新归档过大")?;
        ensure!(total <= MAX_EXPANDED, "解压后的更新包过大");
        ensure!(entry.unpack_in(stage.path())?, "更新归档路径无效");
    }
    let bundle_version = std::process::Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", "Print :CFBundleShortVersionString"])
        .arg(stage.path().join("TTREE.app/Contents/Info.plist"))
        .output()?;
    ensure!(
        bundle_version.status.success()
            && std::str::from_utf8(&bundle_version.stdout)?.trim() == version,
        "更新包版本与发布版本不一致"
    );
    let verifier = stage.path().join("verify.sh");
    std::fs::write(
        &verifier,
        include_str!("../../../scripts/macos/verify-signature.sh"),
    )?;
    ensure!(
        std::process::Command::new("/bin/sh")
            .arg(&verifier)
            .arg(stage.path().join("TTREE.app"))
            .status()?
            .success(),
        "更新应用的 Mac 签名校验失败"
    );
    let helper = stage.path().join("install.sh");
    std::fs::write(&helper, include_str!("install-macos.sh"))?;
    use std::os::unix::process::CommandExt;
    let mut command = std::process::Command::new("/bin/sh");
    command
        .arg(&helper)
        .arg(std::process::id().to_string())
        .arg(app)
        .arg(stage.path())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    // The helper must outlive the GUI or command that launched it. Only an async-signal-safe
    // syscall runs between fork and exec; do not inherit the parent's terminal or process group.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command.spawn()?;
    let _ = stage.keep();
    Ok(())
}
#[cfg(target_os = "windows")]
fn install_windows(package: &Path, exe: &Path) -> Result<()> {
    use std::os::windows::process::CommandExt;
    let stage = tempfile::Builder::new()
        .prefix(".ttree-update-")
        .tempdir_in(exe.parent().unwrap())
        .context("程序目录不可写，请将 TTREE 移至可写目录")?;
    extract_windows(package, stage.path())?;
    let script = include_str!("install-windows.ps1")
        .replace("__PID__", &std::process::id().to_string())
        .replace("__DEST__", &ps_literal(exe))
        .replace("__STAGE__", &ps_literal(stage.path()));
    let helper = stage.path().join("install.ps1");
    // BOM preserves non-ASCII paths in Windows PowerShell 5.1.
    std::fs::write(&helper, format!("\u{feff}{script}"))?;
    std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&helper)
        .creation_flags(0x08000000)
        .spawn()?;
    let _ = stage.keep();
    Ok(())
}
#[cfg(target_os = "windows")]
fn ps_literal(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "''"))
}
fn safe_path(path: &Path) -> bool {
    path.components()
        .all(|c| matches!(c, std::path::Component::Normal(_)))
        && !path.as_os_str().is_empty()
        && !path.to_string_lossy().contains(['\\', ':'])
}
#[cfg(any(target_os = "windows", test))]
fn extract_windows(package: &Path, stage: &Path) -> Result<()> {
    use std::io::Read;
    let mut zip = zip::ZipArchive::new(std::fs::File::open(package)?)?;
    let mut found = false;
    let mut total = 0u64;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let path = PathBuf::from(entry.name());
        ensure!(
            safe_path(&path) && path.starts_with("TTREE"),
            "ZIP 路径无效"
        );
        ensure!(!entry.is_symlink(), "ZIP 不支持符号链接");
        total = total.checked_add(entry.size()).context("ZIP 过大")?;
        ensure!(total <= MAX_EXPANDED, "解压后的更新包过大");
        // Only the executable changes; local files and licenses stay in place.
        if entry.name() == "TTREE/TTREE.exe" {
            ensure!(!found, "ZIP 内存在重复程序");
            found = true;
            let mut file = std::fs::File::create(stage.join("TTREE.exe"))?;
            let mut magic = [0; 2];
            entry.read_exact(&mut magic)?;
            ensure!(&magic == b"MZ", "ZIP 不包含有效的 Windows 程序");
            file.write_all(&magic)?;
            let copied = std::io::copy(&mut entry.by_ref().take(MAX_EXPANDED + 1), &mut file)?;
            ensure!(copied <= MAX_EXPANDED, "程序过大");
            file.sync_all()?;
        }
    }
    ensure!(found, "ZIP 缺少 TTREE.exe");
    Ok(())
}

/// Offline installation probe, compiled only into opt-in verification builds.
/// It verifies the same pinned signature and checksum before invoking the production installer.
#[cfg(feature = "smoke")]
pub fn smoke_from_args() -> Option<Result<()>> {
    let args: Vec<_> = std::env::args_os().collect();
    if args
        .get(1)
        .is_none_or(|arg| arg != "--updater-smoke-install")
    {
        return None;
    }
    Some((|| {
        ensure!(
            args.len() == 4,
            "usage: --updater-smoke-install SIGNED_RELEASE_DIR vX.Y.Z"
        );
        let directory = Path::new(&args[2]);
        let tag = args[3].to_str().context("invalid release tag")?;
        let version = parse_tag(tag)?;
        let name = if cfg!(target_os = "windows") {
            format!("TTREE_{version}_windows_x64.zip")
        } else {
            "TTREE_universal.app.tar.gz".into()
        };
        let manifest = std::fs::read(directory.join("SHA256SUMS"))?;
        verify_signature(&manifest, &std::fs::read(directory.join("SHA256SUMS.sig"))?)?;
        let expected = expected_hash(std::str::from_utf8(&manifest)?, &name)?;
        let package = directory.join(name);
        let mut file = std::fs::File::open(&package)?;
        let mut digest = Sha256::new();
        std::io::copy(&mut file, &mut digest)?;
        ensure!(
            format!("{:x}", digest.finalize()) == expected,
            "更新包校验失败"
        );
        install(&package, &version.to_string())
    })())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stable_versions_are_numeric_and_do_not_accept_untrusted_tags() {
        assert!(parse_tag("v0.10.0").unwrap() > parse_tag("v0.9.0").unwrap());
        for tag in [
            "0.3.0",
            "v0.3.0-dev.12.1",
            "v0.3.0+build",
            "v1/../../evil",
            "latest",
        ] {
            assert!(parse_tag(tag).is_err(), "{tag}");
        }
    }
    #[test]
    fn checksum_requires_one_exact_filename_and_valid_hash() {
        let h = "a".repeat(64);
        let manifest = format!("{h}  app.zip\n{h}  other.zip\n");
        assert_eq!(expected_hash(&manifest, "app.zip").unwrap(), h);
        assert!(expected_hash(&manifest, "missing.zip").is_err());
        assert!(expected_hash(&(manifest.clone() + &manifest), "app.zip").is_err());
        assert!(expected_hash("bad  app.zip", "app.zip").is_err());
        assert!(expected_hash(&format!("{}  app.zip", "A".repeat(64)), "app.zip").is_err());
    }
    #[test]
    fn invalid_and_foreign_signatures_never_authorize_an_update() {
        assert!(verify_signature(b"package", b"garbage").is_err());
        // Public test vector from minisign-verify, unrelated to the release key.
        let signature = "untrusted comment: signature from minisign secret key\nRWQf6LRCGA9i59SLOFxz6NxvASXDJeRtuZykwQepbDEGt87ig1BNpWaVWuNrm73YiIiJbq71Wi+dP9eKL8OC351vwIasSSbXxwA=\ntrusted comment: timestamp:1555779966\tfile:test\nQtKMXWyYcwdpZAlPF7tE2ENJkRd1ujvKjlj1m9RtHTBnZPa5WKU5uWRs5GoP5M/VqE81QFuMKI5k/SfNQUaOAA==";
        let encoded = base64::engine::general_purpose::STANDARD.encode(signature);
        let test_key =
            "untrusted comment: test key\nRWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";
        assert!(verify_with_key(b"test", encoded.as_bytes(), test_key).is_ok());
        assert!(verify_with_key(b"tampered", encoded.as_bytes(), test_key).is_err());
        assert!(verify_signature(b"test", encoded.as_bytes()).is_err());
        assert!(verify_signature(b"tampered", encoded.as_bytes()).is_err());
    }
    #[test]
    fn paths_cannot_escape_or_target_windows_drives() {
        for path in [
            "../TTREE/TTREE.exe",
            "/tmp/TTREE.exe",
            "C:/TTREE.exe",
            "TTREE/../../evil",
            "TTREE\\evil",
            "",
        ] {
            assert!(!safe_path(Path::new(path)), "{path}");
        }
        assert!(safe_path(Path::new("TTREE/TTREE.exe")));
    }
    fn fixture_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, content) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(content).unwrap();
        }
        zip.finish().unwrap();
    }
    #[test]
    fn portable_extraction_requires_an_executable_and_preserves_other_files() {
        let dir = tempfile::tempdir().unwrap();
        let zip = dir.path().join("update.zip");
        std::fs::write(dir.path().join("config.json"), b"untouched").unwrap();
        fixture_zip(
            &zip,
            &[
                ("TTREE/TTREE.exe", b"MZfixture"),
                ("TTREE/README.md", b"readme"),
            ],
        );
        extract_windows(&zip, dir.path()).unwrap();
        assert_eq!(
            std::fs::read(dir.path().join("TTREE.exe")).unwrap(),
            b"MZfixture"
        );
        assert_eq!(
            std::fs::read(dir.path().join("config.json")).unwrap(),
            b"untouched"
        );
        assert!(!dir.path().join("README.md").exists());
        for entries in [
            vec![("TTREE/TTREE.exe", &b"invalid"[..])],
            vec![("TTREE/readme", &b"MZ"[..])],
            vec![("../evil", &b"MZ"[..])],
        ] {
            fixture_zip(&zip, &entries);
            assert!(extract_windows(&zip, dir.path()).is_err());
        }
    }
    #[tokio::test]
    async fn busy_clicks_do_not_spawn_work() {
        let updater = Updater::default();
        {
            let mut s = updater.0.lock().unwrap();
            s.view.busy = true;
        }
        updater.activate(&tokio::runtime::Handle::current());
        assert!(updater.view().busy);
        assert!(updater.0.lock().unwrap().release.is_none());
    }
    #[tokio::test]
    async fn metadata_limit_applies_to_chunked_bodies() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let _ = stream.read(&mut request).await.unwrap();
            stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n6\r\nabcdef\r\n0\r\n\r\n").await.unwrap();
        });
        let response = reqwest::get(format!("http://{address}")).await.unwrap();
        assert!(fetch_response(response, 5).await.is_err());
        server.await.unwrap();
    }
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_helper_installs_and_rolls_back_when_replace_fails() {
        for fail in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let destination = root.path().join("TTREE user's app.exe");
            let stage = root.path().join("stage");
            std::fs::create_dir(&stage).unwrap();
            std::fs::write(&destination, "old").unwrap();
            if !fail {
                std::fs::write(stage.join("TTREE.exe"), "new").unwrap();
            }
            let script = include_str!("install-windows.ps1")
                .replace("__PID__", "99999999")
                .replace("__DEST__", &ps_literal(&destination))
                .replace("__STAGE__", &ps_literal(&stage))
                .replace(
                    "Start-Process -FilePath $Destination -ArgumentList '--show'",
                    "$null = 1",
                )
                .replace("    Add-Type -AssemblyName System.Windows.Forms", "")
                .replace("[System.Windows.Forms.MessageBox]::Show(\"Update failed. Previous version restored where possible.`n$Failure\", 'TTREE') | Out-Null", "Write-Output $Failure");
            let helper = root.path().join("install.ps1");
            std::fs::write(&helper, format!("\u{feff}{script}")).unwrap();
            let status = std::process::Command::new("powershell.exe")
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                ])
                .arg(helper)
                .status()
                .unwrap();
            assert_eq!(status.success(), !fail);
            assert_eq!(
                std::fs::read_to_string(&destination).unwrap(),
                if fail { "old" } else { "new" }
            );
        }
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn mac_helper_installs_and_rolls_back_on_verification_failure() {
        for fail in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let destination = root.path().join("App with spaces.app");
            let stage = root.path().join("stage");
            std::fs::create_dir(&destination).unwrap();
            std::fs::create_dir(&stage).unwrap();
            std::fs::create_dir(stage.join("TTREE.app")).unwrap();
            std::fs::write(destination.join("version"), "old").unwrap();
            std::fs::write(stage.join("TTREE.app/version"), "new").unwrap();
            let verifier = if fail {
                "#!/bin/sh\ncase \"$1\" in */TTREE.app) exit 0;; *) exit 1;; esac\n"
            } else {
                "#!/bin/sh\nexit 0\n"
            };
            std::fs::write(stage.join("verify.sh"), verifier).unwrap();
            // Suppress launching UI; run the actual move/rollback logic in a sandbox.
            let script = include_str!("install-macos.sh").replace("/usr/bin/open", "/usr/bin/true");
            let helper = root.path().join("install.sh");
            std::fs::write(&helper, script).unwrap();
            let status = std::process::Command::new("/bin/sh")
                .arg(helper)
                .args(["99999999"])
                .arg(&destination)
                .arg(&stage)
                .status()
                .unwrap();
            assert_eq!(status.success(), !fail);
            assert_eq!(
                std::fs::read_to_string(destination.join("version")).unwrap(),
                if fail { "old" } else { "new" }
            );
            assert_eq!(stage.exists(), fail);
        }
    }
}
