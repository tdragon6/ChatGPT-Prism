#[cfg(target_os = "macos")]
use std::fs;
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
#[cfg(windows)]
use std::{
    io::{Seek, SeekFrom},
    process::Output,
};

#[derive(Clone, Debug)]
pub struct Client {
    pub executable: PathBuf,
    pub path: PathBuf,
    pub name: String,
    pub has_client_theme: bool,
}

#[cfg(windows)]
pub fn quiet(command: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x08000000)
}

#[cfg(not(windows))]
pub fn quiet(command: &mut Command) -> &mut Command {
    command
}

#[cfg(target_os = "macos")]
pub fn discover(configured: &str) -> Result<Client, String> {
    let candidates = if configured.is_empty() {
        let mut paths = vec![
            PathBuf::from("/Applications/ChatGPT.app"),
            PathBuf::from("/Applications/Codex.app"),
        ];
        if let Some(home) = std::env::home_dir() {
            paths.extend(
                ["ChatGPT.app", "Codex.app"].map(|name| home.join("Applications").join(name)),
            );
        }
        paths
    } else {
        vec![PathBuf::from(configured)]
    };
    candidates
        .iter()
        .find_map(|path| verify_mac(path))
        .ok_or_else(|| "ChatGPT application not found or could not be verified".into())
}

#[cfg(target_os = "macos")]
fn verify_mac(path: &Path) -> Option<Client> {
    let bundle = fs::canonicalize(path).ok()?;
    if bundle.extension()? != "app" {
        return None;
    }
    let plist = bundle.join("Contents/Info.plist");
    let plist_value = |key| {
        let output = Command::new("/usr/bin/plutil")
            .args(["-extract", key, "raw", "-o", "-"])
            .arg(&plist)
            .output()
            .ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
    };
    let identifier = plist_value("CFBundleIdentifier")?;
    if !["com.openai.chatgpt", "com.openai.chat", "com.openai.codex"]
        .iter()
        .any(|prefix| {
            identifier == *prefix
                || identifier
                    .strip_prefix(prefix)
                    .is_some_and(|rest| rest.starts_with(['.', '-']))
        })
    {
        return None;
    }
    let executable_name = plist_value("CFBundleExecutable")?;
    if executable_name.contains(['/', '\\']) || executable_name.starts_with('.') {
        return None;
    }
    let executable = fs::canonicalize(bundle.join("Contents/MacOS").join(executable_name)).ok()?;
    if !executable.starts_with(&bundle) || !executable.is_file() {
        return None;
    }
    let trusted = Command::new("/usr/bin/codesign")
        .args([
            "--verify",
            "--strict",
            "-R=anchor apple generic and certificate leaf[subject.OU] = \"2DC432GLL2\"",
        ])
        .arg(&bundle)
        .output()
        .ok()?;
    if !trusted.status.success() {
        return None;
    }
    Some(Client {
        executable,
        name: bundle.file_stem()?.to_string_lossy().into_owned(),
        has_client_theme: identifier == "com.openai.codex",
        path: bundle,
    })
}

#[cfg(windows)]
const POWERSHELL_DISCOVERY: &str = r#"
$ErrorActionPreference = 'SilentlyContinue'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$paths = @()
function AddDirectory([string] $folder) {
    if (-not $folder) { return }
    foreach ($relative in @('ChatGPT.exe', 'app\ChatGPT.exe', 'Codex.exe', 'app\Codex.exe')) {
        $script:paths += Join-Path $folder $relative
    }
}
if ($env:PRISM_SELECTED_CLIENT) {
    if (Test-Path -LiteralPath $env:PRISM_SELECTED_CLIENT -PathType Container) {
        AddDirectory $env:PRISM_SELECTED_CLIENT
    } else { $paths += $env:PRISM_SELECTED_CLIENT }
} else {
    $paths += Get-CimInstance Win32_Process -Filter "Name='ChatGPT.exe' OR Name='Codex.exe'" |
        Where-Object { $_.ExecutablePath -and $_.CommandLine -notmatch '--type=' } |
        Select-Object -ExpandProperty ExecutablePath
    Get-AppxPackage -Name 'OpenAI.*' | ForEach-Object { AddDirectory $_.InstallLocation }
    foreach ($relative in @('Programs\ChatGPT','ChatGPT','Programs\Codex','Codex')) {
        AddDirectory (Join-Path $env:LOCALAPPDATA $relative)
    }
}
$results = @(
    $paths | Where-Object { $_ } | Select-Object -Unique | ForEach-Object {
        $file = Get-Item -LiteralPath $_ -ErrorAction SilentlyContinue
        if ($file.Name -in @('ChatGPT.exe','Codex.exe') -and -not $file.PSIsContainer) {
            $sign = Get-AuthenticodeSignature -LiteralPath $file.FullName
            if ($sign.Status -eq 'Valid' -and $sign.SignerCertificate.Subject -match '\bOpenAI\b') {
                [PSCustomObject]@{ path = $file.FullName; name = $file.BaseName; product = $file.VersionInfo.ProductName }
            }
        }
    }
)
ConvertTo-Json -InputObject $results -Compress
"#;

#[cfg(windows)]
pub fn discover(configured: &str) -> Result<Client, String> {
    use serde::Deserialize;
    #[derive(Deserialize)]
    struct Candidate {
        path: PathBuf,
        name: String,
        product: Option<String>,
    }
    if !configured.is_empty() && !Path::new(configured).is_absolute() {
        return Err("Choose an absolute application path".into());
    }
    let powershell =
        PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into()))
            .join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    let mut command = Command::new(powershell);
    quiet(&mut command)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            POWERSHELL_DISCOVERY,
        ])
        .env("PRISM_SELECTED_CLIENT", configured);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().eq_ignore_ascii_case("PSModulePath") {
            command.env_remove(key);
        }
    }
    let output = bounded_output(command)?;
    let bytes = output
        .stdout
        .strip_prefix(&[0xef, 0xbb, 0xbf])
        .unwrap_or(&output.stdout);
    let candidates: Vec<Candidate> = serde_json::from_slice(bytes).unwrap_or_default();
    for candidate in candidates {
        if !is_windows_gui(&candidate.path) {
            continue;
        }
        let path = candidate.path;
        return Ok(Client {
            executable: path.clone(),
            has_client_theme: candidate
                .product
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case("Codex")),
            name: candidate.name,
            path,
        });
    }
    Err("ChatGPT application not found or could not be verified".into())
}

#[cfg(windows)]
fn is_windows_gui(path: &Path) -> bool {
    fn read(path: &Path) -> std::io::Result<bool> {
        let mut file = File::open(path)?;
        let mut dos = [0; 64];
        file.read_exact(&mut dos)?;
        let offset = u32::from_le_bytes(dos[60..64].try_into().unwrap());
        if dos[..2] != *b"MZ" || !(64..=16 * 1024 * 1024).contains(&offset) {
            return Ok(false);
        }
        file.seek(SeekFrom::Start(offset.into()))?;
        let mut pe = [0; 94];
        file.read_exact(&mut pe)?;
        let flags = u16::from_le_bytes([pe[22], pe[23]]);
        Ok(pe[..4] == *b"PE\0\0"
            && flags & 0x2002 == 0x2
            && matches!(u16::from_le_bytes([pe[24], pe[25]]), 0x10b | 0x20b)
            && u16::from_le_bytes([pe[92], pe[93]]) == 2)
    }
    read(path).unwrap_or(false)
}

#[cfg(windows)]
fn bounded_output(mut command: Command) -> Result<Output, String> {
    use std::{
        thread,
        time::{Duration, Instant},
    };
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| error.to_string())?;
    let mut stdout = child.stdout.take().unwrap().take(128 * 1024);
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes)?;
        Ok::<_, std::io::Error>(bytes)
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            let stdout = reader
                .join()
                .map_err(|_| "Application discovery failed")?
                .map_err(|error| error.to_string())?;
            return Ok(Output {
                status,
                stdout,
                stderr: Vec::new(),
            });
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Application discovery timed out".into());
        }
        thread::sleep(Duration::from_millis(50));
    }
}

pub fn launch(client: &Client) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("/usr/bin/open");
        command.arg("-a").arg(&client.path);
        command
    };
    #[cfg(windows)]
    let mut command = Command::new(&client.executable);
    command
        .env_remove("ELECTRON_RUN_AS_NODE")
        .env_remove("NODE_OPTIONS")
        .env_remove("NODE_PATH");
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(target_os = "macos")]
    if !command
        .status()
        .map_err(|error| error.to_string())?
        .success()
    {
        return Err("Could not open the selected application".into());
    }
    #[cfg(windows)]
    command.spawn().map_err(|error| error.to_string())?;
    Ok(())
}

fn read_limited(path: &Path, limit: u64) -> Option<String> {
    let file = File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > limit {
        return None;
    }
    let mut text = String::new();
    file.take(limit + 1).read_to_string(&mut text).ok()?;
    (text.len() as u64 <= limit).then_some(text)
}

pub fn client_theme(client: Option<&Client>, system_dark: bool) -> (&'static str, &'static str) {
    let fallback = if system_dark { "dark" } else { "light" };
    if !client.is_some_and(|client| client.has_client_theme) {
        return (fallback, "system-fallback");
    }
    let config_dir = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::home_dir().map(|p| p.join(".codex")));
    let Some(config_dir) = config_dir else {
        return (fallback, "system-fallback");
    };
    let mut preference = None;
    if let Some(text) = read_limited(&config_dir.join("config.toml"), 2 * 1024 * 1024) {
        let Ok(config) = text.trim_start_matches('\u{feff}').parse::<toml::Table>() else {
            return (fallback, "system-fallback");
        };
        if let Some(value) = config
            .get("desktop")
            .and_then(|table| table.get("appearanceTheme"))
        {
            preference = Some(value.as_str().unwrap_or_default().to_owned());
        }
    }
    if preference.is_none()
        && let Some(text) = read_limited(
            &config_dir.join(".codex-global-state.json"),
            8 * 1024 * 1024,
        )
        && let Ok(config) =
            serde_json::from_str::<serde_json::Value>(text.trim_start_matches('\u{feff}'))
    {
        preference = config
            .get("appearanceTheme")
            .map(|value| value.as_str().unwrap_or_default().to_owned());
    }
    match preference.as_deref() {
        Some("light") => ("light", "client-preference"),
        Some("dark") => ("dark", "client-preference"),
        None | Some("system") => (fallback, "client-system"),
        _ => (fallback, "system-fallback"),
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn launch_reports_a_failed_open_request() {
        let folder = tempfile::tempdir().unwrap();
        let missing = folder.path().join("Missing Application.app");
        let client = Client {
            executable: missing.join("Contents/MacOS/Missing"),
            path: missing,
            name: "Missing Application".into(),
            has_client_theme: false,
        };

        assert!(launch(&client).is_err());
    }
}
