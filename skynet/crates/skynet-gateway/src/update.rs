use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use skynet_core::update::{
    compare_versions, InstallMode, ReleaseAsset, ReleaseInfo, UpdateCheckState,
};
use tracing::{info, warn};

const GITHUB_API: &str = "https://api.github.com/repos/inkolin/smartopol-ai/releases/latest";
const USER_AGENT: &str = "skynet-gateway";

/// Current version from Cargo.toml.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Short git commit hash embedded at compile time by build.rs.
pub const GIT_SHA: &str = env!("SKYNET_GIT_SHA");

// ─── GitHub API ──────────────────────────────────────────────────────────────

/// Query GitHub Releases API for the latest release.
pub async fn check_latest_release() -> Result<ReleaseInfo> {
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    let resp: serde_json::Value = client
        .get(GITHUB_API)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .context("failed to reach GitHub API")?
        .error_for_status()
        .context("GitHub API returned error status")?
        .json()
        .await
        .context("failed to parse GitHub API response")?;

    let tag_name = resp["tag_name"]
        .as_str()
        .context("missing tag_name in release")?
        .to_string();
    let version = tag_name.strip_prefix('v').unwrap_or(&tag_name).to_string();

    let assets = resp["assets"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|a| {
                    Some(ReleaseAsset {
                        name: a["name"].as_str()?.to_string(),
                        download_url: a["browser_download_url"].as_str()?.to_string(),
                        size: a["size"].as_u64().unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(ReleaseInfo {
        tag_name,
        version,
        published_at: resp["published_at"]
            .as_str()
            .unwrap_or("unknown")
            .to_string(),
        html_url: resp["html_url"]
            .as_str()
            .unwrap_or("https://github.com/inkolin/smartopol-ai/releases")
            .to_string(),
        assets,
    })
}

// ─── Install Mode Detection ──────────────────────────────────────────────────

/// Auto-detect how Skynet was installed.
pub fn detect_install_mode() -> InstallMode {
    // Docker: /.dockerenv exists inside containers.
    if Path::new("/.dockerenv").exists() {
        return InstallMode::Docker;
    }

    // Source: walk up from the binary looking for a .git directory.
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(|p| p.to_path_buf());
        while let Some(ref d) = dir {
            if d.join(".git").is_dir() {
                return InstallMode::Source {
                    repo_root: d.to_string_lossy().to_string(),
                };
            }
            dir = d.parent().map(|p| p.to_path_buf());
        }

        return InstallMode::Binary {
            exe_path: exe.to_string_lossy().to_string(),
        };
    }

    // Fallback.
    InstallMode::Binary {
        exe_path: "skynet-gateway".to_string(),
    }
}

/// Return the platform target triple suffix for asset matching.
fn platform_asset_suffix() -> &'static str {
    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
    {
        "x86_64-unknown-linux-gnu"
    }
    #[cfg(all(target_arch = "aarch64", target_os = "linux"))]
    {
        "aarch64-unknown-linux-gnu"
    }
    #[cfg(all(target_arch = "x86_64", target_os = "macos"))]
    {
        "x86_64-apple-darwin"
    }
    #[cfg(all(target_arch = "aarch64", target_os = "macos"))]
    {
        "aarch64-apple-darwin"
    }
    #[cfg(not(any(
        all(target_arch = "x86_64", target_os = "linux"),
        all(target_arch = "aarch64", target_os = "linux"),
        all(target_arch = "x86_64", target_os = "macos"),
        all(target_arch = "aarch64", target_os = "macos"),
    )))]
    {
        "unknown"
    }
}

// ─── Version Check (CLI) ────────────────────────────────────────────────────

/// Check for updates and print the result. Returns true if an update is available.
pub async fn check_and_print() -> Result<bool> {
    println!("Checking for updates...");

    let release = check_latest_release().await?;
    let current = VERSION;
    let latest = &release.version;

    match compare_versions(current, latest) {
        Ordering::Less => {
            println!();
            println!("  Update available: v{} -> v{}", current, latest);
            println!("  Release: {}", release.html_url);
            println!();
            println!("  Run: skynet-gateway update");
            Ok(true)
        }
        _ => {
            println!("  You are up to date (v{}).", current);
            Ok(false)
        }
    }
}

// ─── Apply Update ───────────────────────────────────────────────────────────

/// Run the full update flow based on the detected install mode.
pub async fn apply_update(yes: bool) -> Result<()> {
    let release = check_latest_release().await?;
    let current = VERSION;
    let latest = &release.version;

    if compare_versions(current, latest) != Ordering::Less {
        println!("You are already on the latest version (v{}).", current);
        return Ok(());
    }

    println!("Update available: v{} -> v{}", current, latest);

    let mode = detect_install_mode();

    match mode {
        InstallMode::Docker => {
            println!();
            println!("Running in Docker. Update with:");
            println!();
            println!("  docker compose pull && docker compose up -d");
            println!();
            return Ok(());
        }
        InstallMode::Source { ref repo_root } => {
            if !yes {
                println!(
                    "This will git fetch + checkout v{} + cargo build in {}",
                    latest, repo_root
                );
                if !confirm("Proceed?")? {
                    println!("Aborted.");
                    return Ok(());
                }
            }
            apply_source_update(latest, Path::new(repo_root)).await?;
        }
        InstallMode::Binary { ref exe_path } => {
            if !yes {
                println!("This will download the new binary and replace {}", exe_path);
                if !confirm("Proceed?")? {
                    println!("Aborted.");
                    return Ok(());
                }
            }
            apply_binary_update(&release, Path::new(exe_path)).await?;
        }
    }

    println!();
    println!("Updated to v{}. Restarting...", latest);
    restart_service()?;

    Ok(())
}

/// Source update: git fetch + checkout tag + cargo build.
async fn apply_source_update(version: &str, repo_root: &Path) -> Result<()> {
    let tag = format!("v{}", version);

    println!("Fetching tags...");
    run_cmd(repo_root, "git", &["fetch", "--all", "--tags"])?;

    println!("Checking out {}...", tag);
    run_cmd(repo_root, "git", &["checkout", &tag])?;

    // The Cargo workspace is in the skynet/ subdirectory.
    let cargo_dir = if repo_root.join("skynet").join("Cargo.toml").exists() {
        repo_root.join("skynet")
    } else {
        repo_root.to_path_buf()
    };

    println!("Building (this may take a few minutes)...");
    run_cmd(
        &cargo_dir,
        "cargo",
        &["build", "--release", "--bin", "skynet-gateway"],
    )?;

    println!("Build complete.");
    Ok(())
}

/// Binary update: download asset + SHA256 verify + atomic replace.
async fn apply_binary_update(release: &ReleaseInfo, exe_path: &Path) -> Result<()> {
    let suffix = platform_asset_suffix();
    let asset_name = format!("skynet-gateway-{}.tar.gz", suffix);

    let asset = release
        .assets
        .iter()
        .find(|a| a.name == asset_name)
        .context(format!(
            "no binary for this platform ({}) in release",
            suffix
        ))?;

    println!("Downloading {} ({} bytes)...", asset.name, asset.size);

    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(300))
        .build()?;

    let bytes = client
        .get(&asset.download_url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;

    // SHA256 verification — download SHA256SUMS if available.
    if let Some(sums_asset) = release.assets.iter().find(|a| a.name == "SHA256SUMS") {
        println!("Verifying SHA256 checksum...");
        let sums_text = client
            .get(&sums_asset.download_url)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;

        if let Some(expected) = parse_sha256_for(&sums_text, &asset.name) {
            let actual = sha256_hex(&bytes);
            if actual != expected {
                bail!(
                    "SHA256 mismatch: expected {}, got {}. Download may be corrupted.",
                    expected,
                    actual
                );
            }
            println!("Checksum OK.");
        }
    }

    // Extract the tarball to a temp directory.
    let tmp_dir = std::env::temp_dir().join(format!("skynet-update-{}", std::process::id()));
    std::fs::create_dir_all(&tmp_dir)?;

    let tar_path = tmp_dir.join(&asset.name);
    std::fs::write(&tar_path, &bytes)?;

    // Extract using tar (available on all target platforms).
    run_cmd(&tmp_dir, "tar", &["xzf", &tar_path.to_string_lossy()])?;

    let new_binary = tmp_dir.join("skynet-gateway");
    if !new_binary.exists() {
        bail!("extracted archive does not contain skynet-gateway binary");
    }

    // Atomic replace: current -> .bak, new -> current.
    let bak_path = exe_path.with_extension("bak");
    if exe_path.exists() {
        std::fs::rename(exe_path, &bak_path)
            .context("failed to create backup of current binary")?;
        println!("Backup saved to {}", bak_path.display());
    }

    std::fs::copy(&new_binary, exe_path).context("failed to install new binary")?;

    // Make executable on Unix.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(exe_path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(exe_path, perms)?;
    }

    // Clean up temp.
    let _ = std::fs::remove_dir_all(&tmp_dir);

    Ok(())
}

// ─── Rollback ───────────────────────────────────────────────────────────────

/// Restore the `.bak` backup binary.
pub fn rollback() -> Result<()> {
    let mode = detect_install_mode();
    let exe_path = match &mode {
        InstallMode::Binary { exe_path } => PathBuf::from(exe_path),
        InstallMode::Source { .. } => {
            bail!("Rollback is only supported for binary installs. For source installs, use: git checkout <previous-tag>");
        }
        InstallMode::Docker => {
            bail!("Rollback is not supported in Docker. Use: docker compose pull to get a specific version.");
        }
    };

    let bak_path = exe_path.with_extension("bak");
    if !bak_path.exists() {
        bail!(
            "No backup found at {}. Nothing to roll back to.",
            bak_path.display()
        );
    }

    std::fs::rename(&bak_path, &exe_path).context("failed to restore backup binary")?;

    println!("Rolled back to previous version.");
    println!("Restarting...");
    restart_service()?;

    Ok(())
}

// ─── Restart ────────────────────────────────────────────────────────────────

/// Platform-specific restart via detached shell script.
pub fn restart_service() -> Result<()> {
    let exe = std::env::current_exe().context("cannot determine current executable path")?;
    let exe_str = exe.to_string_lossy();
    let pid = std::process::id();

    let script = if cfg!(target_os = "linux") {
        format!(
            "#!/bin/sh\nsleep 1\nsystemctl --user restart skynet-gateway.service 2>/dev/null || \\\n  systemctl restart skynet-gateway.service 2>/dev/null || \\\n  \"{}\" &\nrm -f \"$0\"\n",
            exe_str
        )
    } else if cfg!(target_os = "macos") {
        format!(
            "#!/bin/sh\nsleep 1\nlaunchctl kickstart -k gui/$(id -u)/ai.smartopol.gateway 2>/dev/null || \"{}\" &\nrm -f \"$0\"\n",
            exe_str
        )
    } else {
        format!("#!/bin/sh\nsleep 1\n\"{}\" &\nrm -f \"$0\"\n", exe_str)
    };

    let script_path = std::env::temp_dir().join(format!("skynet-restart-{}.sh", pid));
    std::fs::write(&script_path, &script)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&script_path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script_path, perms)?;
    }

    std::process::Command::new("sh")
        .arg(&script_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("failed to spawn restart script")?;

    info!("restart script spawned, shutting down");
    Ok(())
}

// ─── Startup Check ──────────────────────────────────────────────────────────

/// Fire-and-forget update check on startup. Respects 24h interval and config.
pub async fn check_update_on_startup(data_dir: &Path) {
    let state = UpdateCheckState::load(data_dir);
    if !state.should_check() {
        return;
    }

    match check_latest_release().await {
        Ok(release) => {
            let now = chrono::Utc::now().to_rfc3339();
            let mut new_state = UpdateCheckState {
                last_checked_at: Some(now),
                latest_version: Some(release.version.clone()),
                notified: false,
            };

            if compare_versions(VERSION, &release.version) == Ordering::Less {
                info!(
                    current = VERSION,
                    latest = %release.version,
                    "Update available: v{} (current: v{}). Run: skynet-gateway update",
                    release.version, VERSION
                );
                new_state.notified = true;
            }

            new_state.save(data_dir);
        }
        Err(e) => {
            warn!(error = %e, "startup update check failed (non-fatal)");
        }
    }
}

// ─── Version Command ────────────────────────────────────────────────────────

/// Print detailed version info and exit.
pub fn print_version() {
    let mode = detect_install_mode();

    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let data_dir = format!("{}/.skynet", home);

    println!("skynet-gateway {} ({}) [{}]", VERSION, GIT_SHA, mode);
    println!("Protocol: v{}", skynet_core::config::PROTOCOL_VERSION);
    println!("Data dir: {}", data_dir);
}

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Run a shell command synchronously. Prints output on failure.
fn run_cmd(cwd: &Path, program: &str, args: &[&str]) -> Result<()> {
    let output = std::process::Command::new(program)
        .args(args)
        .current_dir(cwd)
        .output()
        .context(format!("failed to execute: {} {:?}", program, args))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("{} {:?} failed: {}", program, args, stderr.trim());
    }
    Ok(())
}

/// Read a y/n confirmation from stdin.
fn confirm(prompt: &str) -> Result<bool> {
    use std::io::Write;
    print!("{} [y/N] ", prompt);
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(input.trim().eq_ignore_ascii_case("y"))
}

/// Compute SHA256 hex digest of a byte slice.
fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// Parse a SHA256SUMS file for a specific filename.
fn parse_sha256_for(sums: &str, filename: &str) -> Option<String> {
    for line in sums.lines() {
        // Format: "<hash>  <filename>" or "<hash> <filename>"
        let parts: Vec<&str> = line.splitn(2, char::is_whitespace).collect();
        if parts.len() == 2 && parts[1].trim() == filename {
            return Some(parts[0].to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_mode_not_docker() {
        // In test env, we're not in Docker.
        let mode = detect_install_mode();
        assert_ne!(mode, InstallMode::Docker);
    }

    #[test]
    fn parse_sha256sums() {
        let sums = "abc123  skynet-gateway-x86_64-unknown-linux-gnu.tar.gz\ndef456  skynet-gateway-aarch64-apple-darwin.tar.gz\n";
        assert_eq!(
            parse_sha256_for(sums, "skynet-gateway-x86_64-unknown-linux-gnu.tar.gz"),
            Some("abc123".to_string())
        );
        assert_eq!(
            parse_sha256_for(sums, "skynet-gateway-aarch64-apple-darwin.tar.gz"),
            Some("def456".to_string())
        );
        assert_eq!(parse_sha256_for(sums, "nonexistent.tar.gz"), None);
    }
}
