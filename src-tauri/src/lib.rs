use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

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
    /// The channel's identity in the saved state: `canonical_channel_url` of
    /// whatever was typed, so "@foo" and "https://youtube.com/@foo" don't
    /// become two entries.
    url: String,
    title: String,
    uploader: String,
    tab: String,
    video_count: usize,
    videos: Vec<Video>,
    #[serde(flatten)]
    about: ChannelAbout,
}

/// What the channel page shows above its videos. All of it comes free with
/// the listing; none of it is guaranteed, so every field can be absent.
#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
struct ChannelAbout {
    #[serde(default)]
    avatar: Option<String>,
    #[serde(default)]
    banner: Option<String>,
    #[serde(default)]
    followers: Option<u64>,
    #[serde(default)]
    verified: bool,
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
    /// Saved with the channel so the header draws on launch without a fetch.
    /// Absent in state files written before the header showed them.
    #[serde(flatten)]
    about: ChannelAbout,
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
    /// Browser whose logged-in cookies yt-dlp should use, when the user picked
    /// one. Clears YouTube's IP bot check and unlocks members-only videos.
    /// None = anonymous requests. Kept in the state file so it survives
    /// relaunch; the frontend owns the value and passes it to every
    /// fetch/download call.
    #[serde(default)]
    cookies_browser: Option<String>,
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

/// The `--cookies-from-browser` arguments for a browser spec from the UI.
///
/// yt-dlp's supported browser list has no `floorp` entry, but Floorp is a
/// Firefox fork whose cookie store the firefox extractor reads when given the
/// profile directory explicitly. A Floorp selection therefore resolves to
/// `firefox:<profile dir>`; any other spec passes through untouched.
fn cookies_args(spec: &str) -> Result<Vec<String>, String> {
    if spec.trim().is_empty() {
        return Ok(Vec::new());
    }
    if spec == "floorp" {
        let profiles = home().join("Library/Application Support/floorp/Profiles");
        let profile = floorp_profile(&profiles).ok_or_else(|| {
            format!(
                "No Floorp profile found in {} — is Floorp installed and has it been run at least once?",
                profiles.display()
            )
        })?;
        return Ok(vec![
            "--cookies-from-browser".to_string(),
            format!("firefox:{}", profile.display()),
        ]);
    }
    Ok(vec!["--cookies-from-browser".to_string(), spec.to_string()])
}

/// Floorp's default profile: `default-release` when present — the profile
/// Firefox forks write everyday cookies to — else the only profile in
/// `Profiles/`.
fn floorp_profile(profiles_dir: &std::path::Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(profiles_dir).ok()?;
    let dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    let named = dirs
        .iter()
        .find(|d| d.file_name().map_or(false, |n| n.to_string_lossy().ends_with("default-release")));
    named.cloned().or_else(|| if dirs.len() == 1 { dirs.into_iter().next() } else { None })
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

/// A channel's identity: one spelling per channel, whatever was typed or
/// pasted. `@foo`, `youtube.com/@foo`, `m.youtube.com/@foo/videos?view=0` and
/// `https://www.youtube.com/@foo/` are all `https://www.youtube.com/@foo`.
/// The tab is dropped because the fetch appends its own. Handles and the old
/// `/c/` and `/user/` names are lowercased, since YouTube ignores their case;
/// `/channel/UC…` ids are case-sensitive and kept as they are.
///
/// Anything that isn't a YouTube channel URL (a single video, a playlist,
/// another site) comes back as `normalize_input` left it.
fn canonical_channel_url(raw: &str) -> String {
    let url = normalize_input(raw);
    if !is_channel_url(&url) {
        return url;
    }
    let Some((_, rest)) = url.split_once("://") else { return url };
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.to_lowercase();
    if host != "youtube.com" && !host.ends_with(".youtube.com") {
        return url;
    }
    let path = path.split(['?', '#']).next().unwrap_or("");
    let mut segments = path.split('/').filter(|s| !s.is_empty());
    let channel = match segments.next() {
        Some(handle) if handle.starts_with('@') => handle.to_lowercase(),
        Some(kind @ ("channel" | "c" | "user")) => match segments.next() {
            Some(id) if kind == "channel" => format!("{kind}/{id}"),
            Some(name) => format!("{kind}/{}", name.to_lowercase()),
            None => return url,
        },
        _ => return url,
    };
    format!("https://www.youtube.com/{channel}")
}

/// Whether the URL points at a channel (so a tab path can be appended) rather
/// than a single video.
fn is_channel_url(u: &str) -> bool {
    let channelish = u.contains("/@") || u.contains("/channel/") || u.contains("/c/") || u.contains("/user/");
    channelish && !u.contains("/watch") && !u.contains("youtu.be/")
}

/// Whether yt-dlp failed only because the channel has no such tab, e.g.
/// "ERROR: [youtube:tab] @jawed: This channel does not have a shorts tab".
fn is_missing_tab(stderr: &str) -> bool {
    let l = stderr.to_lowercase();
    l.contains("this channel does not have a") && l.contains(" tab")
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
    // The bot check names the fix in its own message, so the reason does too:
    // cookies are the workaround when YouTube starts rate-limiting this IP.
    if has("sign in to confirm you're not a bot") {
        return Some("YouTube bot check — set cookies");
    }
    // Wrong/keyring-locked browser or no logged-in session: yt-dlp names the
    // browser and the operation in its message.
    if has("could not copy") && (has("cookies") || has("keyring")) {
        return Some("Couldn't read cookies — is that browser running? Try logging in, or pick another browser");
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
async fn fetch_channel(url: String, tab: String, cookies: Option<String>) -> Result<ChannelInfo, String> {
    tauri::async_runtime::spawn_blocking(move || fetch_channel_blocking(url, tab, cookies))
        .await
        .map_err(|e| e.to_string())?
}

fn fetch_channel_blocking(raw: String, tab: String, cookies: Option<String>) -> Result<ChannelInfo, String> {
    let yt = ytdlp_path();
    let identity = canonical_channel_url(&raw);
    let mut url = identity.clone();
    // `/videos` excludes both shorts and past livestreams, which is exactly the
    // default view. The other tabs are appended the same way when asked for.
    if is_channel_url(&url) {
        url = format!("{}/{}", url.trim_end_matches('/'), tab);
    }

    // Cookies ride along on listing too: the bot check gates the playlist
    // endpoints just as it gates video downloads.
    let mut args = vec!["--flat-playlist".to_string(), "-J".to_string(), "--no-warnings".to_string()];
    args.extend(cookies_args(cookies.as_deref().unwrap_or("")).map_err(|e| format!("Cookies: {e}"))?);
    args.push(url);

    let out = Command::new(&yt)
        .args(&args)
        .output()
        .map_err(|e| format!("Could not run yt-dlp ({yt}): {e}"))?;

    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        // A channel with no Shorts (or Live) tab is an empty list, not a
        // failure: yt-dlp exits non-zero for it, and passing that on would put
        // an error screen where "No shorts on this channel" belongs.
        if is_missing_tab(&err) {
            return Ok(ChannelInfo {
                url: identity,
                title: String::new(),
                uploader: String::new(),
                tab,
                video_count: 0,
                videos: Vec::new(),
                about: ChannelAbout::default(),
            });
        }
        let msg = err.trim().lines().last().unwrap_or("yt-dlp failed").to_string();
        return Err(msg);
    }

    let root: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("Bad JSON from yt-dlp: {e}"))?;

    Ok(parse_listing(&root, identity, tab))
}

/// Turn yt-dlp's `--flat-playlist -J` output into a channel listing.
fn parse_listing(root: &serde_json::Value, identity: String, tab: String) -> ChannelInfo {
    let raw_entries: Vec<&serde_json::Value> = match root.get("entries") {
        Some(serde_json::Value::Array(a)) => a.iter().collect(),
        _ => vec![root],
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

    ChannelInfo {
        url: identity,
        title,
        uploader,
        tab,
        video_count: videos.len(),
        videos,
        about: channel_about(root),
    }
}

/// The avatar, banner and counts from a listing's top level. yt-dlp puts both
/// images in one `thumbnails` list: the avatar is the square one, banners are
/// the wide ones in several sizes, and each also comes as an `*_uncropped`
/// original with no dimensions.
fn channel_about(root: &serde_json::Value) -> ChannelAbout {
    let thumbs: Vec<&serde_json::Value> =
        root.get("thumbnails").and_then(|t| t.as_array()).map(|a| a.iter().collect()).unwrap_or_default();
    let size = |t: &serde_json::Value| Some((t.get("width")?.as_u64()?, t.get("height")?.as_u64()?));
    let url = |t: &serde_json::Value| t.get("url").and_then(|u| u.as_str()).map(|s| s.to_string());
    let by_id = |id: &str| thumbs.iter().find(|t| t.get("id").and_then(|i| i.as_str()) == Some(id)).and_then(|t| url(t));

    let avatar = thumbs
        .iter()
        .find(|t| size(t).is_some_and(|(w, h)| w == h))
        .and_then(|t| url(t))
        .or_else(|| by_id("avatar_uncropped"));
    // The narrowest banner that still fills a wide window; the largest is 2560.
    let mut banners: Vec<(u64, &serde_json::Value)> =
        thumbs.iter().filter_map(|t| size(t).filter(|(w, h)| w > h).map(|(w, _)| (w, *t))).collect();
    banners.sort_by_key(|(w, _)| *w);
    let banner = banners
        .iter()
        .find(|(w, _)| *w >= 1700)
        .or(banners.last())
        .and_then(|(_, t)| url(t))
        .or_else(|| by_id("banner_uncropped"));

    ChannelAbout {
        avatar,
        banner,
        followers: root.get("channel_follower_count").and_then(|v| v.as_u64()),
        verified: root.get("channel_is_verified").and_then(|v| v.as_bool()).unwrap_or(false),
    }
}

/// One download, as the frontend asks for it.
struct DownloadRequest {
    id: String,
    url: String,
    dir: PathBuf,
    /// Run without the archive, for a file the archive records but the disk
    /// has lost: yt-dlp would otherwise skip the id.
    force: bool,
    cookies: Option<String>,
}

/// What a running download reports. `Done` is always the last event.
enum DownloadEvent {
    Progress(Progress),
    /// yt-dlp found the id in the archive and downloaded nothing.
    Skipped,
    Done { ok: bool, reason: Option<String> },
}

#[tauri::command]
fn download_video(app: AppHandle, id: String, url: String, dir: String, force: Option<bool>, cookies: Option<String>) -> Result<(), String> {
    let request = DownloadRequest {
        id: id.clone(),
        url,
        dir: PathBuf::from(&dir),
        force: force.unwrap_or(false),
        cookies,
    };
    let handle = app.clone();
    let job = id.clone();
    let st = app.state::<AppState>();
    // Held across the spawn so the pid is recorded before `Done` can remove
    // it: a download that fails at once would otherwise leave its pid behind.
    let mut pids = st.pids.lock().unwrap();
    let pid = start_download(request, move |event| match event {
        DownloadEvent::Progress(p) => {
            let _ = handle.emit("download-progress", p);
        }
        DownloadEvent::Skipped => {
            let _ = handle.emit("download-skipped", serde_json::json!({ "id": job }));
        }
        DownloadEvent::Done { ok, reason } => {
            let st = handle.state::<AppState>();
            let was_cancelled = st.cancelled.lock().unwrap().remove(&job);
            st.pids.lock().unwrap().remove(&job);
            let _ = handle.emit(
                "download-done",
                serde_json::json!({
                    "id": job,
                    "ok": ok && !was_cancelled,
                    "cancelled": was_cancelled,
                    "reason": if ok || was_cancelled { None } else { reason },
                }),
            );
        }
    })?;
    pids.insert(id, pid);
    Ok(())
}

/// Spawn yt-dlp for one video and report on it through `emit`, from background
/// threads. Returns the process id once it is running. Nothing here knows about
/// Tauri, so the live test can run a real download without a window.
fn start_download(req: DownloadRequest, emit: impl Fn(DownloadEvent) + Send + 'static) -> Result<u32, String> {
    let yt = ytdlp_path();
    let dir = req.dir.clone();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    // The archive lives inside this channel's folder, so it follows the videos.
    // A forced re-download runs without it — the archive still says this video
    // is present, and letting yt-dlp read it would skip the job — and the
    // record is re-stamped by hand only once the download succeeds.
    let force = req.force;
    let archive = channel_archive_path(&dir);
    if let Some(parent) = archive.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let mut cmd: Command = Command::new(&yt);
    cmd.arg("--newline")
        .arg("--no-warnings");
    // Cookies ride along here too: they clear the IP bot check and they are
    // the only way members-only videos download at all.
    let cookie_flags = req
        .cookies
        .as_deref()
        .map(cookies_args)
        .transpose()
        .map_err(|e| format!("Cookies: {e}"))?
        .unwrap_or_default();
    for a in &cookie_flags {
        cmd.arg(a);
    }
    if !force {
        cmd.arg("--download-archive").arg(&archive);
    }
    cmd
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
        .arg(&req.url);
    let mut child: Child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not run yt-dlp ({yt}): {e}"))?;

    let stdout = child.stdout.take().ok_or("yt-dlp produced no output stream")?;
    let stderr = child.stderr.take().ok_or("yt-dlp produced no error stream")?;
    let pid = child.id();

    // Why the download failed, read off yt-dlp's output by both threads.
    let failure = Arc::new(Mutex::new(FailureNotes::default()));

    // stderr is drained on its own thread. Leaving it piped and unread is a real
    // hazard: a verbose failure fills the pipe buffer and blocks yt-dlp forever.
    let failure_err = failure.clone();
    let err_thread = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            failure_err.lock().unwrap().read(&line);
        }
    });

    let job = req.id;
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(rest) = line.strip_prefix("VAULTPROG|") {
                let p: Vec<&str> = rest.split('|').collect();
                let num = |i: usize| p.get(i).and_then(|s| s.trim().parse::<f64>().ok()).unwrap_or(0.0);
                let downloaded = num(0);
                let total = if num(1) > 0.0 { num(1) } else { num(2) };
                emit(DownloadEvent::Progress(Progress { id: job.clone(), downloaded, total, speed: num(3), eta: num(4) }));
            } else if line.contains("already been recorded in the archive") {
                emit(DownloadEvent::Skipped);
            } else {
                // Some failures print on stdout rather than stderr.
                failure.lock().unwrap().read(&line);
            }
        }
        let status = child.wait();
        let ok = status.map(|s| s.success()).unwrap_or(false);
        let _ = err_thread.join();
        // A forced re-download runs without the archive, so yt-dlp wrote no
        // record; stamp it here, only when the download actually landed.
        if ok && force {
            append_to_archive(&archive, &job);
        }
        let reason = if ok { None } else { Some(failure.lock().unwrap().reason()) };
        emit(DownloadEvent::Done { ok, reason });
    });

    Ok(pid)
}

/// What yt-dlp's output said about a failure: the first line that classifies,
/// and the first raw `ERROR:` line for failures classify_failure doesn't know.
/// Without the raw line an unmapped failure reports as just "Failed", which
/// tells the user nothing — the line usually names the real cause ("The page
/// needs to be reloaded", format problems, etc.).
#[derive(Default)]
struct FailureNotes {
    classified: Option<String>,
    raw: Option<String>,
}

impl FailureNotes {
    fn read(&mut self, line: &str) {
        if self.raw.is_none() && line.trim_start().starts_with("ERROR:") {
            let text = line.trim().strip_prefix("ERROR:").unwrap_or(line).trim();
            // Cut on a character boundary; a byte cut can land inside one.
            self.raw = Some(text.chars().take(200).collect());
        }
        if self.classified.is_none() {
            self.classified = classify_failure(line).map(|r| r.to_string());
        }
    }

    fn reason(&self) -> String {
        self.classified.clone().or_else(|| self.raw.clone()).unwrap_or_else(|| "Failed".into())
    }
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
    /// The owning channel's saved URL — the channel whose folder recorded this
    /// id — so the UI can scope the archive to one channel.
    channel_url: Option<String>,
    /// The folder the file landed in, when the id's channel is known but its
    /// file couldn't be found (e.g. an unmounted external drive).
    dir: Option<String>,
    /// The file on disk, when it can be found.
    path: Option<String>,
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

/// Every archived id across all remembered channels with its owning channel,
/// deduped (first channel to record it wins). Ownership comes from the archive
/// files themselves, not from the saved video lists: a video that has since
/// dropped off its channel's list (deleted on YouTube, channel re-fetched) is
/// still owned by the folder that holds its record. The folders are the source
/// of truth: a channel whose folder is missing (unmounted drive, moved by hand)
/// simply contributes nothing rather than erroring.
fn archive_owners(state: &PersistedState) -> Vec<(String, String, Option<String>)> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for c in &state.channels {
        let Some(dir) = c.download_dir.as_deref() else { continue };
        for id in read_archive(&channel_archive_path(std::path::Path::new(dir))) {
            if seen.insert(id.clone()) {
                out.push((id, c.url.clone(), c.download_dir.clone()));
            }
        }
    }
    out
}

/// Every downloaded file directly under `dir`, by the `[<id>]` tag yt-dlp puts
/// at the end of each name it writes. One directory read answers for every id
/// in the folder; reading it once per id was quadratic, and on a channel of
/// thousands on an external drive that ran after every finished download.
///
/// Only `dir` itself is read: the download template is flat.
fn index_folder(dir: &std::path::Path) -> HashMap<String, PathBuf> {
    let mut files = HashMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return files };
    for e in entries.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        // Skip hidden files. On exFAT macOS parks metadata in AppleDouble
        // companions (`._Title [id].mp4`) beside the real download; they carry
        // the same [id] tag, so without this skip one can win the lookup and
        // get handed to the player — which then opens a metadata stub: listed,
        // but zero duration and nothing to play.
        if name.starts_with('.') {
            continue;
        }
        // A download still in progress, or abandoned, is not a file to play.
        if name.ends_with(".part") || name.ends_with(".ytdl") {
            continue;
        }
        // The tag closes the stem and the extension follows. `[id].mp4`
        // matches, `[id]-extra.mp4` does not.
        let Some(close) = name.rfind(']') else { continue };
        let after = &name[close + 1..];
        if !(after.is_empty() || after.starts_with('.')) {
            continue;
        }
        let Some(open) = name[..close].rfind('[') else { continue };
        files.entry(name[open + 1..close].to_string()).or_insert_with(|| e.path());
    }
    files
}

/// The file for one id, when a single lookup is all that's needed.
#[cfg(test)]
fn find_file(dir: &std::path::Path, id: &str) -> Option<PathBuf> {
    index_folder(dir).remove(id)
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
    archive_entries(&state_or_default())
}

fn archive_entries(state: &PersistedState) -> Vec<ArchiveEntry> {
    // Each channel folder is read once, the first time one of its ids comes up.
    let mut folders: HashMap<String, HashMap<String, PathBuf>> = HashMap::new();
    // id -> (channel title, video). Used only to enrich entries with metadata;
    // ownership is decided by the archive files, not by these lists.
    let mut known: HashMap<String, (String, Video)> = HashMap::new();
    for c in &state.channels {
        for v in &c.videos {
            known.entry(v.id.clone()).or_insert_with(|| (c.title.clone(), v.clone()));
        }
    }
    let titles: HashMap<&str, &str> = state
        .channels
        .iter()
        .map(|c| (c.url.as_str(), c.title.as_str()))
        .collect();

    archive_owners(state)
        .into_iter()
        .map(|(id, owner_url, owner_dir)| {
            let path = owner_dir
                .as_ref()
                .and_then(|d| {
                    folders
                        .entry(d.clone())
                        .or_insert_with(|| index_folder(std::path::Path::new(d)))
                        .get(&id)
                })
                .map(|p| p.to_string_lossy().into_owned());
            let (list_channel, v) = match known.remove(&id) {
                Some((ch, v)) => (Some(ch), Some(v)),
                None => (None, None),
            };
            let owner_title = titles.get(owner_url.as_str()).copied();
            // The owning channel names the entry, even when the video has
            // dropped off every saved list. The saved-list channel is only a
            // fallback for states whose archive predates per-channel folders.
            let channel = owner_title.or(list_channel.as_deref()).map(|s| s.to_string());
            let url = v.as_ref().map(|v| v.url.clone()).unwrap_or_else(|| format!("https://www.youtube.com/watch?v={id}"));
            // A video that has dropped off every saved list still has its
            // title in the file's name: "<title> [<id>].<ext>".
            let title_on_disk = path.as_deref().and_then(|p| {
                let name = std::path::Path::new(p).file_name()?.to_string_lossy().into_owned();
                let title = name[..name.rfind(&format!("[{id}]"))?].trim();
                (!title.is_empty()).then(|| title.to_string())
            });
            ArchiveEntry {
                title: v.as_ref().map(|v| v.title.clone()).or(title_on_disk).unwrap_or_else(|| id.clone()),
                id,
                duration: v.as_ref().and_then(|v| v.duration),
                view_count: v.as_ref().and_then(|v| v.view_count),
                url,
                channel,
                channel_url: Some(owner_url),
                dir: owner_dir,
                path,
            }
        })
        .collect()
}

/// Record an id in an archive file, creating the file if needed. The line
/// format matches what yt-dlp writes — `<extractor> <id>` — and the extractor
/// is always `youtube` here: every URL the app hands yt-dlp is a YouTube one.
/// Used after a forced re-download, which runs without --download-archive so
/// yt-dlp cannot skip the video; the record lands here once it succeeds.
fn append_to_archive(path: &std::path::Path, id: &str) {
    if read_archive(path).iter().any(|have| have == id) {
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut body = std::fs::read_to_string(path).unwrap_or_default();
    if !body.is_empty() && !body.ends_with('\n') {
        body.push('\n');
    }
    body.push_str("youtube ");
    body.push_str(id);
    body.push('\n');
    let _ = std::fs::write(path, body);
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

/// Read the state file. A file that isn't there (or is empty) is a first run
/// and yields the default. A file that is there but can't be read or parsed is
/// an error, never the default: the frontend saves the whole state back after
/// any change, so answering "empty" here is what lets the next save replace a
/// real channel list with nothing.
fn read_state_at(path: &std::path::Path) -> Result<PersistedState, String> {
    let unreadable = |why: String| {
        format!(
            "Couldn't read {}: {why}. Vault left the file alone and won't save over it. \
             Fix the JSON, or move the file aside to start fresh, then reopen Vault.",
            path.display()
        )
    };
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(PersistedState::default()),
        Err(e) => return Err(unreadable(e.to_string())),
    };
    if text.trim().is_empty() {
        return Ok(PersistedState::default());
    }
    let mut state: PersistedState = serde_json::from_str(&text).map_err(|e| unreadable(e.to_string()))?;
    canonicalize_channels(&mut state);
    Ok(state)
}

/// Bring saved channel URLs to their canonical form, so a channel saved under
/// an older spelling is the same channel a fetch returns today. Entries that
/// turn out to be one channel merge into the first, which keeps a download
/// folder if either had one.
fn canonicalize_channels(state: &mut PersistedState) {
    state.active = state.active.take().map(|u| canonical_channel_url(&u));
    let mut merged: Vec<SavedChannel> = Vec::new();
    for mut c in std::mem::take(&mut state.channels) {
        c.url = canonical_channel_url(&c.url);
        match merged.iter_mut().find(|m| m.url == c.url) {
            Some(first) => {
                if first.download_dir.is_none() {
                    first.download_dir = c.download_dir;
                }
            }
            None => merged.push(c),
        }
    }
    state.channels = merged;
}

/// The saved state for the commands that only read it. They degrade to "no
/// channels" when the file is unreadable; `load_state` is where that surfaces.
fn state_or_default() -> PersistedState {
    read_state_at(&state_path()).unwrap_or_default()
}

#[tauri::command]
fn load_state() -> Result<PersistedState, String> {
    read_state_at(&state_path())
}

#[tauri::command]
fn save_state(state: PersistedState) -> Result<(), String> {
    write_state_at(&state_path(), &state)
}

fn write_state_at(path: &std::path::Path, state: &PersistedState) -> Result<(), String> {
    // Refuse to replace a file we couldn't have loaded. Whatever is in it is
    // the only copy of the user's channel list.
    read_state_at(path)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;
    // Write beside the target and rename, so a crash mid-write can't leave a
    // truncated state file where the real one was.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())?;
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

    let state = load_state()?;
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

#[cfg(test)]
mod tests {
    use super::*;

    // Network test. Run with: cargo test -- --ignored --nocapture
    #[test]
    #[ignore]
    fn fetch_real_channel_end_to_end() {
        let info = fetch_channel_blocking("@kurzgesagt".to_string(), "videos".to_string(), None)
            .expect("fetch should succeed against a live channel");
        println!("channel: {} | videos: {}", info.title, info.video_count);
        assert!(info.video_count > 50);
        assert_eq!(info.videos[0].id.len(), 11);
    }

    #[test]
    #[ignore]
    fn members_only_is_flagged_from_the_listing() {
        let info = fetch_channel_blocking("@LinusTechTips".to_string(), "videos".to_string(), None)
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
    fn every_spelling_of_a_channel_is_one_identity() {
        let want = "https://www.youtube.com/@foo";
        for raw in [
            "@foo",
            "@Foo",
            "FOO",
            "https://www.youtube.com/@FoO/videos",
            "foo",
            "https://www.youtube.com/@foo",
            "https://youtube.com/@foo",
            "http://m.youtube.com/@foo/",
            "https://www.youtube.com/@foo/videos",
            "https://www.YouTube.com/@foo/shorts?view=0",
            "https://www.youtube.com/@foo/streams#x",
        ] {
            assert_eq!(canonical_channel_url(raw), want, "from {raw}");
        }
        assert_eq!(
            canonical_channel_url("https://youtube.com/channel/UCXuqSBlHAE6Xw-yeJA0Tunw/videos"),
            "https://www.youtube.com/channel/UCXuqSBlHAE6Xw-yeJA0Tunw"
        );
        assert_eq!(canonical_channel_url("https://youtube.com/c/Foo/about"), "https://www.youtube.com/c/foo");
        // Not a channel: left exactly as typed.
        for raw in [
            "https://www.youtube.com/watch?v=abc12345678",
            "https://youtu.be/abc12345678",
            "https://www.youtube.com/playlist?list=PLabc",
            "https://example.com/@foo/videos",
        ] {
            assert_eq!(canonical_channel_url(raw), raw);
        }
    }

    fn temp_state_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vault-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("state.json")
    }

    #[test]
    fn a_missing_state_file_is_a_first_run() {
        let file = temp_state_file("state-missing");
        let state = read_state_at(&file).expect("no file is not an error");
        assert!(state.channels.is_empty());
        write_state_at(&file, &state).expect("a first save creates the file");
        assert!(file.is_file());
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn an_unreadable_state_file_is_never_saved_over() {
        // The frontend saves the whole state back after any change. If a file
        // that fails to parse loaded as "no channels", the next save would
        // replace the user's channel list with an empty one.
        let file = temp_state_file("state-corrupt");
        let damaged = r#"{"active":"https://www.youtube.com/@foo","channels":[{"url":"#;
        std::fs::write(&file, damaged).unwrap();

        let err = read_state_at(&file).err().expect("a damaged file must not load as empty");
        assert!(err.contains("move the file aside"), "the error should say how to recover: {err}");
        assert!(write_state_at(&file, &PersistedState::default()).is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), damaged, "the file must be left as it was");
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn saved_channels_load_under_their_canonical_url() {
        // A channel saved under an older spelling has to be the channel a
        // fetch returns today, or Sync adds a second entry with no folder.
        let file = temp_state_file("state-canonical");
        std::fs::write(
            &file,
            r#"{"active":"https://youtube.com/@foo/videos","channels":[
                {"url":"https://youtube.com/@foo/videos","title":"Foo","uploader":"Foo"},
                {"url":"https://www.youtube.com/@foo","title":"Foo","uploader":"Foo","download_dir":"/tmp/foo"},
                {"url":"https://www.youtube.com/@bar","title":"Bar","uploader":"Bar"}
            ]}"#,
        )
        .unwrap();
        let state = read_state_at(&file).unwrap();
        assert_eq!(state.active.as_deref(), Some("https://www.youtube.com/@foo"));
        let urls: Vec<&str> = state.channels.iter().map(|c| c.url.as_str()).collect();
        assert_eq!(urls, ["https://www.youtube.com/@foo", "https://www.youtube.com/@bar"]);
        assert_eq!(state.channels[0].download_dir.as_deref(), Some("/tmp/foo"));
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn a_channel_without_the_tab_is_empty_not_a_failure() {
        // Observed from yt-dlp against @jawed, which has one video and no shorts.
        assert!(is_missing_tab("ERROR: [youtube:tab] @jawed: This channel does not have a shorts tab"));
        assert!(is_missing_tab("ERROR: [youtube:tab] @foo: This channel does not have a streams tab\n"));
        assert!(!is_missing_tab("ERROR: [youtube] abc: Sign in to confirm you're not a bot"));
        assert!(!is_missing_tab("ERROR: unable to download webpage: HTTP Error 503"));
    }

    // Network test, one small channel: a listing, and a tab it doesn't have.
    #[test]
    #[ignore]
    fn a_small_channel_lists_and_its_missing_tab_is_empty() {
        let videos = fetch_channel_blocking("@Jawed".to_string(), "videos".to_string(), None).expect("videos");
        assert_eq!(videos.url, "https://www.youtube.com/@jawed");
        assert!(videos.videos.iter().any(|v| v.id == "jNQXAC9IVRw"));
        let shorts = fetch_channel_blocking("@jawed".to_string(), "shorts".to_string(), None)
            .expect("a missing tab is an empty list");
        assert!(shorts.videos.is_empty());
    }

    // Network test: real downloads. Walks a video through every archive state
    // the UI tells apart — not archived, on disk, recorded but missing — and
    // the forced run that restores a missing one.
    #[test]
    #[ignore]
    fn downloads_move_videos_through_the_archive_states() {
        use std::sync::mpsc;

        // Returns (finished ok, yt-dlp skipped it).
        fn run(v: &Video, dir: &std::path::Path, force: bool) -> (bool, bool) {
            let (tx, rx) = mpsc::channel();
            let req = DownloadRequest { id: v.id.clone(), url: v.url.clone(), dir: dir.to_path_buf(), force, cookies: None };
            start_download(req, move |e| {
                let _ = tx.send(e);
            })
            .expect("yt-dlp should start");
            let mut skipped = false;
            loop {
                match rx.recv_timeout(std::time::Duration::from_secs(300)).expect("download timed out") {
                    DownloadEvent::Skipped => skipped = true,
                    DownloadEvent::Done { ok, reason } => {
                        assert!(ok, "download of {} failed: {reason:?}", v.id);
                        return (ok, skipped);
                    }
                    DownloadEvent::Progress(_) => {}
                }
            }
        }

        let info = fetch_channel_blocking("@billwurtz".to_string(), "videos".to_string(), None).expect("listing");
        assert!(info.video_count > 5, "need a channel with several videos");
        // The two shortest, so the test moves little data.
        let mut by_length: Vec<&Video> = info.videos.iter().filter(|v| v.duration.is_some()).collect();
        by_length.sort_by(|a, b| a.duration.partial_cmp(&b.duration).unwrap());
        let (first, second) = (by_length[0], by_length[1]);

        let dir = std::env::temp_dir().join(format!("vault-live-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut state = PersistedState::default();
        state.channels.push(SavedChannel {
            url: info.url.clone(),
            title: info.title.clone(),
            download_dir: Some(dir.to_string_lossy().into_owned()),
            videos: info.videos.clone(),
            ..Default::default()
        });

        assert!(archive_entries(&state).is_empty(), "nothing is archived before the first download");

        assert_eq!(run(first, &dir, false), (true, false));
        assert_eq!(run(second, &dir, false), (true, false));
        let entries = archive_entries(&state);
        assert_eq!(entries.len(), 2, "only the two downloaded videos are archived");
        for e in &entries {
            let path = e.path.as_deref().expect("a downloaded video is on disk");
            assert!(path.ends_with(".mp4"), "merged into one mp4: {path}");
            assert_eq!(e.channel_url.as_deref(), Some(info.url.as_str()));
        }
        println!("downloaded: {:?}", entries.iter().map(|e| e.path.clone().unwrap()).collect::<Vec<_>>());

        // Lose one file. The archive still records it, so it reads as missing.
        let lost = entries.iter().find(|e| e.id == first.id).unwrap().path.clone().unwrap();
        std::fs::remove_file(&lost).unwrap();
        let entries = archive_entries(&state);
        let missing = entries.iter().find(|e| e.id == first.id).unwrap();
        assert!(missing.path.is_none() && missing.dir.is_some());
        assert!(entries.iter().find(|e| e.id == second.id).unwrap().path.is_some());

        // An ordinary download can't bring it back: yt-dlp trusts the archive.
        assert_eq!(run(first, &dir, false), (true, true));
        assert!(archive_entries(&state).iter().find(|e| e.id == first.id).unwrap().path.is_none());

        // A forced one does, and records the id once.
        assert_eq!(run(first, &dir, true), (true, false));
        assert!(archive_entries(&state).iter().find(|e| e.id == first.id).unwrap().path.is_some());
        let recorded = read_archive(&channel_archive_path(&dir));
        assert_eq!(recorded.iter().filter(|id| **id == first.id).count(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failure_notes_prefer_the_classified_reason() {
        let mut known = FailureNotes::default();
        known.read("[youtube] Extracting URL");
        known.read("ERROR: [youtube] abc: Private video. Sign in if you've been granted access");
        assert_eq!(known.reason(), "Private");

        let mut unknown = FailureNotes::default();
        unknown.read("ERROR: [youtube] abc: The page needs to be reloaded.");
        unknown.read("ERROR: something later");
        assert_eq!(unknown.reason(), "[youtube] abc: The page needs to be reloaded.");

        // 200 characters, not bytes: a cut inside a multi-byte one would panic.
        let mut long = FailureNotes::default();
        long.read(&format!("ERROR: {}", "é".repeat(300)));
        assert_eq!(long.reason().chars().count(), 200);

        assert_eq!(FailureNotes::default().reason(), "Failed");
    }

    #[test]
    fn a_listing_carries_the_channel_art_and_counts() {
        // The shape yt-dlp returned for a real channel: sized banners, a
        // square avatar, and an uncropped original of each with no size.
        let root: serde_json::Value = serde_json::from_str(
            r#"{
            "channel": "Some Channel", "channel_follower_count": 5540000, "channel_is_verified": true,
            "thumbnails": [
                {"id": "0", "width": 1060, "height": 175, "url": "https://img/banner-1060"},
                {"id": "2", "width": 1707, "height": 283, "url": "https://img/banner-1707"},
                {"id": "5", "width": 2560, "height": 424, "url": "https://img/banner-2560"},
                {"id": "banner_uncropped", "url": "https://img/banner-raw"},
                {"id": "7", "width": 900, "height": 900, "url": "https://img/avatar-900"},
                {"id": "avatar_uncropped", "url": "https://img/avatar-raw"}
            ],
            "entries": []
        }"#,
        )
        .unwrap();
        let about = parse_listing(&root, "u".into(), "videos".into()).about;
        assert_eq!(about.avatar.as_deref(), Some("https://img/avatar-900"));
        assert_eq!(about.banner.as_deref(), Some("https://img/banner-1707"));
        assert_eq!(about.followers, Some(5540000));
        assert!(about.verified);

        // A channel with nothing set still parses.
        let bare: serde_json::Value = serde_json::from_str(r#"{"channel": "Bare", "entries": []}"#).unwrap();
        assert_eq!(parse_listing(&bare, "u".into(), "videos".into()).about, ChannelAbout::default());
    }

    #[test]
    fn a_video_gone_from_the_listing_keeps_its_title_from_the_file() {
        let dir = std::env::temp_dir().join(format!("vault-orphan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = channel_archive_path(&dir);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "youtube abcdefghijk\nyoutube zzzzzzzzzzz\n").unwrap();
        std::fs::write(dir.join("Deleted upload [abcdefghijk].mp4"), b"x").unwrap();

        let mut s = PersistedState::default();
        s.channels.push(SavedChannel {
            url: "https://www.youtube.com/@foo".into(),
            title: "Foo".into(),
            download_dir: Some(dir.to_string_lossy().into_owned()),
            ..Default::default()
        });
        let entries = archive_entries(&s);
        assert_eq!(entries[0].title, "Deleted upload");
        // No list and no file: the id is all there is.
        assert_eq!(entries[1].title, "zzzzzzzzzzz");
        assert!(entries[1].path.is_none());
        let _ = std::fs::remove_dir_all(&dir);
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
        assert_eq!(
            classify_failure("ERROR: [youtube] abc: Sign in to confirm you're not a bot. Use --cookies-from-browser"),
            Some("YouTube bot check — set cookies")
        );
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
        // exFAT AppleDouble companion: same tag as the real file, must never
        // win — opening it in a player shows a 0:00 metadata stub.
        std::fs::write(dir.join("._Some Title [abcdefghijk].mp4"), b"x").unwrap();
        // A download that never finished is not a file to play.
        std::fs::write(dir.join("Half [partpartpar].mp4.part"), b"x").unwrap();
        assert!(find_file(&dir, "partpartpar").is_none());

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
        let info = parse_listing(&root, "https://www.youtube.com/@some".into(), "videos".into());
        let order: Vec<&str> = info.videos.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(info.title, "Some Channel");
        assert_eq!(order, vec!["newest00001", "middle00002", "oldest00003"]);
    }

    #[test]
    fn archive_ownership_follows_the_folders() {
        // Ownership comes from the archive files, not the saved video lists:
        // a video still archived in a folder belongs to that channel even when
        // it has dropped off the channel's fetched list (deleted on YouTube).
        let dir = std::env::temp_dir().join(format!("vault-owners-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = channel_archive_path(&dir);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "youtube abcdefghijk\n").unwrap();

        let mut s = PersistedState::default();
        s.channels.push(SavedChannel {
            url: "https://www.youtube.com/@foo".into(),
            title: "Foo".into(),
            download_dir: Some(dir.to_string_lossy().into_owned()),
            ..Default::default()
        });
        let owners = archive_owners(&s);
        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0].0, "abcdefghijk");
        assert_eq!(owners[0].1, "https://www.youtube.com/@foo");
        assert_eq!(owners[0].2.as_deref(), Some(dir.to_string_lossy().as_ref()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_to_archive_is_idempotent_and_creates_the_file() {
        let dir = std::env::temp_dir().join(format!("vault-append-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = channel_archive_path(&dir);

        // Creates the .vault folder and file when neither exists.
        append_to_archive(&file, "abcdefghijk");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "youtube abcdefghijk\n");
        // A second append must not duplicate the line.
        append_to_archive(&file, "abcdefghijk");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "youtube abcdefghijk\n");
        append_to_archive(&file, "zzzzzzzzzzz");
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "youtube abcdefghijk\nyoutube zzzzzzzzzzz\n"
        );

        let _ = std::fs::remove_dir_all(&dir);
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
            list_archive,
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