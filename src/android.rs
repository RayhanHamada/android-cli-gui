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

/// A point-in-time view of the SDK that the UI renders.
#[derive(Clone, Debug, Default)]
pub struct SdkSnapshot {
    /// Path to the `android` binary, when one is on `PATH`.
    pub cli: Option<PathBuf>,
    pub cli_version: Option<String>,
    pub root: Option<PathBuf>,
    pub platforms: Vec<Platform>,
    /// Non-fatal CLI or parse failure.
    pub error: Option<String>,
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
    if cfg!(windows) { "android.exe" } else { "android" }
}

/// Find the official Android CLI on `PATH`.
pub fn find_cli() -> Option<PathBuf> {
    let name = cli_binary_name();
    let path_var = env::var_os("PATH")?;
    env::split_paths(&path_var)
        .map(|dir| dir.join(name))
        .find(|path| is_executable(path))
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

    match run(
        &cli,
        root.as_deref(),
        &["sdk", "list", "--all", "platforms*"],
        &running,
    ) {
        Ok(output) => snapshot.platforms = parse_platforms(&output),
        Err(err) => snapshot.error = Some(err),
    }
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

/// Parse `android sdk list --all platforms*` output. Both sections are kept
/// and tagged, so the UI can show uninstalled versions alongside installed
/// ones:
///
/// ```text
/// Installed packages:
///   platforms/android-37.0   2.0.0   Android SDK Platform 37.0
/// Available packages:
///   platforms/android-36     2.0.0   Android SDK Platform 36
/// ```
fn parse_platforms(output: &str) -> Vec<Platform> {
    let mut seen = HashSet::new();
    let mut installed = None;
    let mut platforms: Vec<Platform> = output
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
            let api_level = package.strip_prefix("platforms/android-")?;
            if !seen.insert(package.to_string()) {
                return None;
            }
            let version = parts.next().unwrap_or_default().to_string();
            let description = parts.collect::<Vec<_>>().join(" ");
            Some(Platform {
                package: package.to_string(),
                api_level: api_level.to_string(),
                version,
                description,
                installed: installed.unwrap_or(false),
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
    let output =
        output.map_err(|err| format!("Failed to wait for {}: {err}", bin.display()))?;

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
        Err(format!(
            "`android` exited {}: {}",
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut child = Command::new("sleep").arg("30").spawn().expect("spawn sleep");
        let running = RunningCli::default();
        running.track_for_test(child.id());

        let started = Instant::now();
        running.shutdown(Duration::from_millis(100));
        let status = child.wait().expect("reap sleep");

        assert!(!status.success());
        assert!(started.elapsed() < Duration::from_secs(2));
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
