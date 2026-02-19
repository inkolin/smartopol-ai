# Self-Update System

Skynet includes a built-in self-update mechanism that checks GitHub Releases for newer versions and applies updates in-place. The system supports three installation modes (source, binary, Docker), provides SHA256 integrity verification for binary downloads, and offers rollback to the previous version.

## Overview

The update system is split across two crates:

| Crate | File | Responsibility |
|-------|------|----------------|
| `skynet-core` | `src/update.rs` | Shared types (`ReleaseInfo`, `InstallMode`, `UpdateCheckState`), semver comparison, 24h interval logic |
| `skynet-gateway` | `src/update.rs` | GitHub API client, install mode detection, update application, rollback, restart, startup check |

On startup the gateway spawns a fire-and-forget task that checks GitHub for a newer release. If one exists, a log line is emitted. The user can then run a CLI command or invoke a WS method to apply the update.

## Install Mode Detection

The system auto-detects how Skynet was installed by inspecting the runtime environment:

```
detect_install_mode()
  |
  +-- /.dockerenv exists?  --> Docker
  |
  +-- walk up from current_exe() looking for .git/ directory
  |     found?  --> Source { repo_root }
  |
  +-- fallback --> Binary { exe_path }
```

| Mode | Detection | Update strategy |
|------|-----------|----------------|
| **Source** | `.git/` directory found above the binary | `git fetch --all --tags` + `git checkout v{version}` + `cargo build --release` |
| **Binary** | No `.git/` directory, no `/.dockerenv` | Download platform-specific tarball from GitHub Releases, SHA256 verify, atomic replace |
| **Docker** | `/.dockerenv` file exists | Print instructions: `docker compose pull && docker compose up -d` |

The `InstallMode` enum is defined in `skynet-core` and serialized as `"source"`, `"binary"`, or `"docker"` in API responses.

## CLI Commands

### `skynet-gateway version`

Print detailed version information and exit.

```
$ skynet-gateway version
skynet-gateway 0.5.0 (a1b2c3d) [source]
Protocol: v3
Data dir: /home/user/.skynet
```

Output includes the version from `Cargo.toml`, the short git commit hash (embedded at compile time by `build.rs`), the detected install mode in brackets, the protocol version, and the data directory path.

### `skynet-gateway update`

Check for updates and apply if available.

```
$ skynet-gateway update
Update available: v0.4.0 -> v0.5.0
This will git fetch + checkout v0.5.0 + cargo build in /home/user/smartopol-ai
Proceed? [y/N] y
Fetching tags...
Checking out v0.5.0...
Building (this may take a few minutes)...
Build complete.

Updated to v0.5.0. Restarting...
```

**Flags:**

| Flag | Short | Description |
|------|-------|-------------|
| `--check` | | Only check for updates, do not apply |
| `--yes` | `-y` | Skip the confirmation prompt |
| `--rollback` | | Restore the `.bak` backup binary (binary installs only) |

**`--check` example:**

```
$ skynet-gateway update --check
Checking for updates...

  Update available: v0.4.0 -> v0.5.0
  Release: https://github.com/inkolin/smartopol-ai/releases/tag/v0.5.0

  Run: skynet-gateway update
```

**`--rollback` example:**

```
$ skynet-gateway update --rollback
Rolled back to previous version.
Restarting...
```

Rollback is only supported for binary installs. For source installs, use `git checkout <previous-tag>`. For Docker, use `docker compose pull` with a specific image tag.

## WebSocket Methods

Three new `system.*` methods are exposed over the WebSocket protocol.

### system.version

Returns version metadata. No params required.

**Request:**
```json
{ "type": "req", "id": "1", "method": "system.version" }
```

**Response:**
```json
{
  "type": "res", "id": "1", "ok": true,
  "payload": {
    "version": "0.5.0",
    "git_sha": "a1b2c3d",
    "protocol": 3,
    "install_mode": "source"
  }
}
```

### system.check_update

Queries the GitHub Releases API for the latest release and compares it to the running version.

**Request:**
```json
{ "type": "req", "id": "2", "method": "system.check_update" }
```

**Response (update available):**
```json
{
  "type": "res", "id": "2", "ok": true,
  "payload": {
    "update_available": true,
    "current": "0.4.0",
    "latest": "0.5.0",
    "release_url": "https://github.com/inkolin/smartopol-ai/releases/tag/v0.5.0",
    "published_at": "2026-02-19T12:00:00Z"
  }
}
```

**Response (up to date):**
```json
{
  "type": "res", "id": "2", "ok": true,
  "payload": {
    "update_available": false,
    "current": "0.5.0",
    "latest": "0.5.0",
    "release_url": "...",
    "published_at": "..."
  }
}
```

### system.update

Triggers the update flow. The response is sent before the server restarts so the client receives it.

**Params (optional):**
```json
{ "yes": true }
```

WS callers are assumed to consent, so `yes` defaults to `true`.

**Response (updating):**
```json
{
  "type": "res", "id": "3", "ok": true,
  "payload": {
    "status": "updating",
    "from": "0.4.0",
    "to": "0.5.0",
    "message": "Update started. Server will restart shortly."
  }
}
```

**Response (Docker):**
```json
{
  "type": "res", "id": "3", "ok": true,
  "payload": {
    "status": "docker",
    "message": "Running in Docker. Update with: docker compose pull && docker compose up -d"
  }
}
```

**Response (already up to date):**
```json
{
  "type": "res", "id": "3", "ok": true,
  "payload": {
    "status": "up_to_date",
    "version": "0.5.0"
  }
}
```

The actual update is spawned as a background Tokio task with a 500ms delay so the WS response frame is flushed before the process exits.

## Update Paths in Detail

### Source Update

1. `git fetch --all --tags` in the repository root
2. `git checkout v{version}` to the release tag
3. Detect the Cargo workspace directory (`skynet/Cargo.toml` or `./Cargo.toml`)
4. `cargo build --release --bin skynet-gateway`
5. Restart the service

### Binary Update

1. Determine the platform target triple (`x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, `aarch64-apple-darwin`)
2. Find the matching `skynet-gateway-{target}.tar.gz` asset in the release
3. Download the tarball (300s timeout)
4. If `SHA256SUMS` is present in the release assets, download it and verify the checksum
5. Extract to a temporary directory using `tar xzf`
6. Atomic replace: rename current binary to `.bak`, copy new binary into place
7. Set executable permissions (`chmod 755`) on Unix
8. Clean up temp directory
9. Restart the service

### Docker Update

The system detects Docker by the presence of `/.dockerenv` and does not attempt an in-place update. Instead, it prints instructions:

```
docker compose pull && docker compose up -d
```

## SHA256 Verification

Binary downloads are verified against the `SHA256SUMS` file published with each release. The file format follows the standard `sha256sum` output:

```
abc123def456...  skynet-gateway-x86_64-unknown-linux-gnu.tar.gz
789012fed345...  skynet-gateway-aarch64-apple-darwin.tar.gz
```

Verification uses the `sha2` crate for hashing and the `hex` crate for encoding. If the checksum does not match, the update is aborted with a clear error message. If no `SHA256SUMS` file is present in the release, verification is skipped.

## Rollback Mechanism

When a binary update is applied, the current binary is renamed to `{exe_path}.bak` before the new binary is copied into place. The `--rollback` flag reverses this:

```
skynet-gateway.bak  -->  skynet-gateway
```

After restoring the backup, the service is restarted. Only one backup level is kept. Rollback is only available for binary installs.

For source installs, the equivalent is:

```bash
cd /path/to/repo
git checkout v0.4.0
cd skynet
cargo build --release
```

## Restart Mechanism

After an update is applied, the gateway needs to restart itself. This is done via a detached shell script that:

1. Writes a temporary restart script to `/tmp/skynet-restart-{pid}.sh`
2. Waits 1 second for the current process to exit
3. Attempts a platform-specific restart:
   - **Linux:** `systemctl --user restart skynet-gateway.service`, falling back to `systemctl restart`, falling back to direct binary execution
   - **macOS:** `launchctl kickstart -k gui/$(id -u)/ai.smartopol.gateway`, falling back to direct binary execution
   - **Other:** Direct binary execution
4. Deletes itself

The script is spawned with stdin, stdout, and stderr redirected to `/dev/null` so it survives the parent process exit.

## Startup Update Check

On server start, if `config.update.check_on_start` is `true` (the default), the gateway spawns a fire-and-forget task:

1. Load `~/.skynet/update-check.json`
2. If the last check was less than 24 hours ago, skip
3. Query GitHub Releases API for the latest version
4. If a newer version is available, emit an `info` log line:
   ```
   Update available: v0.5.0 (current: v0.4.0). Run: skynet-gateway update
   ```
5. Save the updated state to `update-check.json`

The `update-check.json` file tracks:

```json
{
  "last_checked_at": "2026-02-19T10:00:00+00:00",
  "latest_version": "0.5.0",
  "notified": true
}
```

The `should_check()` method uses `chrono` to compare timestamps and enforce the 24-hour interval. On any parse error or missing file, a check is performed.

## Configuration

### skynet.toml

```toml
[update]
check_on_start = true    # default: true
```

### Environment Variable

```bash
export SKYNET_UPDATE_CHECK_ON_START=false
```

The environment variable follows the standard `SKYNET_*` override convention used by all other config keys.

## Health Endpoint Changes

The `GET /health` endpoint now includes the `git_sha` field:

```json
{
  "status": "ok",
  "version": "0.5.0",
  "git_sha": "a1b2c3d",
  "protocol": 3,
  "ws_clients": 2
}
```

The git SHA is embedded at compile time by `build.rs`, which runs `git rev-parse --short HEAD` and sets the `SKYNET_GIT_SHA` environment variable for the compiler. The build script watches `.git/HEAD` and `.git/refs/` so it re-runs on new commits.

## GitHub Actions Release Workflow

The `.github/workflows/release.yml` workflow triggers on tags matching `v*` and builds binaries for four targets:

| Target | Runner | Method |
|--------|--------|--------|
| `x86_64-unknown-linux-gnu` | `ubuntu-latest` | Native cargo |
| `aarch64-unknown-linux-gnu` | `ubuntu-latest` | `cross` (cross-compilation) |
| `x86_64-apple-darwin` | `macos-13` | Native cargo |
| `aarch64-apple-darwin` | `macos-latest` | Native cargo |

The workflow:

1. **Build job (matrix):** Compiles `skynet-gateway` for each target, packages into `skynet-gateway-{target}.tar.gz`, uploads as artifact
2. **Release job:** Downloads all artifacts, generates `SHA256SUMS` via `sha256sum *.tar.gz`, creates a GitHub Release with auto-generated release notes and all files attached

This pipeline produces the exact assets that `apply_binary_update()` expects to find.

## Comparison with OpenClaw

| Feature | Skynet | OpenClaw |
|---------|--------|----------|
| Update check | 24h interval, fire-and-forget | On every start |
| Source update | `git fetch` + `cargo build` (~30s incremental) | `git pull` + `npm install` + restart |
| Binary update | Platform-specific tarball + SHA256 | Not supported |
| Docker update | Detection + instructions | Not supported |
| Integrity | SHA256SUMS verification | None |
| Rollback | `.bak` file swap | None |
| Install mode detection | Automatic (Docker > Source > Binary) | Manual |
| WS API | `system.version`, `system.check_update`, `system.update` | None |
| CLI | `update --check`, `update --yes`, `update --rollback`, `version` | None |

## Tests

The update module includes unit tests in both crates:

**`skynet-core/src/update.rs`:**
- `version_compare_basic` — basic semver ordering
- `version_compare_with_v_prefix` — handles `v` prefix
- `version_compare_major_minor` — major/minor overflow
- `state_should_check_no_previous` — first run always checks
- `state_should_check_recent` — skips within 24h
- `state_should_check_old` — checks after 24h

**`skynet-gateway/src/update.rs`:**
- `detect_mode_not_docker` — verifies non-Docker detection in test environment
- `parse_sha256sums` — parses SHA256SUMS format correctly
