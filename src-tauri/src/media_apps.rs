//! Which media apps may drive the media card. Apps are remembered by their source app
//! user model ID; the switched-off ones are stored in Tuck's settings directory.
#![cfg_attr(not(windows), allow(dead_code))]
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Manager, WebviewWindow};

/// Most apps remembered at once. Past this, the oldest allowed app is forgotten first.
pub const MAX_APPS: usize = 32;
const FILE_NAME: &str = "media-apps.json";
const MAX_FILE_BYTES: u64 = 64 * 1024;

fn default_true() -> bool {
    true
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
struct StoredApp {
    id: String,
    #[serde(default = "default_true")]
    follow: bool,
}

#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
struct StoreFile {
    #[serde(default)]
    apps: Vec<StoredApp>,
}

/// One row in Settings > Media and lyrics.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaApp {
    pub id: String,
    pub name: String,
    pub follow: bool,
}

struct State {
    path: Option<PathBuf>,
    apps: Vec<StoredApp>,
}

static STATE: Mutex<State> = Mutex::new(State {
    path: None,
    apps: Vec::new(),
});

/// Loads the remembered apps from the settings directory. A missing or unreadable file
/// means no apps yet, so every app follows the card.
pub fn init(app: &AppHandle) {
    let path = app
        .path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join(FILE_NAME));
    let apps = path.as_deref().map(load).unwrap_or_default();
    if let Ok(mut state) = STATE.lock() {
        state.path = path;
        state.apps = apps;
    }
}

fn load(path: &Path) -> Vec<StoredApp> {
    use std::io::Read;
    let Ok(file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let mut bytes = Vec::new();
    if file
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() as u64 > MAX_FILE_BYTES
    {
        return Vec::new();
    }
    serde_json::from_slice::<StoreFile>(&bytes)
        .map(|file| file.apps)
        .unwrap_or_default()
}

fn save(path: &Path, apps: &[StoredApp]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|_| "Could not create settings directory")?;
    }
    let bytes = serde_json::to_vec(&StoreFile {
        apps: apps.to_vec(),
    })
    .map_err(|_| "Could not encode media apps")?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, bytes)
        .and_then(|_| std::fs::rename(&temporary, path))
        .map_err(|_| "Could not save media apps".to_string())
}

/// Records apps the media service has seen, oldest first. Writes only when a new app appears.
pub fn remember(ids: &[&str]) {
    let Ok(mut state) = STATE.lock() else {
        return;
    };
    let mut changed = false;
    for id in ids.iter().map(|id| id.trim()).filter(|id| !id.is_empty()) {
        changed |= remember_in(&mut state.apps, id);
    }
    if changed {
        if let Some(path) = state.path.clone() {
            if let Err(error) = save(&path, &state.apps) {
                eprintln!("Media apps: {error}");
            }
        }
    }
}

/// App IDs that must not drive the card.
pub fn off_apps() -> Vec<String> {
    STATE
        .lock()
        .map(|state| off_in(&state.apps))
        .unwrap_or_default()
}

/// Every remembered app, oldest first.
pub fn snapshot() -> Vec<MediaApp> {
    STATE
        .lock()
        .map(|state| snapshot_of(&state.apps))
        .unwrap_or_default()
}

#[tauri::command]
pub fn media_apps_list() -> Vec<MediaApp> {
    snapshot()
}

/// Switches one remembered app on or off for the media card. Settings window only.
#[tauri::command]
pub fn media_app_set_follow(
    window: WebviewWindow,
    id: String,
    follow: bool,
) -> Result<Vec<MediaApp>, String> {
    if window.label() != "settings" {
        return Err("Change media apps in settings".into());
    }
    let mut state = STATE
        .lock()
        .map_err(|_| "Media apps are unavailable".to_string())?;
    if !set_follow_in(&mut state.apps, &id, follow) {
        return Err("This app has not been seen yet".into());
    }
    if let Some(path) = state.path.clone() {
        save(&path, &state.apps)?;
    }
    let apps = snapshot_of(&state.apps);
    drop(state);
    crate::media::follow_changed();
    Ok(apps)
}

/// Readable name from a source app user model ID: `Spotify.exe` becomes `Spotify`, and a
/// packaged `Family_version!App` uses its app part (`MSEdge`).
pub fn display_name(id: &str) -> String {
    let tail = id.rsplit('!').next().unwrap_or(id);
    let stem = if tail.to_ascii_lowercase().ends_with(".exe") {
        &tail[..tail.len() - 4]
    } else {
        tail
    };
    let segment = stem
        .rsplit('.')
        .next()
        .filter(|segment| !segment.is_empty())
        .unwrap_or(stem);
    let mut chars = segment.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => id.to_string(),
    }
}

/// Adds an app as allowed, forgetting the oldest one when full. A forgotten app is one that
/// follows the card, so a switched-off app keeps its setting until the list is all off.
fn remember_in(apps: &mut Vec<StoredApp>, id: &str) -> bool {
    if apps.iter().any(|app| app.id.eq_ignore_ascii_case(id)) {
        return false;
    }
    if apps.len() >= MAX_APPS {
        let index = apps.iter().position(|app| app.follow).unwrap_or(0);
        apps.remove(index);
    }
    apps.push(StoredApp {
        id: id.to_string(),
        follow: true,
    });
    true
}

fn set_follow_in(apps: &mut [StoredApp], id: &str, follow: bool) -> bool {
    match apps.iter_mut().find(|app| app.id.eq_ignore_ascii_case(id)) {
        Some(app) => {
            app.follow = follow;
            true
        }
        None => false,
    }
}

fn off_in(apps: &[StoredApp]) -> Vec<String> {
    apps.iter()
        .filter(|app| !app.follow)
        .map(|app| app.id.clone())
        .collect()
}

fn snapshot_of(apps: &[StoredApp]) -> Vec<MediaApp> {
    apps.iter()
        .map(|app| MediaApp {
            id: app.id.clone(),
            name: display_name(&app.id),
            follow: app.follow,
        })
        .collect()
}

/// Picks the session that drives the card, controls and keys. `preferred` is the pinned or
/// OS-current session. A switched-off app is replaced by the next allowed session in list
/// order, wrapping around; `None` means nothing may drive the card.
pub fn choose_session(apps: &[&str], preferred: Option<usize>, off: &[String]) -> Option<usize> {
    let len = apps.len();
    let allowed = |index: usize| !off.iter().any(|id| id.eq_ignore_ascii_case(apps[index]));
    let preferred = preferred.filter(|index| *index < len);
    if let Some(index) = preferred {
        if allowed(index) {
            return Some(index);
        }
    }
    let start = preferred.map_or(0, |index| index + 1);
    (0..len)
        .map(|offset| (start + offset) % len)
        .find(|index| allowed(*index))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored(ids: &[(&str, bool)]) -> Vec<StoredApp> {
        ids.iter()
            .map(|(id, follow)| StoredApp {
                id: id.to_string(),
                follow: *follow,
            })
            .collect()
    }

    fn off(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    #[test]
    fn keeps_the_pinned_or_os_session_when_its_app_may_drive_the_card() {
        let apps = ["Spotify.exe", "msedge.exe"];
        assert_eq!(choose_session(&apps, Some(1), &[]), Some(1));
        assert_eq!(
            choose_session(&apps, Some(0), &off(&["msedge.exe"])),
            Some(0)
        );
    }

    #[test]
    fn switched_off_session_hands_over_to_the_next_allowed_one() {
        let apps = ["chrome.exe", "Spotify.exe", "msedge.exe"];
        assert_eq!(
            choose_session(&apps, Some(0), &off(&["chrome.exe"])),
            Some(1)
        );
        // Wraps past the end of the list
        assert_eq!(
            choose_session(&apps, Some(2), &off(&["msedge.exe"])),
            Some(0)
        );
        // Skips other switched-off apps on the way
        assert_eq!(
            choose_session(&apps, Some(0), &off(&["chrome.exe", "Spotify.exe"])),
            Some(2)
        );
    }

    #[test]
    fn nothing_plays_when_every_session_is_switched_off() {
        let apps = ["chrome.exe", "Spotify.exe"];
        assert_eq!(
            choose_session(&apps, Some(0), &off(&["chrome.exe", "Spotify.exe"])),
            None
        );
        assert_eq!(choose_session(&[], None, &[]), None);
        assert_eq!(choose_session(&[], Some(0), &off(&[])), None);
    }

    #[test]
    fn without_an_os_session_the_first_allowed_one_is_chosen() {
        let apps = ["chrome.exe", "Spotify.exe"];
        assert_eq!(choose_session(&apps, None, &[]), Some(0));
        assert_eq!(choose_session(&apps, None, &off(&["chrome.exe"])), Some(1));
        assert_eq!(choose_session(&apps, Some(9), &[]), Some(0));
    }

    #[test]
    fn app_ids_match_regardless_of_case() {
        let apps = ["Spotify.exe", "msedge.exe"];
        assert_eq!(
            choose_session(&apps, Some(0), &off(&["spotify.EXE"])),
            Some(1)
        );
    }

    #[test]
    fn display_names_come_from_the_app_id() {
        assert_eq!(display_name("Spotify.exe"), "Spotify");
        assert_eq!(display_name("chrome.exe"), "Chrome");
        assert_eq!(
            display_name("Microsoft.MicrosoftEdge.Stable_8wekyb3d8bbwe!MSEdge"),
            "MSEdge"
        );
        assert_eq!(
            display_name("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"),
            "ZuneMusic"
        );
        assert_eq!(display_name("!"), "!");
    }

    #[test]
    fn remembers_each_app_once_in_first_seen_order() {
        let mut apps = Vec::new();
        assert!(remember_in(&mut apps, "Spotify.exe"));
        assert!(remember_in(&mut apps, "chrome.exe"));
        assert!(!remember_in(&mut apps, "spotify.exe"));
        assert_eq!(
            apps.iter().map(|app| app.id.as_str()).collect::<Vec<_>>(),
            ["Spotify.exe", "chrome.exe"]
        );
        assert!(apps.iter().all(|app| app.follow));
    }

    #[test]
    fn full_list_forgets_the_oldest_allowed_app_first() {
        let mut apps = stored(&[("off.exe", false)]);
        for index in 1..MAX_APPS {
            apps.push(StoredApp {
                id: format!("app{index}.exe"),
                follow: true,
            });
        }
        assert_eq!(apps.len(), MAX_APPS);
        assert!(remember_in(&mut apps, "new.exe"));
        assert_eq!(apps.len(), MAX_APPS);
        assert_eq!(apps[0].id, "off.exe", "a switched-off app is kept");
        assert_eq!(apps[1].id, "app2.exe", "the oldest allowed app goes");
        assert_eq!(apps.last().unwrap().id, "new.exe");
    }

    #[test]
    fn full_list_of_switched_off_apps_forgets_the_oldest() {
        let mut apps: Vec<StoredApp> = (0..MAX_APPS)
            .map(|index| StoredApp {
                id: format!("off{index}.exe"),
                follow: false,
            })
            .collect();
        assert!(remember_in(&mut apps, "new.exe"));
        assert_eq!(apps.len(), MAX_APPS);
        assert_eq!(apps[0].id, "off1.exe");
    }

    #[test]
    fn switching_an_unknown_app_fails_without_changing_the_list() {
        let mut apps = stored(&[("Spotify.exe", true)]);
        assert!(!set_follow_in(&mut apps, "chrome.exe", false));
        assert!(set_follow_in(&mut apps, "spotify.exe", false));
        assert_eq!(off_in(&apps), off(&["Spotify.exe"]));
        assert_eq!(snapshot_of(&apps)[0].name, "Spotify");
        assert!(!snapshot_of(&apps)[0].follow);
    }

    #[test]
    fn saved_file_round_trips_and_missing_fields_default_to_following() {
        let file: StoreFile = serde_json::from_str(r#"{"apps":[{"id":"chrome.exe"}]}"#).unwrap();
        assert!(file.apps[0].follow);
        let empty: StoreFile = serde_json::from_str("{}").unwrap();
        assert!(empty.apps.is_empty());
        let json = serde_json::to_string(&StoreFile {
            apps: stored(&[("chrome.exe", false)]),
        })
        .unwrap();
        assert_eq!(json, r#"{"apps":[{"id":"chrome.exe","follow":false}]}"#);
    }

    #[test]
    fn media_app_rows_use_camel_case() {
        let json = serde_json::to_value(MediaApp {
            id: "Spotify.exe".into(),
            name: "Spotify".into(),
            follow: true,
        })
        .unwrap();
        assert_eq!(json["follow"], true);
        assert_eq!(json["name"], "Spotify");
    }
}
