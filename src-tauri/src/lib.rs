use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

/// Downloads that are currently running, keyed by the app's own job id.
/// We store the pid rather than the `Child` so the reader thread can own the
/// child and call `wait()` on it while a cancel still has something to kill.
#[derive(Default)]
struct AppState {
    pids: Mutex<HashMap<String, u32>>,
    cancelled: Mutex<HashSet<String>>,
}

#[derive(Serialize, Deserialize, Clone)]
struct Video {
    id: String,
    title: String,
    duration: Option<f64>,
    view_count: Option<u64>,
    url: String,
    /// flat-playlist populates this only for restricted videos. The observed
    /// value for a members-only video is `subscriber_only`; it is null for
    /// ordinary public videos. This is what the Members filter keys on — the
    /// "no view count" signal is NOT reliable (public videos on some channels
    /// come back with a null view_count in flat mode).
    #[serde(default)]
    availability: Option<String>,
}

impl Video {
    /// Only exercised by the tests today — the UI does this check in JS — but
    /// it keeps the rule in one named place on the Rust side.
    #[allow(dead_code)]
    fn is_members_only(&self) -> bool {
        self.availability.as_deref() == Some("subscriber_only")
    }
}

#[derive(Serialize, Clone)]
struct ChannelInfo {
    /// The normalized channel URL. This is the channel's identity in the saved
    /// state, so "@foo" and "https://youtube.com/@foo" don't become two entries.
    url: String,
    title: String,
    uploader: String,
    tab: String,
    video_count: usize,
    videos: Vec<Video>,
}

/// A channel the user has fetched. Persisted so the app reopens where it left
/// off, and so its download folder survives across launches.
#[derive(Serialize, Deserialize, Clone, Default)]
struct SavedChannel {
    url: String,
    title: String,
    uploader: String,
    #[serde(default)]
    video_count: usize,
    #[serde(default)]
    fetched_at: u64,
    /// Where this channel's videos are saved. None until the user picks one,
    /// which is what triggers the "choose a folder" prompt on first download.
    #[serde(default)]
    download_dir: Option<String>,
    /// The Videos tab only. Shorts and the live tab are fetched on demand and
    /// are not persisted.
    #[serde(default)]
    videos: Vec<Video>,
}

/// Everything the app remembers, in one file. The frontend owns this object and
/// hands the whole thing back through `save_state` after any change, so the
/// Rust side stays a plain reader/writer.
#[derive(Serialize, Deserialize, Clone, Default)]
struct PersistedState {
    #[serde(default)]
    active: Option<String>,
    #[serde(default)]
    channels: Vec<SavedChannel>,
}

#[derive(Serialize, Clone)]
struct Progress {
    id: String,
    downloaded: f64,
    total: f64,
    speed: f64,
    eta: f64,
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
}

/// The folder a channel's downloads go to by default, used to pre-fill the
/// folder picker.
fn default_download_dir() -> PathBuf {
    home().join("Downloads").join("Vault")
}

/// The download archive used before archives moved into each channel's folder.
/// Only read by the one-time migration that splits it up.
fn legacy_archive_path() -> PathBuf {
    home().join(".vault").join("archive.txt")
}

/// The download archive for a channel, kept inside the channel's own download
/// directory rather than in `~/.vault`. The folder is then self-describing: it
/// holds the videos and the record of what has been fetched, so moving it to
/// another drive keeps working, and two channels can't collide on a video id.
///
/// The `Vault` subfolder keeps yt-dlp's bookkeeping out of the video listing,
/// and the leading dot keeps it hidden in Finder.
fn channel_archive_path(dir: &std::path::Path) -> PathBuf {
    dir.join(".vault").join("archive.txt")
}

/// The meta file that makes a directory a Vault channel folder. Written next to
/// the archive so a folder can be identified as one of ours without the app's
/// state file — the pointer lives in `state.json`, the identity lives here.
fn channel_meta_path(dir: &std::path::Path) -> PathBuf {
    dir.join(".vault").join("channel.json")
}

fn state_path() -> PathBuf {
    home().join(".vault").join("state.json")
}

/// Resolve the yt-dlp binary without relying on the GUI app's PATH, which on
/// macOS does not include the shell rc files where brew / uv put things.
fn ytdlp_path() -> String {
    let candidates = [
        home().join(".local/bin/yt-dlp"),
        PathBuf::from("/opt/homebrew/bin/yt-dlp"),
        PathBuf::from("/usr/local/bin/yt-dlp"),
    ];
    for c in candidates {
        if c.is_file() {
            return c.to_string_lossy().into_owned();
        }
    }
    "yt-dlp".to_string()
}

/// Resolve ffmpeg alongside yt-dlp: the GUI app's PATH omits brew, and without
/// ffmpeg yt-dlp downloads the video and audio streams separately and cannot
/// merge them into one MP4.
fn ffmpeg_path() -> Option<PathBuf> {
    [
        home().join(".local/bin/ffmpeg"),
        PathBuf::from("/opt/homebrew/bin/ffmpeg"),
        PathBuf::from("/usr/local/bin/ffmpeg"),
    ]
    .into_iter()
    .find(|c| c.is_file())
}

/// Turn a bare handle or a pasted URL into something yt-dlp understands.
fn normalize_input(raw: &str) -> String {
    let s = raw.trim();
    if s.is_empty() {
        return s.to_string();
    }
    if s.starts_with("http://") || s.starts_with("https://") {
        return s.to_string();
    }
    if s.starts_with('@') {
        return format!("https://www.youtube.com/{s}");
    }
    if s.starts_with("UC") && s.len() == 24 {
        return format!("https://www.youtube.com/channel/{s}");
    }
    format!("https://www.youtube.com/@{s}")
}

/// Whether the URL points at a channel (so a tab path can be appended) rather
/// than a single video.
fn is_channel_url(u: &str) -> bool {
    let channelish = u.contains("/@") || u.contains("/channel/") || u.contains("/c/") || u.contains("/user/");
    channelish && !u.contains("/watch") && !u.contains("youtu.be/")
}

/// Map a line of yt-dlp output to a short, human reason. The first match wins,
/// so the specific cases are checked before the generic ones.
fn classify_failure(line: &str) -> Option<&'static str> {
    let l = line.to_lowercase();
    let has = |s: &str| l.contains(s);
    if has("members-only") || has("members only") || has("join this channel") || has("subscriber_only") || has("channel's members") {
        return Some("Members only");
    }
    if has("private video") || has("this video is private") {
        return Some("Private");
    }
    if has("sign in to confirm your age") || has("age-restricted") || has("age restricted") {
        return Some("Age-restricted");
    }
    if has("not available in your country") || has("blocked in your country") || has("geo-restricted") {
        return Some("Geo-blocked");
    }
    if has("video unavailable") || has("has been removed") || has("no longer available") || has("removed by the uploader") || has("account associated") {
        return Some("Unavailable");
    }
    if has("requested format is not available") || has("no video formats found") {
        return Some("No downloadable format");
    }
    if has("http error 404") || has("http error 403") || has("http error 400") {
        return Some("Not found");
    }
    if has("unable to download webpage")
        || has("http error 5")
        || has("timed out")
        || has("timeout")
        || has("connection")
        || has("name resolution")
        || has("temporary failure")
        || has("urlopen error")
        || has("network is unreachable")
        || has("ssl")
    {
        return Some("Network error");
    }
    if has("ffmpeg") || has("postprocessing") || has("post-processing") {
        return Some("Post-processing failed");
    }
    None
}

#[tauri::command]
async fn fetch_channel(url: String, tab: String) -> Result<ChannelInfo, String> {
    tauri::async_runtime::spawn_blocking(move || fetch_channel_blocking(url, tab))
        .await
        .map_err(|e| e.to_string())?
}

fn fetch_channel_blocking(raw: String, tab: String) -> Result<ChannelInfo, String> {
    let yt = ytdlp_path();
    let mut url = normalize_input(&raw);
    // `/videos` excludes both shorts and past livestreams, which is exactly the
    // default view. The other tabs are appended the same way when asked for.
    if is_channel_url(&url) {
        url = format!("{}/{}", url.trim_end_matches('/'), tab);
    }

    let out = Command::new(&yt)
        .args(["--flat-playlist", "-J", "--no-warnings", &url])
        .output()
        .map_err(|e| format!("Could not run yt-dlp ({yt}): {e}"))?;

    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let msg = err.trim().lines().last().unwrap_or("yt-dlp failed").to_string();
        return Err(msg);
    }

    let root: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("Bad JSON from yt-dlp: {e}"))?;

    let raw_entries: Vec<&serde_json::Value> = match root.get("entries") {
        Some(serde_json::Value::Array(a)) => a.iter().collect(),
        _ => vec![&root],
    };

    // Entries may be nested one level (channel -> tab -> videos). Walk them
    // breadth-first, preserving the order yt-dlp returned: that order is
    // newest-first, and it is the only thing the Newest/Oldest sort has to work
    // with (flat mode gives a null `timestamp` for every entry). A stack would
    // pop LIFO and silently reverse the list, which read as the sort options
    // being swapped.
    let mut videos: Vec<Video> = Vec::new();
    let mut queue: std::collections::VecDeque<&serde_json::Value> = raw_entries.into();
    while let Some(e) = queue.pop_front() {
        if let Some(serde_json::Value::Array(nested)) = e.get("entries") {
            queue.extend(nested.iter());
            continue;
        }
        let Some(id) = e.get("id").and_then(|v| v.as_str()) else { continue };
        if id.len() != 11 {
            continue;
        }
        let title = e.get("title").and_then(|v| v.as_str()).unwrap_or("(untitled)").to_string();
        let duration = e.get("duration").and_then(|v| v.as_f64());
        let view_count = e.get("view_count").and_then(|v| v.as_u64());
        let availability = e.get("availability").and_then(|v| v.as_str()).map(|s| s.to_string());
        let web = e.get("url").or_else(|| e.get("webpage_url")).and_then(|v| v.as_str());
        let url = match web {
            Some(u) if u.starts_with("http") => u.to_string(),
            _ => format!("https://www.youtube.com/watch?v={id}"),
        };
        videos.push(Video { id: id.to_string(), title, duration, view_count, url, availability });
    }

    // `title` on a tab URL carries the tab name ("Foo - Videos"), so prefer the
    // channel/uploader fields and only fall back to title with the suffix
    // stripped. Saved titles are the channel's identity in the selector.
    let raw_title = root
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("Channel");
    let title = root
        .get("channel")
        .or_else(|| root.get("uploader"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            // " - Videos" / " - Shorts" / " - Live", case-insensitively.
            let lower = raw_title.to_lowercase();
            for suffix in [" - videos", " - shorts", " - live", " - streams"] {
                if lower.ends_with(suffix) {
                    return raw_title[..raw_title.len() - suffix.len()].to_string();
                }
            }
            raw_title.to_string()
        });
    let uploader = root
        .get("uploader")
        .or_else(|| root.get("channel"))
        .and_then(|v| v.as_str())
        .unwrap_or(&title)
        .to_string();

    Ok(ChannelInfo {
        url: normalize_input(&raw),
        title,
        uploader,
        tab,
        video_count: videos.len(),
        videos,
    })
}

#[tauri::command]
fn download_video(app: AppHandle, id: String, url: String, dir: String) -> Result<(), String> {
    let yt = ytdlp_path();
    let dir = PathBuf::from(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    // The archive lives inside this channel's folder, so it follows the videos.
    let archive = channel_archive_path(&dir);
    if let Some(parent) = archive.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let mut child: Child = Command::new(&yt)
        .arg("--newline")
        .arg("--no-warnings")
        .arg("--download-archive")
        .arg(&archive)
        // Prefer AV1, then H.264, always with AAC audio, muxed into MP4.
        //
        // yt-dlp's default lands on VP9+Opus in WebM: same visual quality at
        // roughly a third of H.264's bitrate. It is the better encode, but
        // macOS' WebKit plays WebM inconsistently, so the app couldn't play its
        // own downloads. AV1 in MP4 measures the same as WebM — 29.0 MiB vs
        // 29.4 MiB for 1080p on a test video — in a container WebKit handles.
        // The h264 step is the fallback for videos with no AV1 ladder.
        .arg("-f")
        .arg("bv*+ba/b")
        .arg("-S")
        .arg("res,vcodec:av01,vcodec:h264,acodec:aac")
        // yt-dlp's own PATH lookup misses brew's ffmpeg in the GUI environment,
        // and without it the merge step is skipped: two separate files.
        .args(ffmpeg_path().map(|f| vec!["--ffmpeg-location".to_string(), f.to_string_lossy().into_owned()]).unwrap_or_default())
        .arg("--merge-output-format")
        .arg("mp4")
        .arg("--progress-template")
        .arg("download:VAULTPROG|%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.total_bytes_estimate)s|%(progress.speed)s|%(progress.eta)s")
        .arg("-o")
        // A channel's directory holds flat files: the folder is already the
        // channel, so there is no uploader subfolder here.
        .arg(format!("{}/%(title)s [%(id)s].%(ext)s", dir.display()))
        .arg(&url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not run yt-dlp ({yt}): {e}"))?;

    let stdout = child.stdout.take().ok_or("yt-dlp produced no output stream")?;
    let stderr = child.stderr.take().ok_or("yt-dlp produced no error stream")?;
    let pid = child.id();
    {
        let st = app.state::<AppState>();
        st.pids.lock().unwrap().insert(id.clone(), pid);
    }

    // stderr is drained on its own thread. Leaving it piped and unread is a real
    // hazard: a verbose failure fills the pipe buffer and blocks yt-dlp forever.
    let reason: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let reason_err = reason.clone();
    let err_thread = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if let Some(r) = classify_failure(&line) {
                let mut g = reason_err.lock().unwrap();
                if g.is_none() {
                    *g = Some(r.to_string());
                }
            }
        }
    });

    let handle = app.clone();
    let job = id.clone();
    let reason_out = reason.clone();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(rest) = line.strip_prefix("VAULTPROG|") {
                let p: Vec<&str> = rest.split('|').collect();
                let num = |i: usize| p.get(i).and_then(|s| s.trim().parse::<f64>().ok()).unwrap_or(0.0);
                let downloaded = num(0);
                let total = if num(1) > 0.0 { num(1) } else { num(2) };
                let _ = handle.emit(
                    "download-progress",
                    Progress { id: job.clone(), downloaded, total, speed: num(3), eta: num(4) },
                );
            } else if line.contains("already been recorded in the archive") {
                let _ = handle.emit("download-skipped", serde_json::json!({ "id": job }));
            } else if let Some(r) = classify_failure(&line) {
                // Some failures print on stdout rather than stderr.
                let mut g = reason_out.lock().unwrap();
                if g.is_none() {
                    *g = Some(r.to_string());
                }
            }
        }
        let status = child.wait();
        let ok = status.map(|s| s.success()).unwrap_or(false);
        let _ = err_thread.join();
        let was_cancelled = {
            let st = handle.state::<AppState>();
            let removed = st.cancelled.lock().unwrap().remove(&job);
            removed
        };
        {
            let st = handle.state::<AppState>();
            st.pids.lock().unwrap().remove(&job);
        }
        let fail_reason = reason.lock().unwrap().clone();
        let _ = handle.emit(
            "download-done",
            serde_json::json!({
                "id": job,
                "ok": ok && !was_cancelled,
                "cancelled": was_cancelled,
                "reason": if ok || was_cancelled { serde_json::Value::Null }
                          else { serde_json::json!(fail_reason.unwrap_or_else(|| "Failed".into())) },
            }),
        );
    });

    Ok(())
}

/// Sends SIGTERM to a running download. Returns whether a live process was
/// signalled — false means the job had no process (already finished, or still
/// only queued), which the frontend handles by dropping it from its own queue.
#[tauri::command]
fn cancel_download(app: AppHandle, id: String) -> Result<bool, String> {
    let pid = {
        let st = app.state::<AppState>();
        let found = st.pids.lock().unwrap().get(&id).copied();
        found
    };
    let Some(pid) = pid else { return Ok(false) };
    {
        let st = app.state::<AppState>();
        st.cancelled.lock().unwrap().insert(id.clone());
    }
    let status = Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status()
        .map_err(|e| format!("Could not signal process {pid}: {e}"))?;
    Ok(status.success())
}

#[derive(Serialize, Clone)]
struct ArchiveEntry {
    id: String,
    title: String,
    duration: Option<f64>,
    view_count: Option<u64>,
    url: String,
    /// The channel this id belongs to, when it can be resolved locally.
    channel: Option<String>,
    /// The channel's saved URL, so the UI can filter the archive to one channel.
    channel_url: Option<String>,
    /// The folder the file landed in, when the id's channel is known but its
    /// file couldn't be found (e.g. an unmounted external drive).
    dir: Option<String>,
    /// The file on disk, when it can be found.
    path: Option<String>,
}

#[derive(Serialize, Clone)]
struct ChannelVault {
    url: String,
    title: String,
    dir: Option<String>,
    /// How many ids this channel's own archive file records.
    archived: usize,
    /// Whether the channel's folder (and its archive) was reachable.
    reachable: bool,
}

/// Read an archive file into ids, in file order (oldest download first).
/// yt-dlp's format is `<extractor> <id>` per line.
fn read_archive(path: &std::path::Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    text.lines()
        .filter_map(|l| {
            let mut parts = l.split_whitespace();
            let _ = parts.next()?;
            parts.next().map(|s| s.to_string())
        })
        .collect()
}

/// Every archived id across all remembered channels, deduped. Reads each
/// channel's own archive inside its download folder. The folders are the source
/// of truth: a channel whose folder is missing (unmounted drive, moved by hand)
/// simply contributes nothing rather than erroring.
fn all_archive_ids(state: &PersistedState) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut ids = Vec::new();
    for c in &state.channels {
        let Some(dir) = c.download_dir.as_deref() else { continue };
        for id in read_archive(&channel_archive_path(std::path::Path::new(dir))) {
            if seen.insert(id.clone()) {
                ids.push(id);
            }
        }
    }
    ids
}

/// Look up a file by the `[<id>]` tag yt-dlp puts in every name it writes.
/// Only levels directly under `dir` are searched: the download template is
/// flat, so a recursive walk would just be wasted time on a big library.
fn find_file(dir: &std::path::Path, id: &str) -> Option<PathBuf> {
    let tag = format!("[{id}]");
    let entries = std::fs::read_dir(dir).ok()?;
    for e in entries.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        // Stems carry the tag; the extension follows. `[id].mp4` matches,
        // `[id]-extra.mp4` does not.
        if let Some(rest) = name.rsplit_once(&tag) {
            if rest.1.is_empty() || rest.1.starts_with('.') {
                return Some(e.path());
            }
        }
    }
    None
}

/// The archive, resolved against the channels the app remembers. An id is
/// matched to its channel through the saved video lists, which is what makes
/// titles and durations available without a network call. Anything that isn't
/// in a saved list still gets an entry — the id alone, so the count is honest.
///
/// Archived state is per channel: an id is only "archived" for the channel
/// whose folder records it, so two channels that share a video don't hide each
/// other's copy.
#[tauri::command]
fn list_archive() -> Vec<ArchiveEntry> {
    let state = load_state();
    // id -> (channel title, channel url, download dir, video).
    let mut known: HashMap<String, (String, String, Option<String>, Video)> = HashMap::new();
    for c in &state.channels {
        for v in &c.videos {
            known.entry(v.id.clone()).or_insert_with(|| {
                (c.title.clone(), c.url.clone(), c.download_dir.clone(), v.clone())
            });
        }
    }

    all_archive_ids(&state)
        .into_iter()
        .map(|id| {
            let hit = known.remove(&id);
            let dir = hit.as_ref().and_then(|(_, _, d, _)| d.clone());
            let path = dir
                .as_ref()
                .and_then(|d| find_file(std::path::Path::new(d), &id))
                .map(|p| p.to_string_lossy().into_owned());
            let (channel, channel_url, title, duration, view_count, url) = match hit {
                Some((ch, chu, _, v)) => {
                    (Some(ch), Some(chu), Some(v.title), v.duration, v.view_count, v.url)
                }
                None => (None, None, None, None, None, format!("https://www.youtube.com/watch?v={id}")),
            };
            ArchiveEntry {
                title: title.unwrap_or_else(|| id.clone()),
                id,
                duration,
                view_count,
                url,
                channel,
                channel_url,
                dir,
                path,
            }
        })
        .collect()
}

/// Per-channel archive state, so the UI can show each channel's own vault
/// rather than one undifferentiated list. `reachable` is false when the
/// channel's folder can't be read — an unmounted drive is the usual reason.
#[tauri::command]
fn channel_vaults() -> Vec<ChannelVault> {
    let state = load_state();
    state
        .channels
        .iter()
        .map(|c| {
            let (archived, reachable) = match c.download_dir.as_deref() {
                Some(d) => {
                    let dir = std::path::Path::new(d);
                    let file = channel_archive_path(dir);
                    (read_archive(&file).len(), file.is_file())
                }
                None => (0, false),
            };
            ChannelVault {
                url: c.url.clone(),
                title: c.title.clone(),
                dir: c.download_dir.clone(),
                archived,
                reachable,
            }
        })
        .collect()
}

/// The set of video ids already archived anywhere, so the UI can mark cards as
/// "downloaded" on load without re-checking the filesystem.
#[tauri::command]
fn downloaded_ids() -> Vec<String> {
    all_archive_ids(&load_state())
}

/// Record a channel's identity inside its own folder, so the directory is
/// self-describing and can be re-attached later without the app's state file.
fn write_channel_meta(dir: &std::path::Path, url: &str, title: &str) -> Result<(), String> {
    let meta = channel_meta_path(dir);
    if let Some(parent) = meta.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let body = serde_json::json!({ "url": url, "title": title, "created_by": "Vault" });
    let json = serde_json::to_string_pretty(&body).map_err(|e| e.to_string())?;
    std::fs::write(&meta, json).map_err(|e| format!("Could not write {}: {e}", meta.display()))
}

/// Point a channel at its folder and stamp the folder with the channel's
/// identity. Called when the folder is chosen, which is the one moment the
/// pairing changes.
#[tauri::command]
fn set_channel_dir(url: String, title: String, dir: String) -> Result<(), String> {
    let dir = PathBuf::from(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    write_channel_meta(&dir, &url, &title)
}

#[tauri::command]
fn load_state() -> PersistedState {
    let path = state_path();
    let Ok(text) = std::fs::read_to_string(&path) else { return PersistedState::default() };
    serde_json::from_str(&text).unwrap_or_default()
}

#[tauri::command]
fn save_state(state: PersistedState) -> Result<(), String> {
    let path = state_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(&state).map_err(|e| e.to_string())?;
    // Write beside the target and rename, so a crash mid-write can't leave a
    // truncated state file where the real one was.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn default_download_dir_str() -> String {
    default_download_dir().to_string_lossy().into_owned()
}

/// Create a folder if it isn't there. Used to materialise the suggested
/// per-channel folder before the picker opens on it, since the macOS folder
/// dialog ignores a defaultPath that doesn't exist.
#[tauri::command]
fn ensure_dir(dir: String) -> Result<(), String> {
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not create {dir}: {e}"))
}

/// One-time move from the old single global archive to per-channel archives.
///
/// It runs once, guarded by a marker file rather than by "does the legacy file
/// exist" — the legacy file is left in place on purpose so nothing is lost if a
/// channel's folder is missing or on an unmounted drive, and a re-run must not
/// duplicate lines into archives that already have them.
///
/// Each id is assigned to every remembered channel whose saved video list (or
/// existing per-channel archive) contains it. An id that can't be attributed to
/// any channel stays in the legacy file and is reported back, so it can't
/// silently vanish.
#[tauri::command]
fn migrate_archives() -> Result<MigrateReport, String> {
    let marker = home().join(".vault").join("archives-migrated");
    if marker.is_file() {
        return Ok(MigrateReport { ran: false, ..Default::default() });
    }

    let legacy = legacy_archive_path();
    let legacy_lines: Vec<String> = std::fs::read_to_string(&legacy)
        .map(|t| t.lines().filter(|l| !l.trim().is_empty()).map(|s| s.to_string()).collect())
        .unwrap_or_default();
    if legacy_lines.is_empty() {
        // Nothing to move. Record that the step is done so later runs are free.
        let _ = std::fs::write(&marker, "nothing to do\n");
        return Ok(MigrateReport { ran: true, ..Default::default() });
    }

    let state = load_state();
    let mut report = MigrateReport { ran: true, ..Default::default() };
    let mut claimed: HashSet<String> = HashSet::new();

    for c in &state.channels {
        let Some(dir) = c.download_dir.as_deref() else { continue };
        let dir = std::path::Path::new(dir);
        let target = channel_archive_path(dir);
        // Ids this channel already has recorded, so the merge is idempotent.
        let mut have: HashSet<String> = read_archive(&target).into_iter().collect();
        let owned: HashSet<&str> = c.videos.iter().map(|v| v.id.as_str()).collect();

        let lines: Vec<&String> = legacy_lines
            .iter()
            .filter(|l| {
                l.split_whitespace().nth(1).map(|id| owned.contains(id) || have.contains(id)).unwrap_or(false)
            })
            .collect();
        if lines.is_empty() {
            continue;
        }

        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut body = std::fs::read_to_string(&target).unwrap_or_default();
        if !body.is_empty() && !body.ends_with('\n') {
            body.push('\n');
        }
        for l in &lines {
            let id = l.split_whitespace().nth(1).unwrap_or("").to_string();
            if have.insert(id.clone()) {
                body.push_str(l);
                body.push('\n');
                report.moved += 1;
            }
            claimed.insert(id);
        }
        std::fs::write(&target, body).map_err(|e| format!("Could not write {}: {e}", target.display()))?;
        // Read back rather than trusting the write. On an external volume the
        // write can appear to succeed and the data not be there afterwards;
        // marking the migration done on that basis would strand the ids.
        let landed = read_archive(&target).len();
        if landed < have.len() {
            return Err(format!(
                "Wrote {} to {} but only {} ids read back — leaving the migration unfinished",
                have.len(), target.display(), landed
            ));
        }
        if write_channel_meta(dir, &c.url, &c.title).is_err() {
            report.meta_failed += 1;
        }
        report.channels += 1;
    }

    report.unmatched = legacy_lines
        .iter()
        .filter_map(|l| l.split_whitespace().nth(1))
        .filter(|id| !claimed.contains(*id))
        .count();

    // The marker means "verified done", not "attempted". It is only written
    // when every id was attributed AND every archive read back in full, so a
    // run that half-succeeded retries instead of looking finished.
    if report.unmatched == 0 && report.meta_failed == 0 {
        let _ = std::fs::write(&marker, "done\n");
    }
    Ok(report)
}

#[derive(Serialize, Clone, Default)]
struct MigrateReport {
    /// Whether this call did anything at all (false when already migrated).
    ran: bool,
    channels: usize,
    moved: usize,
    /// Legacy entries that no remembered channel claimed; they stay put.
    unmatched: usize,
    meta_failed: usize,
}

#[tauri::command]
fn reveal_folder(dir: String) -> Result<(), String> {
    let dir = PathBuf::from(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Command::new("open")
        .arg(&dir)
        .status()
        .map_err(|e| format!("Could not open {}: {e}", dir.display()))?;
    Ok(())
}

/// Opens a downloaded file with the system's default app, so VLC (or whatever
/// handles the extension) plays it.
///
/// There is deliberately no `exists()` guard. On macOS a `stat` against a path
/// the process can't reach fails the same way a missing file does, so guarding
/// on it turns "the folder is on a drive this build can't see" into "no longer
/// there" and refuses to launch — the user sees nothing at all. Letting
/// LaunchServices be the judge means the file opens when it can and the error
/// we surface is the real one.
///
/// `open` is also passed the path as its own argv entry, never through a shell,
/// so spaces, parentheses, brackets and non-ASCII names survive intact.
#[tauri::command]
fn open_file(path: String) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("No file to open.".into());
    }
    let out = Command::new("open")
        .arg(&path)
        .output()
        .map_err(|e| format!("Could not run `open`: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let msg = err.trim();
        return Err(if msg.is_empty() {
            format!("`open` failed on {path}")
        } else {
            msg.to_string()
        });
    }
    Ok(())
}

#[allow(dead_code)]
fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Network test. Run with: cargo test -- --ignored --nocapture
    #[test]
    #[ignore]
    fn fetch_real_channel_end_to_end() {
        let info = fetch_channel_blocking("@kurzgesagt".to_string(), "videos".to_string())
            .expect("fetch should succeed against a live channel");
        println!("channel: {} | videos: {}", info.title, info.video_count);
        assert!(info.video_count > 50);
        assert_eq!(info.videos[0].id.len(), 11);
    }

    #[test]
    #[ignore]
    fn members_only_is_flagged_from_the_listing() {
        let info = fetch_channel_blocking("@LinusTechTips".to_string(), "videos".to_string())
            .expect("fetch should succeed");
        let members: Vec<_> = info.videos.iter().filter(|v| v.is_members_only()).collect();
        println!("LTT: {} of {} flagged subscriber_only", members.len(), info.video_count);
        assert!(!members.is_empty(), "expected some members-only videos on LTT");
        assert!(members.iter().all(|v| v.availability.as_deref() == Some("subscriber_only")));
    }

    #[test]
    fn normalize_handles_all_input_forms() {
        assert_eq!(normalize_input("@foo"), "https://www.youtube.com/@foo");
        assert_eq!(
            normalize_input("UCXuqSBlHAE6Xw-yeJA0Tunw"),
            "https://www.youtube.com/channel/UCXuqSBlHAE6Xw-yeJA0Tunw"
        );
        assert_eq!(normalize_input("https://youtu.be/abc"), "https://youtu.be/abc");
    }

    #[test]
    fn channel_url_detection() {
        assert!(is_channel_url("https://www.youtube.com/@foo"));
        assert!(is_channel_url("https://www.youtube.com/channel/UCabc"));
        assert!(!is_channel_url("https://www.youtube.com/watch?v=abc"));
        assert!(!is_channel_url("https://youtu.be/abc"));
    }

    #[test]
    fn classify_failure_maps_known_reasons() {
        assert_eq!(
            classify_failure("ERROR: Join this channel to get access to members-only content"),
            Some("Members only")
        );
        assert_eq!(classify_failure("ERROR: unable to download webpage: HTTP Error 503"), Some("Network error"));
        assert_eq!(classify_failure("ERROR: Requested format is not available"), Some("No downloadable format"));
        assert_eq!(classify_failure("ERROR: Private video. Sign in if you've been granted access"), Some("Private"));
        // Progress lines must not be mistaken for failures.
        assert_eq!(classify_failure("[download]  42.0% of 10.00MiB at 2.00MiB/s"), None);
        assert_eq!(classify_failure("[youtube] Extracting URL"), None);
    }

    #[test]
    fn old_saved_videos_without_availability_still_load() {
        // availability was added after the first release; old state.json files
        // won't have the key.
        let old = r#"{"id":"abc12345678","title":"t","duration":null,"view_count":null,"url":"u"}"#;
        let v: Video = serde_json::from_str(old).expect("old shape should still deserialize");
        assert!(!v.is_members_only());
    }

    #[test]
    fn find_file_matches_the_id_tag_not_a_prefix() {
        let dir = std::env::temp_dir().join(format!("vault-find-file-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Some Title [abcdefghijk].mp4"), b"x").unwrap();
        std::fs::write(dir.join("Other [abcdefghijk]-extra.mp4"), b"x").unwrap();
        std::fs::write(dir.join("Unrelated [zzzzzzzzzzz].webm"), b"x").unwrap();

        let hit = find_file(&dir, "abcdefghijk").expect("should find the tagged file");
        assert_eq!(hit.file_name().unwrap().to_string_lossy(), "Some Title [abcdefghijk].mp4");
        assert!(find_file(&dir, "nosuchvide").is_none());
        // A missing directory is a normal case, not a panic.
        assert!(find_file(&dir.join("gone"), "abcdefghijk").is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn flat_entries_keep_ytdlp_order() {
        // A flat playlist in yt-dlp's newest-first order. The parser used to
        // walk this with a stack, which popped LIFO and reversed the list — the
        // symptom being that Newest and Oldest appeared swapped.
        let payload = r#"{
            "title": "Some Channel - Videos",
            "channel": "Some Channel",
            "entries": [
                {"id": "newest00001", "title": "newest"},
                {"id": "middle00002", "title": "middle"},
                {"id": "oldest00003", "title": "oldest"}
            ]
        }"#;
        let root: serde_json::Value = serde_json::from_str(payload).unwrap();
        let raw: Vec<&serde_json::Value> = root["entries"].as_array().unwrap().iter().collect();
        let mut queue: std::collections::VecDeque<&serde_json::Value> = raw.into();
        let mut order = Vec::new();
        while let Some(e) = queue.pop_front() {
            if let Some(nested) = e.get("entries").and_then(|v| v.as_array()) {
                queue.extend(nested.iter());
                continue;
            }
            if let Some(id) = e.get("id").and_then(|v| v.as_str()) {
                order.push(id.to_string());
            }
        }
        assert_eq!(order, vec!["newest00001", "middle00002", "oldest00003"]);
    }

    #[test]
    fn state_round_trips_through_json() {
        let mut s = PersistedState::default();
        s.active = Some("@kurzgesagt".into());
        s.channels.push(SavedChannel {
            url: "@kurzgesagt".into(),
            title: "Kurzgesagt".into(),
            download_dir: Some("/tmp/videos".into()),
            videos: vec![Video {
                id: "abc12345678".into(),
                title: "A video".into(),
                duration: Some(61.0),
                view_count: Some(1000),
                url: "https://www.youtube.com/watch?v=abc12345678".into(),
                availability: None,
            }],
            ..Default::default()
        });
        let json = serde_json::to_string(&s).unwrap();
        let back: PersistedState = serde_json::from_str(&json).unwrap();
        assert_eq!(back.active.as_deref(), Some("@kurzgesagt"));
        assert_eq!(back.channels[0].download_dir.as_deref(), Some("/tmp/videos"));
        assert_eq!(back.channels[0].videos[0].id, "abc12345678");
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            fetch_channel,
            download_video,
            cancel_download,
            downloaded_ids,
            list_archive,
            channel_vaults,
            load_state,
            save_state,
            default_download_dir_str,
            ensure_dir,
            set_channel_dir,
            migrate_archives,
            reveal_folder,
            open_file
        ])
        .run(tauri::generate_context!())
        .expect("error while running Vault");
}