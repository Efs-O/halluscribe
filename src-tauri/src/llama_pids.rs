// HalluScribe - registry of llama-server child PIDs this app spawned.
//
// Why this exists (OPS-1): the per-process `infer_lock` only serialises model
// jobs inside ONE app instance, and `Drop` (which kills a spawned server) never
// runs when the app is force-killed. A hard-killed HalluScribe therefore ORPHANS
// its llama-server child, which keeps holding VRAM; the next launch's model then
// cannot fit on the GPU and spills to CPU (the 2026-07-04 6-hour regression).
//
// This module reaps ONLY our own orphans. It never matches on the binary name
// because HalluScribe shares the exact `llama-server.exe` path with the user's
// AdvanLLM setup - killing by binary would kill unrelated work. Instead we record
// `<app_pid> <server_pid>` pairs in a marker file when we spawn, and only kill a
// recorded server when (a) the app process that spawned it is dead - so a live
// second HalluScribe window's servers are spared - and (b) the server PID is
// still a live `llama-server` process, which guards against the OS having reused
// that PID for something unrelated after the orphan died.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

/// One recorded spawn: which app instance launched which llama-server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Entry {
    app_pid: u32,
    server_pid: u32,
}

/// Serialises marker-file read/modify/write within this process. Cross-process
/// races are possible but benign (single-instance running is the common case and
/// the reap is best-effort orphan cleanup, not a correctness guarantee).
fn file_lock() -> &'static Mutex<()> {
    static LOCK: Mutex<()> = Mutex::new(());
    &LOCK
}

/// Resolve `<home>/.halluscribe/.llama-servers.pids`, falling back to the system
/// temp dir when the home directory cannot be resolved. Every app instance on the
/// machine resolves the same path, so a later launch can see a prior one's PIDs.
fn marker_path() -> PathBuf {
    let base = home_dir()
        .map(|home| home.join(".halluscribe"))
        .unwrap_or_else(std::env::temp_dir);
    base.join(".llama-servers.pids")
}

fn home_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("USERPROFILE").map(PathBuf::from)
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

fn read_entries() -> Vec<Entry> {
    let path = marker_path();
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    parse_entries(&contents)
}

fn parse_entries(contents: &str) -> Vec<Entry> {
    contents
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let app_pid = parts.next()?.parse().ok()?;
            let server_pid = parts.next()?.parse().ok()?;
            Some(Entry {
                app_pid,
                server_pid,
            })
        })
        .collect()
}

fn serialize_entries(entries: &[Entry]) -> String {
    let mut out = String::new();
    for entry in entries {
        out.push_str(&entry.app_pid.to_string());
        out.push(' ');
        out.push_str(&entry.server_pid.to_string());
        out.push('\n');
    }
    out
}

fn write_entries(entries: &[Entry]) {
    let path = marker_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, serialize_entries(entries));
}

/// Record a llama-server we just spawned so a future launch can reap it if we die
/// before `Drop` runs. Idempotent: a duplicate pair is not appended twice.
pub fn register(server_pid: u32) {
    let _guard = file_lock().lock().unwrap_or_else(|p| p.into_inner());
    let mut entries = read_entries();
    let entry = Entry {
        app_pid: std::process::id(),
        server_pid,
    };
    if !entries.contains(&entry) {
        entries.push(entry);
        write_entries(&entries);
    }
}

/// Drop our record of a server we just killed cleanly (from `Drop`/`kill_*`), so
/// its PID is never a reap candidate after the OS potentially reuses it.
pub fn unregister(server_pid: u32) {
    let _guard = file_lock().lock().unwrap_or_else(|p| p.into_inner());
    let our_pid = std::process::id();
    let mut entries = read_entries();
    let before = entries.len();
    entries.retain(|entry| !(entry.app_pid == our_pid && entry.server_pid == server_pid));
    if entries.len() != before {
        write_entries(&entries);
    }
}

/// Kill any recorded llama-server orphaned by a dead app instance, then rewrite
/// the marker file without the reaped/stale rows. Call before spawning a new
/// server so a leftover model releases its VRAM first.
pub fn reap_orphans() {
    let _guard = file_lock().lock().unwrap_or_else(|p| p.into_inner());
    let entries = read_entries();
    let plan = plan_reap(&entries, std::process::id(), pid_alive, pid_is_llama_server);
    for server_pid in &plan.to_kill {
        kill_pid(*server_pid);
    }
    write_entries(&plan.keep);
}

struct ReapPlan {
    to_kill: Vec<u32>,
    keep: Vec<Entry>,
}

/// Pure reap decision, split out so it can be tested without real processes.
/// - owner is us, or another live app instance -> keep the row, never kill.
/// - owner dead + server still a llama-server -> orphan: kill and drop the row.
/// - owner dead + server not a llama-server -> gone/reused: drop the row, no kill.
fn plan_reap(
    entries: &[Entry],
    our_app_pid: u32,
    owner_alive: impl Fn(u32) -> bool,
    server_is_llama: impl Fn(u32) -> bool,
) -> ReapPlan {
    let mut to_kill = Vec::new();
    let mut keep = Vec::new();
    for entry in entries {
        if entry.app_pid == our_app_pid || owner_alive(entry.app_pid) {
            keep.push(*entry);
        } else if server_is_llama(entry.server_pid) {
            to_kill.push(entry.server_pid);
        }
    }
    ReapPlan { to_kill, keep }
}

/// True if any process with this PID currently exists.
fn pid_alive(pid: u32) -> bool {
    process_image(pid).is_some()
}

/// True only if a live process with this PID is a llama-server binary.
fn pid_is_llama_server(pid: u32) -> bool {
    process_image(pid)
        .map(|image| image.to_ascii_lowercase().contains("llama-server"))
        .unwrap_or(false)
}

/// Return the executable image/command name for a live PID, or `None` if no such
/// process exists. Shells out to the platform's process lister (no new deps).
#[cfg(target_os = "windows")]
fn process_image(pid: u32) -> Option<String> {
    let mut cmd = Command::new("tasklist");
    cmd.args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"]);
    crate::llama_runtime::apply_no_window(&mut cmd);
    let output = cmd.output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    // A match is a CSV row like `"llama-server.exe","1234",...`; no match prints
    // an "INFO: No tasks..." line to stdout instead.
    let line = text
        .lines()
        .find(|line| line.trim_start().starts_with('"'))?;
    let image = line.trim_start_matches('"');
    let image = image.split('"').next()?;
    Some(image.to_string())
}

#[cfg(not(target_os = "windows"))]
fn process_image(pid: u32) -> Option<String> {
    let output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

#[cfg(target_os = "windows")]
fn kill_pid(pid: u32) {
    let mut cmd = Command::new("taskkill");
    cmd.args(["/F", "/PID", &pid.to_string()]);
    crate::llama_runtime::apply_no_window(&mut cmd);
    let _ = cmd.output();
}

#[cfg(not(target_os = "windows"))]
fn kill_pid(pid: u32) {
    let _ = Command::new("kill").args(["-9", &pid.to_string()]).output();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_round_trips_serialize() {
        let entries = vec![
            Entry {
                app_pid: 100,
                server_pid: 200,
            },
            Entry {
                app_pid: 101,
                server_pid: 201,
            },
        ];
        assert_eq!(parse_entries(&serialize_entries(&entries)), entries);
    }

    #[test]
    fn parse_skips_malformed_lines() {
        let text = "100 200\ngarbage\n101\n102 300\n";
        assert_eq!(
            parse_entries(text),
            vec![
                Entry {
                    app_pid: 100,
                    server_pid: 200
                },
                Entry {
                    app_pid: 102,
                    server_pid: 300
                },
            ]
        );
    }

    #[test]
    fn spares_our_own_live_servers() {
        // Our app_pid is 42; the row is ours, so it must never be reaped even
        // though the "owner_alive" probe would say the owner is dead.
        let entries = [Entry {
            app_pid: 42,
            server_pid: 500,
        }];
        let plan = plan_reap(&entries, 42, |_| false, |_| true);
        assert!(plan.to_kill.is_empty());
        assert_eq!(plan.keep, entries);
    }

    #[test]
    fn spares_another_live_instances_servers() {
        // Owner 99 is a different, still-alive HalluScribe instance.
        let entries = [Entry {
            app_pid: 99,
            server_pid: 500,
        }];
        let plan = plan_reap(&entries, 42, |pid| pid == 99, |_| true);
        assert!(plan.to_kill.is_empty());
        assert_eq!(plan.keep, entries);
    }

    #[test]
    fn reaps_orphan_of_dead_owner_that_is_still_llama() {
        let entries = [Entry {
            app_pid: 7,
            server_pid: 500,
        }];
        let plan = plan_reap(&entries, 42, |_| false, |_| true);
        assert_eq!(plan.to_kill, vec![500]);
        assert!(plan.keep.is_empty());
    }

    #[test]
    fn does_not_kill_reused_pid_that_is_not_llama() {
        // Owner dead, but the server PID now belongs to some other process.
        let entries = [Entry {
            app_pid: 7,
            server_pid: 500,
        }];
        let plan = plan_reap(&entries, 42, |_| false, |_| false);
        assert!(plan.to_kill.is_empty());
        // Stale row is dropped (not kept) since the orphan is already gone.
        assert!(plan.keep.is_empty());
    }
}
