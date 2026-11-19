//! Android SDK facts, read from the official Android CLI (`android sdk`).
//!
//! The CLI is the single source of truth: listing comes from
//! `android sdk list`, and mutations shell out to `android sdk install`,
//! `android sdk update`, and `android sdk remove`.

use std::{
    collections::HashSet,
    env,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, Mutex, MutexGuard},
    thread,
    time::{Duration, Instant},
};

/// An installed SDK platform, e.g. `platforms/android-36`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Platform {
    /// Package id accepted by `android sdk install|update|remove`.
    pub package: String,
    /// API level derived from the package id (`android-36` -> `36`).
    pub api_level: String,
    /// Version reported by the CLI.
    pub version: String,
    /// Human description reported by the CLI.
    pub description: String,
    /// Whether the package is installed or only available.
    pub installed: bool,
}

impl Platform {
    /// Case-insensitive match against the fields the search bar exposes.
    /// `query` is expected to be lowercased by the caller.
    pub fn matches(&self, query: &str) -> bool {
        query.is_empty()
            || self.package.to_lowercase().contains(query)
            || self.api_level.to_lowercase().contains(query)
            || self.version.to_lowercase().contains(query)
            || self.description.to_lowercase().contains(query)
    }
}

/// An SDK package keyed by version rather than API level, e.g.
/// `build-tools/36.0.0`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildTool {
    /// Package id accepted by `android sdk install|update|remove`.
    pub package: String,
    /// Version reported by the CLI.
    pub version: String,
    /// Human description reported by the CLI.
    pub description: String,
    /// Whether the package is installed or only available.
    pub installed: bool,
}

impl BuildTool {
    /// Case-insensitive match against the fields the search bar exposes.
    /// `query` is expected to be lowercased by the caller.
    pub fn matches(&self, query: &str) -> bool {
        query.is_empty()
            || self.package.to_lowercase().contains(query)
            || self.version.to_lowercase().contains(query)
            || self.description.to_lowercase().contains(query)
    }
}

/// A device reported by `adb devices -l`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    /// `adb` serial, e.g. `emulator-5554` or a physical device serial.
    pub serial: String,
    /// Raw adb state: `device`, `unauthorized`, `offline`, etc.
    pub state: String,
    pub model: Option<String>,
    pub product: Option<String>,
    pub device: Option<String>,
    pub transport_id: Option<String>,
    pub usb: Option<String>,
    pub is_emulator: bool,
}

impl Device {
    pub fn is_online(&self) -> bool {
        self.state == "device"
    }
}

/// A configured Android Virtual Device from `android emulator list --long`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Emulator {
    pub id: String,
    pub name: String,
    pub api_level: String,
    pub status: String,
    pub serial: Option<String>,
}

impl Emulator {
    pub fn is_online(&self) -> bool {
        self.status.eq_ignore_ascii_case("online") || self.status.eq_ignore_ascii_case("running")
    }
}

/// A point-in-time view of the SDK that the UI renders.
#[derive(Clone, Debug, Default)]
pub struct SdkSnapshot {
    /// Path to the `android` binary, when one is on `PATH`.
    pub cli: Option<PathBuf>,
    pub cli_version: Option<String>,
    pub root: Option<PathBuf>,
    pub platforms: Vec<Platform>,
    pub build_tools: Vec<BuildTool>,
    pub devices: Vec<Device>,
    pub emulators: Vec<Emulator>,
    /// Path to the `adb` binary used for device discovery.
    pub adb: Option<PathBuf>,
    /// Non-fatal CLI, adb, or parse failure.
    pub error: Option<String>,
    pub devices_error: Option<String>,
}

/// Tracks the running `android` child so shutdown can stop it cleanly.
#[derive(Clone, Default)]
pub struct RunningCli {
    inner: Arc<Mutex<ProcessState>>,
}

#[derive(Default)]
struct ProcessState {
    pid: Option<u32>,
    running: bool,
    /// Set once shutdown begins; new commands are refused after this point.
    cancelled: bool,
}

impl RunningCli {
    fn state(&self) -> MutexGuard<'_, ProcessState> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn is_running(&self) -> bool {
        self.state().running
    }

    /// Ask the running CLI to stop, then force it if it ignores the grace period.
    /// Called from the app's quit handler.
    pub fn shutdown(&self, grace: Duration) {
        let pid = {
            let mut state = self.state();
            state.cancelled = true;
            state.pid
        };
        let Some(pid) = pid else {
            return;
        };

        signal_process(pid, false);
        let deadline = Instant::now() + grace;
        while self.is_running() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        if self.is_running() {
            signal_process(pid, true);
        }
    }

    fn finish(&self) {
        let mut state = self.state();
        state.pid = None;
        state.running = false;
    }

    #[cfg(test)]
    fn track_for_test(&self, pid: u32) {
        let mut state = self.state();
        state.pid = Some(pid);
        state.running = true;
    }
}

#[cfg(unix)]
fn signal_process(pid: u32, force: bool) {
    let signal = if force { libc::SIGKILL } else { libc::SIGTERM };
    // SAFETY: `pid` is the child we spawned; a stale pid is a no-op error.
    unsafe {
        libc::kill(pid as libc::pid_t, signal);
    }
}

#[cfg(not(unix))]
fn signal_process(pid: u32, force: bool) {
    let mut command = Command::new("taskkill");
    command.args(["/PID", &pid.to_string(), "/T"]);
    if force {
        command.arg("/F");
    }
    let _ = command.status();
}

/// Executable name of the official Android CLI on this platform.
pub fn cli_binary_name() -> &'static str {
    if cfg!(windows) {
        "android.exe"
    } else {
        "android"
    }
}

/// Find the official Android CLI on `PATH` or in a standard install location.
///
/// macOS apps launched from Finder inherit launchd's minimal `PATH`, not the
/// login shell's. Homebrew installs the CLI at `/opt/homebrew/bin` (Apple
/// Silicon) or `/usr/local/bin` (Intel), so check those too.
pub fn find_cli() -> Option<PathBuf> {
    let name = cli_binary_name();
    search_paths()
        .into_iter()
        .map(|dir| dir.join(name))
        .find(|path| is_executable(path))
}

fn search_paths() -> Vec<PathBuf> {
    search_paths_from(env::var_os("PATH").as_deref())
}

fn search_paths_from(path_var: Option<&std::ffi::OsStr>) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = path_var
        .map(env::split_paths)
        .into_iter()
        .flatten()
        .filter(|dir| !dir.as_os_str().is_empty())
        .collect();

    #[cfg(target_os = "macos")]
    let extra = ["/opt/homebrew/bin", "/usr/local/bin"];
    #[cfg(not(target_os = "macos"))]
    let extra = ["/usr/local/bin"];

    for dir in extra {
        let dir = PathBuf::from(dir);
        if !paths.contains(&dir) {
            paths.push(dir);
        }
    }

    if let Some(home) = env::var_os("HOME") {
        let dir = PathBuf::from(home).join(".local/bin");
        if !paths.contains(&dir) {
            paths.push(dir);
        }
    }

    paths
}

/// PATH for child `android` processes. Android tooling invokes helpers such as
/// `java` and `adb`, so the GUI must hand them the same expanded PATH.
fn child_path() -> std::ffi::OsString {
    env::join_paths(search_paths()).unwrap_or_else(|_| env::var_os("PATH").unwrap_or_default())
}

#[cfg(unix)]
fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &std::path::Path) -> bool {
    path.is_file()
}

fn adb_binary_name() -> &'static str {
    if cfg!(windows) { "adb.exe" } else { "adb" }
}

fn find_adb(root: Option<&std::path::Path>) -> Option<PathBuf> {
    let name = adb_binary_name();
    let mut sdk_roots: Vec<PathBuf> = Vec::new();
    if let Some(root) = root {
        sdk_roots.push(root.to_path_buf());
    }
    for var in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(root) = env::var_os(var) {
            sdk_roots.push(PathBuf::from(root));
        }
    }

    for sdk_root in sdk_roots {
        let adb = sdk_root.join("platform-tools").join(name);
        if is_executable(&adb) {
            return Some(adb);
        }
    }

    search_paths()
        .into_iter()
        .map(|dir| dir.join(name))
        .find(|path| is_executable(path))
}

impl SdkSnapshot {
    /// Refresh configured AVDs and attached devices from the official tooling.
    pub fn refresh_devices(&mut self) {
        self.adb = None;
        self.devices.clear();
        self.emulators.clear();
        self.devices_error = None;
        let mut errors = Vec::new();

        if let Some(cli) = self.cli.clone() {
            let running = RunningCli::default();
            match run(
                &cli,
                self.root.as_deref(),
                &["emulator", "list", "--long"],
                &running,
            ) {
                Ok(output) => self.emulators = parse_emulators(&output),
                Err(err) => errors.push(err),
            }
        }

        let Some(adb) = find_adb(self.root.as_deref()) else {
            errors.push(format!(
                "`{}` was not found. Install Android SDK Platform-Tools with `android sdk install platform-tools`.",
                adb_binary_name()
            ));
            self.devices_error = Some(errors.join(" "));
            return;
        };

        self.adb = Some(adb.clone());
        let running = RunningCli::default();
        match run(&adb, None, &["devices", "-l"], &running) {
            Ok(output) => self.devices = parse_devices(&output),
            Err(err) => errors.push(err),
        }

        if !errors.is_empty() {
            self.devices_error = Some(errors.join(" "));
        }
    }
}

/// Parse `adb devices -l`:
///
/// ```text
/// List of devices attached
/// emulator-5554 device product:sdk_gphone64_arm64 model:sdk_gphone64_arm64 device:emu64a transport_id:1
/// R58M12345     unauthorized usb:1-1 transport_id:2
/// ```
fn parse_devices(output: &str) -> Vec<Device> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty()
                || line.starts_with("List of devices attached")
                || line.starts_with('*')
            {
                return None;
            }

            let mut parts = line.split_whitespace();
            let serial = parts.next()?.to_string();
            let state = parts.next()?.to_string();
            let mut model = None;
            let mut product = None;
            let mut device = None;
            let mut transport_id = None;
            let mut usb = None;

            for field in parts {
                let Some((key, value)) = field.split_once(':') else {
                    continue;
                };
                match key {
                    "model" => model = Some(value.to_string()),
                    "product" => product = Some(value.to_string()),
                    "device" => device = Some(value.to_string()),
                    "transport_id" => transport_id = Some(value.to_string()),
                    "usb" => usb = Some(value.to_string()),
                    _ => {}
                }
            }

            let is_emulator = serial.starts_with("emulator-");
            Some(Device {
                serial,
                state,
                model,
                product,
                device,
                transport_id,
                usb,
                is_emulator,
            })
        })
        .collect()
}

/// Parse the fixed-width table from `android emulator list --long`:
///
/// ```text
/// AVD ID                   AVD Name                      API Level      Status         Serial
/// Medium_Phone_API_37.0    Medium Phone API 37.0         android-37.0   Offline
/// Pixel_8                  Pixel 8                       android-35     Online         emulator-5554
/// ```
fn parse_emulators(output: &str) -> Vec<Emulator> {
    let mut lines = output.lines();
    let Some(header) = lines.find(|line| line.contains("AVD ID") && line.contains("AVD Name"))
    else {
        return Vec::new();
    };

    let Some(id_start) = header.find("AVD ID") else {
        return Vec::new();
    };
    let Some(name_start) = header.find("AVD Name") else {
        return Vec::new();
    };
    let Some(api_start) = header.find("API Level") else {
        return Vec::new();
    };
    let Some(status_start) = header.find("Status") else {
        return Vec::new();
    };
    let Some(serial_start) = header.find("Serial") else {
        return Vec::new();
    };

    if !(id_start < name_start && name_start < api_start && api_start < status_start)
        || status_start >= serial_start
    {
        return Vec::new();
    }

    let field = |line: &str, start: usize, end: Option<usize>| {
        let start = start.min(line.len());
        let end = end.unwrap_or(line.len()).min(line.len());
        line.get(start..end).unwrap_or_default().trim().to_string()
    };

    lines
        .filter_map(|line| {
            if line.trim().is_empty() {
                return None;
            }
            let id = field(line, id_start, Some(name_start));
            if id.is_empty() {
                return None;
            }
            let name = field(line, name_start, Some(api_start));
            let api_level = field(line, api_start, Some(status_start));
            let status = field(line, status_start, Some(serial_start));
            let serial = field(line, serial_start, None);
            Some(Emulator {
                id,
                name,
                api_level,
                status,
                serial: (!serial.is_empty()).then_some(serial),
            })
        })
        .collect()
}

pub fn snapshot() -> SdkSnapshot {
    let Some(cli) = find_cli() else {
        return SdkSnapshot {
            error: Some(format!(
                "`{}` is not on PATH. Install the official Android CLI from https://developer.android.com/tools/agents/android-cli",
                cli_binary_name()
            )),
            ..SdkSnapshot::default()
        };
    };

    let running = RunningCli::default();
    let (root, cli_version, info_error) = match run(&cli, None, &["info"], &running) {
        Ok(output) => {
            let (root, version) = parse_info(&output);
            (root, version, None)
        }
        Err(err) => (None, None, Some(err)),
    };
    let mut snapshot = SdkSnapshot {
        cli: Some(cli.clone()),
        cli_version,
        root: root.clone(),
        error: info_error,
        ..SdkSnapshot::default()
    };

    match run(&cli, root.as_deref(), &["sdk", "list", "--all"], &running) {
        Ok(output) => {
            snapshot.platforms = parse_platforms(&output);
            snapshot.build_tools = parse_build_tools(&output);
        }
        Err(err) => snapshot.error = Some(err),
    }
    snapshot.refresh_devices();
    snapshot
}

/// Parse `android info` output:
///
/// ```text
/// sdk: /Users/me/Library/Android/sdk
/// version: 1.1.16919734
/// launcher_version: 1.1.16919734
/// ```
fn parse_info(output: &str) -> (Option<PathBuf>, Option<String>) {
    let mut root = None;
    let mut version = None;
    for line in output.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        match key.trim() {
            "sdk" => root = Some(PathBuf::from(value)),
            "version" => version = Some(value.to_string()),
            _ => {}
        }
    }
    (root, version)
}

/// One row of `android sdk list --all`, before filtering by package family.
struct ListedPackage {
    package: String,
    version: String,
    description: String,
    installed: bool,
}

/// Parse `android sdk list --all` output. Both sections are kept and tagged,
/// so the UI can show uninstalled versions alongside installed ones:
///
/// ```text
/// Installed packages:
///   platforms/android-37.0   2.0.0   Android SDK Platform 37.0
/// Available packages:
///   platforms/android-36     2.0.0   Android SDK Platform 36
/// ```
fn parse_listed_packages(output: &str) -> Vec<ListedPackage> {
    let mut seen = HashSet::new();
    let mut installed = None;
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            match line {
                "Installed packages:" => {
                    installed = Some(true);
                    return None;
                }
                "Available packages:" => {
                    installed = Some(false);
                    return None;
                }
                _ => {}
            }

            let mut parts = line.split_whitespace();
            let package = parts.next()?;
            if !seen.insert(package.to_string()) {
                return None;
            }
            let version = parts.next().unwrap_or_default().to_string();
            let description = parts.collect::<Vec<_>>().join(" ");
            Some(ListedPackage {
                package: package.to_string(),
                version,
                description,
                installed: installed.unwrap_or(false),
            })
        })
        .collect()
}

/// Extract platform rows from a full `android sdk list --all` dump.
fn parse_platforms(output: &str) -> Vec<Platform> {
    let mut platforms: Vec<Platform> = parse_listed_packages(output)
        .into_iter()
        .filter_map(|listed| {
            let api_level = listed
                .package
                .strip_prefix("platforms/android-")?
                .to_string();
            Some(Platform {
                package: listed.package,
                api_level,
                version: listed.version,
                description: listed.description,
                installed: listed.installed,
            })
        })
        .collect();

    // Installed first, then newest API level; previews such as `33-ext4`
    // sort next to `33`.
    platforms.sort_by(|a, b| {
        b.installed
            .cmp(&a.installed)
            .then_with(|| sort_key(&b.api_level).cmp(&sort_key(&a.api_level)))
    });
    platforms
}

/// Extract build-tools rows from a full `android sdk list --all` dump.
fn parse_build_tools(output: &str) -> Vec<BuildTool> {
    let mut build_tools: Vec<BuildTool> = parse_listed_packages(output)
        .into_iter()
        .filter(|listed| listed.package.starts_with("build-tools/"))
        .map(|listed| BuildTool {
            package: listed.package,
            version: listed.version,
            description: listed.description,
            installed: listed.installed,
        })
        .collect();

    // Installed first, then newest version (`36.0.0` after `9.0.0`).
    build_tools.sort_by_cached_key(|tool| {
        (
            !tool.installed,
            std::cmp::Reverse(version_key(&tool.version)),
        )
    });
    build_tools
}

/// Sortable version key: numeric release components, then stable before
/// previews, then the preview suffix (`rc2` after `rc1`).
fn version_key(version: &str) -> (Vec<u64>, bool, String) {
    let (base, suffix) = version.split_once('-').unwrap_or((version, ""));
    let numbers = base
        .split('.')
        .filter_map(|part| part.parse().ok())
        .collect();
    (numbers, suffix.is_empty(), suffix.to_string())
}

fn sort_key(api_level: &str) -> (u32, String) {
    let digits: String = api_level
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    (digits.parse().unwrap_or(0), api_level.to_string())
}

/// Run an `android` subcommand, returning stdout or the last error line.
/// The child is tracked so app shutdown can terminate it.
fn run(
    bin: &std::path::Path,
    root: Option<&std::path::Path>,
    args: &[&str],
    running: &RunningCli,
) -> Result<String, String> {
    let mut command = Command::new(bin);
    command.env("PATH", child_path());
    if let Some(root) = root {
        command.arg(format!("--sdk={}", root.display()));
    }
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let child = {
        let mut state = running.state();
        if state.cancelled {
            return Err("Operation cancelled during shutdown.".to_string());
        }
        let child = command
            .spawn()
            .map_err(|err| format!("Failed to run {}: {err}", bin.display()))?;
        state.pid = Some(child.id());
        state.running = true;
        child
    };

    let output = child.wait_with_output();
    running.finish();
    let output = output.map_err(|err| format!("Failed to wait for {}: {err}", bin.display()))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{stdout}\n{stderr}");
        // Progress output rewrites lines with `\r`; report only the last one.
        let tail = combined
            .split(['\n', '\r'])
            .rev()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or("no output");
        let program = bin
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("command");
        Err(format!(
            "`{program}` exited {}: {}",
            output.status,
            truncate(tail, 200)
        ))
    }
}

fn truncate(text: &str, max_chars: usize) -> String {
    let mut truncated: String = text.chars().take(max_chars).collect();
    if text.chars().count() > max_chars {
        truncated.push('…');
    }
    truncated
}

pub fn install(
    cli: &std::path::Path,
    root: Option<&std::path::Path>,
    package: &str,
    running: &RunningCli,
) -> Result<String, String> {
    run(cli, root, &["sdk", "install", package], running)
}

pub fn update(
    cli: &std::path::Path,
    root: Option<&std::path::Path>,
    package: &str,
    running: &RunningCli,
) -> Result<String, String> {
    run(cli, root, &["sdk", "update", package], running)
}

pub fn remove(
    cli: &std::path::Path,
    root: Option<&std::path::Path>,
    package: &str,
    running: &RunningCli,
) -> Result<String, String> {
    run(cli, root, &["sdk", "remove", package], running)
}

pub fn start_emulator(
    cli: &std::path::Path,
    root: Option<&std::path::Path>,
    device: &str,
    running: &RunningCli,
) -> Result<String, String> {
    run(cli, root, &["emulator", "start", device], running)
}

pub fn stop_emulator(
    cli: &std::path::Path,
    root: Option<&std::path::Path>,
    device: &str,
    running: &RunningCli,
) -> Result<String, String> {
    run(cli, root, &["emulator", "stop", device], running)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_sorts_build_tools() {
        let output = "\
Installed packages:
  build-tools/36.0.0   36.0.0   Android SDK Build-Tools 36
  build-tools/9.0.0    9.0.0    Android SDK Build-Tools 9
Available packages:
  build-tools/34.0.0   34.0.0   Android SDK Build-Tools 34
  build-tools/36.0.0   36.0.0   Android SDK Build-Tools 36
";
        let tools = parse_build_tools(output);

        assert_eq!(tools.len(), 3);
        assert_eq!(tools[0].package, "build-tools/36.0.0");
        assert!(tools[0].installed);
        assert_eq!(tools[1].package, "build-tools/9.0.0");
        assert!(tools[1].installed);
        assert_eq!(tools[2].package, "build-tools/34.0.0");
        assert!(!tools[2].installed);
    }

    #[test]
    fn stable_build_tool_sorts_before_preview() {
        let output = "\
Available packages:
  build-tools/36.1.0-rc1   36.1.0-rc1   Android SDK Build-Tools 36.1-rc1
  build-tools/36.1.0       36.1.0       Android SDK Build-Tools 36.1
  build-tools/36.1.0-rc2   36.1.0-rc2   Android SDK Build-Tools 36.1-rc2
";
        let tools = parse_build_tools(output);

        assert_eq!(tools[0].package, "build-tools/36.1.0");
        assert_eq!(tools[1].package, "build-tools/36.1.0-rc2");
        assert_eq!(tools[2].package, "build-tools/36.1.0-rc1");
    }

    #[test]
    fn matches_build_tool_fields() {
        let tool = BuildTool {
            package: "build-tools/36.0.0".into(),
            version: "36.0.0".into(),
            description: "Android SDK Build-Tools 36".into(),
            installed: false,
        };
        assert!(tool.matches(""));
        assert!(tool.matches("36"));
        assert!(tool.matches("build-tools"));
        assert!(tool.matches("36.0.0"));
        assert!(!tool.matches("35"));
    }

    #[test]
    fn parses_adb_devices() {
        let output = "\
List of devices attached
emulator-5554 device product:sdk_gphone64_arm64 model:sdk_gphone64_arm64 device:emu64a transport_id:1
R58M12345 unauthorized usb:1-1 transport_id:2
";
        let devices = parse_devices(output);

        assert_eq!(devices.len(), 2);
        assert_eq!(
            devices[0],
            Device {
                serial: "emulator-5554".into(),
                state: "device".into(),
                model: Some("sdk_gphone64_arm64".into()),
                product: Some("sdk_gphone64_arm64".into()),
                device: Some("emu64a".into()),
                transport_id: Some("1".into()),
                usb: None,
                is_emulator: true,
            }
        );
        assert!(devices[0].is_online());
        assert_eq!(devices[1].state, "unauthorized");
        assert_eq!(devices[1].usb.as_deref(), Some("1-1"));
        assert!(!devices[1].is_online());
    }

    #[test]
    fn ignores_adb_noise_and_blank_lines() {
        let output = "\
List of devices attached

* daemon not running; starting now at tcp:5037
";
        assert!(parse_devices(output).is_empty());
    }

    #[test]
    fn parses_emulator_list_long() {
        let header = "AVD ID                   AVD Name                      API Level      Status         Serial";
        let offline =
            "Medium_Phone_API_37.0    Medium Phone API 37.0         android-37.0   Offline";
        let online = format!(
            "{:<25}{:<30}{:<15}{:<15}{}",
            "Pixel_8", "Pixel 8", "android-35", "Online", "emulator-5554"
        );
        let output = format!("{header}\n{offline}\n{online}\n");

        let emulators = parse_emulators(&output);
        assert_eq!(emulators.len(), 2);
        assert_eq!(emulators[0].id, "Medium_Phone_API_37.0");
        assert_eq!(emulators[0].name, "Medium Phone API 37.0");
        assert_eq!(emulators[0].api_level, "android-37.0");
        assert_eq!(emulators[0].status, "Offline");
        assert_eq!(emulators[0].serial, None);
        assert!(!emulators[0].is_online());
        assert_eq!(emulators[1].serial.as_deref(), Some("emulator-5554"));
        assert!(emulators[1].is_online());
    }

    #[test]
    fn parses_installed_and_available_platforms() {
        let output = "\
Installed packages:
  build-tools/36.0.0                                              36.0.0   Android SDK Build-Tools 36
  platforms/android-37.0                                          2.0.0    Android SDK Platform 37.0
  platforms/android-33-ext4                                       1.0.0    Android SDK Platform 33-ext4
Available packages:
  platforms/android-36                                           2.0.0    Android SDK Platform 36
  platforms/android-33-ext4                                       1.0.0    Android SDK Platform 33-ext4
  platforms/android-10                                           2.0.0    Android SDK Platform 10
";
        let platforms = parse_platforms(output);

        assert_eq!(platforms.len(), 4);
        assert_eq!(platforms[0].package, "platforms/android-37.0");
        assert!(platforms[0].installed);
        assert_eq!(platforms[0].api_level, "37.0");
        assert_eq!(platforms[0].version, "2.0.0");
        assert_eq!(platforms[0].description, "Android SDK Platform 37.0");
        assert_eq!(platforms[1].api_level, "33-ext4");
        assert!(platforms[1].installed);
        assert_eq!(platforms[2].api_level, "36");
        assert!(!platforms[2].installed);
        assert_eq!(platforms[3].api_level, "10");
        assert!(!platforms[3].installed);
    }

    #[test]
    fn matches_api_package_description_and_version() {
        let platform = Platform {
            package: "platforms/android-36".into(),
            api_level: "36".into(),
            version: "2.0.0".into(),
            description: "Android SDK Platform 36".into(),
            installed: false,
        };
        assert!(platform.matches(""));
        assert!(platform.matches("36"));
        assert!(platform.matches("android-36"));
        assert!(platform.matches("platform"));
        assert!(platform.matches("2.0"));
        assert!(!platform.matches("35"));
    }

    #[test]
    fn parses_info_root_and_version() {
        let (root, version) = parse_info("sdk: /Users/me/Library/Android/sdk\nversion: 1.1.169\n");
        assert_eq!(root, Some(PathBuf::from("/Users/me/Library/Android/sdk")));
        assert_eq!(version.as_deref(), Some("1.1.169"));
    }

    #[cfg(unix)]
    #[test]
    fn shutdown_stops_tracked_child() {
        let mut child = Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep");
        let running = RunningCli::default();
        running.track_for_test(child.id());

        let started = Instant::now();
        running.shutdown(Duration::from_millis(100));
        let status = child.wait().expect("reap sleep");

        assert!(!status.success());
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn search_paths_fall_back_to_standard_bin_dirs() {
        let path_var = env::join_paths([PathBuf::from("/usr/bin"), PathBuf::from("/bin")]).unwrap();
        let paths = search_paths_from(Some(&path_var));

        assert_eq!(paths[0], PathBuf::from("/usr/bin"));
        assert!(paths.contains(&PathBuf::from("/usr/local/bin")));
        #[cfg(target_os = "macos")]
        assert!(paths.contains(&PathBuf::from("/opt/homebrew/bin")));
    }

    #[test]
    fn search_paths_do_not_duplicate_existing_dirs() {
        let path_var =
            env::join_paths([PathBuf::from("/usr/local/bin"), PathBuf::from("/usr/bin")]).unwrap();
        let paths = search_paths_from(Some(&path_var));

        assert_eq!(
            paths
                .iter()
                .filter(|path| *path == &PathBuf::from("/usr/local/bin"))
                .count(),
            1
        );
    }

    #[cfg(unix)]
    #[test]
    fn only_executable_files_count_as_cli() {
        use std::{fs, os::unix::fs::PermissionsExt};

        let dir = std::env::temp_dir().join(format!("acg-cli-test-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("android");
        fs::write(&path, "#!/bin/sh\n").expect("write fake cli");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("chmod 644");
        assert!(!is_executable(&path));

        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod 755");
        assert!(is_executable(&path));

        fs::remove_dir_all(&dir).expect("clean temp dir");
    }

    #[test]
    fn shutdown_refuses_new_commands() {
        let running = RunningCli::default();
        running.shutdown(Duration::ZERO);

        let error = run(
            std::path::Path::new("/nonexistent/android"),
            None,
            &["info"],
            &running,
        )
        .unwrap_err();

        assert_eq!(error, "Operation cancelled during shutdown.");
    }
}
