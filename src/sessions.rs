//! Discover installed desktop sessions from the freedesktop session dirs.

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct SessionEntry {
    pub name: String,
    pub exec: String,
    pub wayland: bool,
    pub desktop_names: String,
}

const WAYLAND_DIR: &str = "/usr/share/wayland-sessions";
const X_DIR: &str = "/usr/share/xsessions";

/// A minimal .desktop parser: we only need `Name`, `Exec`, `DesktopNames`,
/// and we skip entries with `Hidden=true` or `NoDisplay=true`.
fn parse_desktop(path: &Path, wayland: bool) -> Option<SessionEntry> {
    let text = fs::read_to_string(path).ok()?;
    if !text.starts_with("[Desktop Entry]") && !text.contains("\n[Desktop Entry]") {
        return None;
    }
    let mut name = None;
    let mut exec = None;
    let mut desktop_names = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "Name" => name = Some(value.trim().to_string()),
            "Exec" => exec = Some(value.trim().to_string()),
            "DesktopNames" => desktop_names = value.trim().to_string(),
            "Hidden" | "NoDisplay" if value.trim().eq_ignore_ascii_case("true") => return None,
            _ => {}
        }
    }
    // Exec must not contain field codes (%c, %f, ...) that duxdm would have
    // to expand — pass it through as-is, like other minimal launchers do.
    Some(SessionEntry {
        name: name.unwrap_or_else(|| {
            path.file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "session".to_string())
        }),
        exec: exec?,
        wayland,
        desktop_names,
    })
}

fn read_dir_sessions(dir: &str, wayland: bool) -> Vec<SessionEntry> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut entries: Vec<SessionEntry> = rd
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if p.extension().map(|x| x == "desktop").unwrap_or(false) {
                parse_desktop(&p, wayland)
            } else {
                None
            }
        })
        .collect();
    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    entries
}

/// Allow overriding the session directories via environment variables
/// (used by the test suite; defaults to the freedesktop locations).
fn dir_or_env(default: &'static str, var: &'static str) -> String {
    std::env::var(var).unwrap_or_else(|_| default.to_string())
}

/// All available sessions, Wayland first, then X11, both alphabetically.
pub fn discover() -> Vec<SessionEntry> {
    let mut out = read_dir_sessions(
        &dir_or_env(WAYLAND_DIR, "DUXDM_WAYLAND_SESSIONS"),
        true,
    );
    out.extend(read_dir_sessions(
        &dir_or_env(X_DIR, "DUXDM_XSESSIONS"),
        false,
    ));
    out
}

/// Where the X11 session's temporary xinitrc lives.
pub fn xinitrc_path() -> PathBuf {
    PathBuf::from("/tmp/duxdm-xinitrc")
}
