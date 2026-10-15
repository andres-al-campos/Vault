use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

mod player;

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
    /// Roughly when it was uploaded, in Unix seconds. YouTube's listing only
    /// says "3 weeks ago", so this is good for showing that back and no
    /// better: old videos are right to the month or year. The order of the
    /// list, not this, is what Newest and Oldest sort by.
    #[serde(default)]
    timestamp: Option<i64>,
}

/// Which of a channel's tabs get their new uploads queued when the channel is
/// refreshed. The frontend owns the rule for what counts as new.
#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Debug)]
struct Subscription {
    #[serde(default)]
    videos: bool,
    #[serde(default)]
    shorts: bool,
    #[serde(default)]
    streams: bool,
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
    /// None when the channel isn't subscribed.
    #[serde(default)]
    subscribe: Option<Subscription>,
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

/// Where upload dates were kept for a short while before `videos.json`. Only
/// read, by the one-time import.
fn channel_dates_path(dir: &std::path::Path) -> PathBuf {
    dir.join(".vault").join("dates.txt")
}

/// A channel folder's record of what it holds: one object per archived video.
/// This is the only thing that says a video is archived. It lives in the
/// folder so the folder can move to another drive and still describe itself.
fn records_path(dir: &std::path::Path) -> PathBuf {
    dir.join(".vault").join("videos.json")
}

/// One archived video. Everything but the id can be absent: a file copied in
/// by hand starts with nothing else, and the rest is filled in from the
/// channel listing or looked up on the next Sync. Kept here, a video YouTube
/// has since removed still has all of it.
///
/// There is no title: a channel can rename a video, so a recorded one would go
/// stale. A card shows the listing's title, and once the video is delisted,
/// the one in the file's name.
#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Debug)]
struct Record {
    id: String,
    /// The real upload time, in Unix seconds, from the video's own page.
    #[serde(default)]
    timestamp: Option<i64>,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    view_count: Option<u64>,
    #[serde(default)]
    availability: Option<String>,
    /// When Vault downloaded it. Absent for files it found rather than fetched.
    #[serde(default)]
    downloaded_at: Option<u64>,
}

impl Record {
    fn blank(id: &str) -> Record {
        Record { id: id.to_string(), ..Default::default() }
    }

    /// Takes whatever `other` knows that this record doesn't. Returns whether
    /// anything changed.
    fn fill_from(&mut self, other: &Record) -> bool {
        let before = self.clone();
        self.timestamp = self.timestamp.or(other.timestamp);
        self.duration = self.duration.or(other.duration);
        self.view_count = self.view_count.or(other.view_count);
        self.availability = self.availability.take().or_else(|| other.availability.clone());
        self.downloaded_at = self.downloaded_at.or(other.downloaded_at);
        *self != before
    }
}

#[derive(Serialize, Deserialize, Default)]
struct RecordsFile {
    videos: Vec<Record>,
}

/// A folder's records, in the order they were added.
///
/// A folder from before `videos.json` is read from what it had instead:
/// yt-dlp's `archive.txt` for the ids and `dates.txt` for upload dates. Those
/// two are never written or removed, so an import that went wrong can be
/// redone by deleting `videos.json`.
///
/// A `videos.json` that can't be parsed is an error, not an empty folder:
/// treating it as empty would let the next write replace it.
fn load_records(dir: &std::path::Path) -> Result<Vec<Record>, String> {
    let path = records_path(dir);
    match std::fs::read_to_string(&path) {
        Ok(text) if !text.trim().is_empty() => serde_json::from_str::<RecordsFile>(&text)
            .map(|f| f.videos)
            .map_err(|e| format!(
                "Couldn't read {}: {e}. Vault left the file alone and won't save over it. Fix the JSON, or move the file aside and press Sync to rebuild it from the videos in the folder.",
                path.display()
            )),
        _ => {
            let dates: HashMap<String, i64> = std::fs::read_to_string(channel_dates_path(dir))
                .unwrap_or_default()
                .lines()
                .filter_map(|l| {
                    let (id, t) = l.trim().split_once(' ')?;
                    Some((id.to_string(), t.trim().parse::<f64>().ok()? as i64))
                })
                .collect();
            let mut seen = HashSet::new();
            Ok(read_archive(&channel_archive_path(dir))
                .into_iter()
                .filter(|id| seen.insert(id.clone()))
                .map(|id| Record { timestamp: dates.get(&id).copied(), ..Record::blank(&id) })
                .collect())
        }
    }
}

/// The records of a folder that may be unreadable, for callers that only
/// show them: the reason goes to the log and the folder reads as empty.
fn records_or_none(dir: &std::path::Path) -> Vec<Record> {
    load_records(dir).unwrap_or_else(|e| {
        eprintln!("{e}");
        Vec::new()
    })
}

/// One writer at a time: a finished download, a rescan and a date lookup can
/// all land at once, and each reads the file, changes it and writes it back.
static RECORDS: Mutex<()> = Mutex::new(());

/// Reads a folder's records, lets `change` edit them, and writes them back if
/// it says it changed something (or if this is the import of an older folder).
/// Refuses when the file is damaged, so it is never replaced.
fn update_records(dir: &std::path::Path, change: impl FnOnce(&mut Vec<Record>) -> bool) -> Result<(), String> {
    let _guard = RECORDS.lock().unwrap_or_else(|e| e.into_inner());
    let mut records = load_records(dir)?;
    let path = records_path(dir);
    let importing = !path.is_file() && !records.is_empty();
    if !(change(&mut records) || importing) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Could not create {}: {e}", parent.display()))?;
    }
    let body = serde_json::to_string_pretty(&RecordsFile { videos: records }).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, body).map_err(|e| format!("Could not write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("Could not replace {}: {e}", path.display()))
}

/// Adds or completes one record.
fn upsert(records: &mut Vec<Record>, new: Record) -> bool {
    match records.iter_mut().find(|r| r.id == new.id) {
        Some(r) => r.fill_from(&new),
        None => {
            records.push(new);
            true
        }
    }
}

/// What yt-dlp is asked to print about a video, as one JSON object per line.
const RECORD_FIELDS: &str = "%(.{id,timestamp,duration,view_count,availability})j";

/// The records in yt-dlp's printed lines. Lines that aren't one are skipped.
fn printed_records(printed: &str) -> Vec<Record> {
    printed
        .lines()
        .filter_map(|l| {
            let v: serde_json::Value = serde_json::from_str(l.trim()).ok()?;
            Some(Record {
                id: v.get("id")?.as_str()?.to_string(),
                timestamp: v.get("timestamp").and_then(|t| t.as_f64()).map(|t| t as i64),
                duration: v.get("duration").and_then(|t| t.as_f64()),
                view_count: v.get("view_count").and_then(|t| t.as_u64()),
                availability: v.get("availability").and_then(|t| t.as_str()).map(String::from),
                downloaded_at: None,
            })
        })
        .collect()
}

/// The archived ids in `dir` whose real upload date isn't known yet.
fn undated(dir: &std::path::Path) -> Vec<String> {
    records_or_none(dir).into_iter().filter(|r| r.timestamp.is_none()).map(|r| r.id).collect()
}

/// Looks up every archived video in `dir` that has no upload date: ones
/// downloaded before dates were kept, or copied in by hand. One request per
/// video, so this runs in the background after a Sync. Whatever else the page
/// says that the record lacks (length, views) is taken too. Returns how
/// many dates it added. A video YouTube no longer has yields nothing and is
/// asked for again next time.
#[tauri::command]
async fn fetch_dates(dir: String, cookies: Option<String>) -> Result<usize, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dir = PathBuf::from(dir);
        let ids = undated(&dir);
        if ids.is_empty() {
            return Ok(0);
        }
        let yt = ytdlp_path();
        let mut args: Vec<String> = ["--skip-download", "--no-warnings", "--ignore-errors", "--print", RECORD_FIELDS]
            .map(String::from)
            .to_vec();
        args.extend(cookies_args(cookies.as_deref().unwrap_or("")).map_err(|e| format!("Cookies: {e}"))?);
        args.extend(ids.iter().map(|id| format!("https://www.youtube.com/watch?v={id}")));
        let out = Command::new(&yt)
            .args(&args)
            .output()
            .map_err(|e| format!("Could not run yt-dlp ({yt}): {e}. Install it with `brew install yt-dlp`, then press Sync again."))?;
        record_lookups(&dir, &String::from_utf8_lossy(&out.stdout))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Completes existing records from a lookup's output. A line for a video the
/// folder doesn't record is ignored: a lookup never archives anything.
/// Returns how many records gained an upload date.
fn record_lookups(dir: &std::path::Path, printed: &str) -> Result<usize, String> {
    let found = printed_records(printed);
    let mut dated = 0;
    update_records(dir, |records| {
        let mut changed = false;
        for f in &found {
            let Some(r) = records.iter_mut().find(|r| r.id == f.id) else { continue };
            let had_date = r.timestamp.is_some();
            changed |= r.fill_from(f);
            if !had_date && r.timestamp.is_some() {
                dated += 1;
            }
        }
        changed
    })?;
    Ok(dated)
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
    // A link copied without its scheme, which would otherwise read as a handle.
    let host = s.split('/').next().unwrap_or("").to_lowercase();
    if s.contains('/') && ["youtube.com", "youtu.be", "youtube-nocookie.com"].iter().any(|h| host == *h || host.ends_with(&format!(".{h}"))) {
        return format!("https://{s}");
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

/// Whether a link is to one YouTube video: a watch page, a youtu.be link, a
/// Short, a live page or an embed. Such a link opens the video's channel.
fn is_video_url(raw: &str) -> bool {
    let url = normalize_input(raw);
    let Some((_, rest)) = url.split_once("://") else { return false };
    let (host, tail) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.to_lowercase();
    let path = tail.split(['?', '#']).next().unwrap_or("");
    let first = path.split('/').next().unwrap_or("");
    let has_id = || path.split('/').nth(1).map_or(false, |id| !id.is_empty());
    if host == "youtu.be" {
        return !first.is_empty();
    }
    if host != "youtube.com" && !host.ends_with(".youtube.com") && host != "youtube-nocookie.com" && !host.ends_with(".youtube-nocookie.com") {
        return false;
    }
    match first {
        "watch" => tail.contains("v="),
        "shorts" | "live" | "embed" => has_id(),
        _ => false,
    }
}

/// One video a pasted link points at, and its channel.
#[derive(Serialize, Debug, PartialEq)]
struct VideoRef {
    id: String,
    title: String,
    url: String,
    /// The channel to add when it isn't saved yet.
    channel_url: String,
    /// Every spelling the channel may be saved under: by its handle and by its
    /// id. A channel added either way is the same channel.
    channel_urls: Vec<String>,
    /// Who can see it: `unlisted` for a video only its link finds.
    availability: Option<String>,
}

/// The video and channel in yt-dlp's `-J` output for one video.
fn video_ref(root: &serde_json::Value) -> Option<VideoRef> {
    let text = |k: &str| root.get(k).and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(str::to_string);
    let id = text("id")?;
    let by_handle = text("uploader_id").filter(|h| h.starts_with('@')).map(|h| canonical_channel_url(&h));
    let by_id = text("channel_id").map(|c| format!("https://www.youtube.com/channel/{c}"));
    let channel_urls: Vec<String> = by_handle.into_iter().chain(by_id).collect();
    Some(VideoRef {
        title: text("title").unwrap_or_else(|| id.clone()),
        url: format!("https://www.youtube.com/watch?v={id}"),
        channel_url: channel_urls.first()?.clone(),
        channel_urls,
        availability: text("availability"),
        id,
    })
}

/// Whether yt-dlp failed only because the channel has no such tab, e.g.
/// "ERROR: [youtube:tab] @jawed: This channel does not have a shorts tab".
fn is_missing_tab(stderr: &str) -> bool {
    let l = stderr.to_lowercase();
    l.contains("this channel does not have a") && l.contains(" tab")
}

/// Whether a line of yt-dlp output is YouTube's bot check. yt-dlp writes it
/// with a curly apostrophe ("you’re"); older versions used a straight one.
fn is_bot_check(line: &str) -> bool {
    line.to_lowercase().replace('’', "'").contains("sign in to confirm you're not a bot")
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
    // The queue waits this one out and tries again (see botCheck in
    // main.js), so the reason is matched there word for word.
    if is_bot_check(line) {
        return Some("YouTube bot check");
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

/// What a failed lookup (a channel's list, or whose video a link is) tells the
/// user: yt-dlp's last line, except for the bot check, whose own advice is a
/// command-line flag nobody using the app can pass.
fn lookup_failure(stderr: &str, cookies: Option<&str>) -> String {
    let last = stderr.trim().lines().last().unwrap_or("yt-dlp failed");
    if !is_bot_check(last) {
        return last.to_string();
    }
    match cookies.filter(|c| !c.is_empty()) {
        None => "YouTube wants to check you're not a bot. It usually lets up after a while, so try again later. If it doesn't, pick the browser you're signed into YouTube with from the cookie menu in the top bar.".to_string(),
        Some(id) => {
            // Named as the cookie menu names it: "firefox" reads "Firefox".
            let browser = id[..1].to_uppercase() + &id[1..];
            format!("YouTube wants to check you're not a bot, even with {browser}'s cookies. Open youtube.com in {browser}, check you're signed in, then try again.")
        }
    }
}

#[tauri::command]
fn is_video_link(raw: String) -> bool {
    is_video_url(&raw)
}

/// The 11-character id in a link to one video, whichever form it takes.
fn video_id(raw: &str) -> Option<String> {
    let url = normalize_input(raw);
    let (_, rest) = url.split_once("://")?;
    let (host, tail) = rest.split_once('/').unwrap_or((rest, ""));
    let (path, query) = tail.split_once('?').unwrap_or((tail, ""));
    let path = path.split('#').next().unwrap_or("");
    let mut parts = path.split('/').filter(|p| !p.is_empty());
    let id = if host.eq_ignore_ascii_case("youtu.be") {
        parts.next()?.to_string()
    } else {
        match parts.next()? {
            "watch" => query.split(['&', '#']).find_map(|kv| kv.strip_prefix("v="))?.to_string(),
            "shorts" | "live" | "embed" => parts.next()?.to_string(),
            _ => return None,
        }
    };
    let ok = id.len() == 11 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    ok.then_some(id)
}

/// A page from YouTube the way a browser asks for it, or nothing.
fn get_page(url: &str) -> Option<String> {
    let out = Command::new("/usr/bin/curl")
        .args(["-sSfL", "--max-time", "15", "-H", "Accept-Language: en"])
        .args(["-A", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15"])
        .arg(url)
        .output()
        .ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The string value of the first `"key":"…"` in a page's embedded JSON.
fn json_string_after(page: &str, key: &str) -> Option<String> {
    let open = format!("\"{key}\":\"");
    let start = page.find(&open)? + open.len();
    let mut end = start;
    let bytes = page.as_bytes();
    while end < bytes.len() && bytes[end] != b'"' {
        end += if bytes[end] == b'\\' { 2 } else { 1 };
    }
    serde_json::from_str(&format!("\"{}\"", page.get(start..end)?)).ok().filter(|s: &String| !s.is_empty())
}

fn html_unescape(s: &str) -> String {
    s.replace("&quot;", "\"").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

/// The video and its channel from the video's own web page. The page loads
/// without the bot check, which stops yt-dlp's lookup, and it names the owner
/// even of a members-only video it won't play. Its keys are YouTube's to
/// change, so anything missing means nothing, and the next way is tried.
fn page_ref(page: &str, id: &str) -> Option<VideoRef> {
    if !page.contains(&format!("<link rel=\"canonical\" href=\"https://www.youtube.com/watch?v={id}\"")) {
        return None;
    }
    let by_handle = json_string_after(page, "ownerProfileUrl").map(|u| canonical_channel_url(&u)).filter(|u| is_channel_url(u));
    let by_id = json_string_after(page, "externalChannelId").map(|c| format!("https://www.youtube.com/channel/{c}"));
    let channel_urls: Vec<String> = by_handle.into_iter().chain(by_id).collect();
    let title = page
        .split_once("<meta name=\"title\" content=\"")
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(t, _)| html_unescape(t))
        .filter(|t| !t.is_empty());
    Some(VideoRef {
        id: id.to_string(),
        title: title.unwrap_or_else(|| id.to_string()),
        url: format!("https://www.youtube.com/watch?v={id}"),
        channel_url: channel_urls.first()?.clone(),
        channel_urls,
        availability: page.contains("\"isUnlisted\":true").then(|| "unlisted".to_string()),
    })
}

/// The video and its channel from YouTube's oEmbed answer, which it gives for
/// public and unlisted videos (not members-only ones). It names the channel by
/// handle only.
fn oembed_ref(json: &str, id: &str) -> Option<VideoRef> {
    let root: serde_json::Value = serde_json::from_str(json).ok()?;
    let channel_url = canonical_channel_url(root.get("author_url")?.as_str()?);
    if !is_channel_url(&channel_url) {
        return None;
    }
    Some(VideoRef {
        id: id.to_string(),
        title: root.get("title").and_then(|t| t.as_str()).filter(|t| !t.is_empty()).unwrap_or(id).to_string(),
        url: format!("https://www.youtube.com/watch?v={id}"),
        channel_urls: vec![channel_url.clone()],
        channel_url,
        availability: None,
    })
}

/// Asks YouTube which channel a video belongs to: a few seconds, one request.
/// The video's page first, then oEmbed, then yt-dlp, each only when the one
/// before couldn't say. yt-dlp's lookup is the one the bot check stops.
#[tauri::command]
async fn resolve_video(url: String, cookies: Option<String>) -> Result<VideoRef, String> {
    tauri::async_runtime::spawn_blocking(move || resolve_video_blocking(url, cookies))
        .await
        .map_err(|e| e.to_string())?
}

fn resolve_video_blocking(raw: String, cookies: Option<String>) -> Result<VideoRef, String> {
    if let Some(id) = video_id(&raw) {
        let page = get_page(&format!("https://www.youtube.com/watch?v={id}"));
        if let Some(r) = page.and_then(|p| page_ref(&p, &id)) {
            return Ok(r);
        }
        let oembed = get_page(&format!("https://www.youtube.com/oembed?format=json&url=https://www.youtube.com/watch?v={id}"));
        if let Some(r) = oembed.and_then(|j| oembed_ref(&j, &id)) {
            return Ok(r);
        }
    }
    let yt = ytdlp_path();
    let mut args = vec!["-J".to_string(), "--no-warnings".to_string(), "--no-playlist".to_string(), "--skip-download".to_string()];
    args.extend(cookies_args(cookies.as_deref().unwrap_or("")).map_err(|e| format!("Cookies: {e}"))?);
    args.push(normalize_input(&raw));
    let out = Command::new(&yt)
        .args(&args)
        .output()
        .map_err(|e| format!("Could not run yt-dlp ({yt}): {e}"))?;
    if !out.status.success() {
        return Err(lookup_failure(&String::from_utf8_lossy(&out.stderr), cookies.as_deref()));
    }
    let root: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("Bad JSON from yt-dlp: {e}"))?;
    video_ref(&root).ok_or_else(|| {
        "YouTube didn't say whose video this is. Check the link opens a video in a browser, then try again.".to_string()
    })
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
    // Has yt-dlp turn the page's "3 weeks ago" into a timestamp per video.
    args.extend(["--extractor-args".to_string(), "youtubetab:approximate_date".to_string()]);
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
        return Err(lookup_failure(&err, cookies.as_deref()));
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
    // newest-first, and it is what the Newest/Oldest sort works with (the
    // listing's `timestamp` is only approximate). A stack would
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
        let timestamp = e.get("timestamp").and_then(|v| v.as_f64()).map(|t| t as i64);
        videos.push(Video { id: id.to_string(), title, duration, view_count, url, availability, timestamp });
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
fn download_video(app: AppHandle, id: String, url: String, dir: String, cookies: Option<String>) -> Result<(), String> {
    let request = DownloadRequest {
        id: id.clone(),
        url,
        dir: PathBuf::from(&dir),
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
    // yt-dlp writes what it learned about the video here once the file is in
    // place; the record is made from it when the download ends.
    let landed = dir.join(".vault").join(format!(".landed-{}.json", req.id));
    std::fs::create_dir_all(dir.join(".vault")).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    let _ = std::fs::remove_file(&landed);

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
        // Title, real upload date, length and views, once the file is in
        // place.
        .arg("--print-to-file")
        .arg(format!("after_move:{RECORD_FIELDS}"))
        .arg(&landed)
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
            } else if line.contains("has already been downloaded") {
                // The finished file is already in the folder; yt-dlp leaves it.
                emit(DownloadEvent::Skipped);
            } else {
                // Some failures print on stdout rather than stderr.
                failure.lock().unwrap().read(&line);
            }
        }
        let status = child.wait();
        let ok = status.map(|s| s.success()).unwrap_or(false);
        let _ = err_thread.join();
        let (ok, reason) = if ok {
            match record_download(&dir, &job, &landed) {
                Ok(()) => (true, None),
                Err(e) => (false, Some(e)),
            }
        } else {
            (false, Some(failure.lock().unwrap().reason()))
        };
        let _ = std::fs::remove_file(&landed);
        emit(DownloadEvent::Done { ok, reason });
    });

    Ok(pid)
}

/// Records a finished download: what yt-dlp printed about it, or, when it
/// printed nothing (the file was already there), a bare record for the file.
/// With neither there is nothing to call archived.
fn record_download(dir: &std::path::Path, id: &str, landed: &std::path::Path) -> Result<(), String> {
    let printed = std::fs::read_to_string(landed).unwrap_or_default();
    let mut record = match printed_records(&printed).into_iter().rev().find(|r| r.id == id) {
        Some(r) => r,
        None => match index_folder(dir).get(id) {
            Some(_) => Record::blank(id),
            None => return Err(format!(
                "yt-dlp finished but no video file for {id} is in {}. Check the folder is writable and has space, then retry.",
                dir.display()
            )),
        },
    };
    record.downloaded_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs());
    update_records(dir, |records| upsert(records, record))
}

/// The title in a file's name: "<title> [<id>].<ext>".
fn title_in_name(file: &std::path::Path, id: &str) -> Option<String> {
    let name = file.file_name()?.to_string_lossy().into_owned();
    let title = name[..name.rfind(&format!("[{id}]"))?].trim();
    (!title.is_empty()).then(|| title.to_string())
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
    /// The real upload date, when it has been recorded.
    timestamp: Option<i64>,
    /// YouTube's word for who can see it (`public`, `unlisted`,
    /// `subscriber_only`…): the listing's when listed, else what was recorded.
    availability: Option<String>,
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
        for id in records_or_none(std::path::Path::new(dir)).into_iter().map(|r| r.id) {
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
        // The tag closes the stem and one media extension follows. `[id].mp4`
        // matches, `[id]-extra.mp4` does not. Nor does a download in progress
        // or abandoned (`[id].mp4.part`, `[id].f137.mp4`), which is not a file
        // to play, or a thumbnail or subtitle saved beside the video.
        let Some(close) = name.rfind(']') else { continue };
        let Some(ext) = name[close + 1..].strip_prefix('.') else { continue };
        if !MEDIA_EXTS.contains(&ext.to_ascii_lowercase().as_str()) {
            continue;
        }
        let Some(open) = name[..close].rfind('[') else { continue };
        files.entry(name[open + 1..close].to_string()).or_insert_with(|| e.path());
    }
    files
}

/// What a finished download can be. yt-dlp merges to mp4 here, but files from
/// before that setting, or copied in by hand, come in the others.
const MEDIA_EXTS: [&str; 12] = [
    "mp4", "mkv", "webm", "mov", "m4v", "flv", "m4a", "mp3", "opus", "ogg", "aac", "flac",
];

/// Brings each channel folder's records up to date with what is in it, and
/// returns how many videos it newly recorded:
///
/// - a finished file with no record gets one. This is
///   what lets a file copied in by hand, or a folder from another machine,
///   show as archived;
/// - a record missing its length, views or members flag takes them
///   from the channel's saved list, so they are still there if YouTube later
///   drops the video. Not the date: the listing's is approximate.
///
/// It only ever adds. A record whose file is gone stays, so the card can offer
/// to download it again. Only tags shaped like a YouTube id are taken, so
/// "Song [Official Video].mp4" isn't recorded as a video called that.
fn adopt_untracked(state: &PersistedState) -> usize {
    let mut seen = HashSet::new();
    let mut added = 0;
    for c in &state.channels {
        let Some(dir) = c.download_dir.as_deref() else { continue };
        if !seen.insert(dir) {
            continue;
        }
        let dir = std::path::Path::new(dir);
        let files = index_folder(dir);
        let listed: HashMap<&str, &Video> = c.videos.iter().map(|v| (v.id.as_str(), v)).collect();
        let mut new = 0;
        let result = update_records(dir, |records| {
            let have: HashSet<String> = records.iter().map(|r| r.id.clone()).collect();
            let mut untracked: Vec<(&String, &PathBuf)> =
                files.iter().filter(|(id, _)| !have.contains(*id) && is_video_id(id)).collect();
            untracked.sort();
            for (id, _) in &untracked {
                records.push(Record::blank(id));
            }
            new = untracked.len();
            let mut changed = new > 0;
            for r in records.iter_mut() {
                let Some(v) = listed.get(r.id.as_str()) else { continue };
                changed |= r.fill_from(&Record {
                    id: v.id.clone(),
                    duration: v.duration,
                    view_count: v.view_count,
                    availability: v.availability.clone(),
                    ..Default::default()
                });
            }
            changed
        });
        match result {
            Ok(()) => added += new,
            Err(e) => eprintln!("{e}"),
        }
    }
    added
}

fn is_video_id(s: &str) -> bool {
    s.len() == 11 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
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
    let state = state_or_default();
    adopt_untracked(&state);
    archive_entries(&state)
}

fn archive_entries(state: &PersistedState) -> Vec<ArchiveEntry> {
    // Each channel folder is read once, the first time one of its ids comes up.
    let mut folders: HashMap<String, HashMap<String, PathBuf>> = HashMap::new();
    let mut records: HashMap<String, HashMap<String, Record>> = HashMap::new();
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
            // The record is what's left of a video that has dropped off every
            // saved list; the file's name is the last resort for a title.
            let record = owner_dir.as_ref().and_then(|d| {
                records
                    .entry(d.clone())
                    .or_insert_with(|| {
                        records_or_none(std::path::Path::new(d)).into_iter().map(|r| (r.id.clone(), r)).collect()
                    })
                    .get(&id)
                    .cloned()
            }).unwrap_or_else(|| Record::blank(&id));
            let title_on_disk = path.as_deref().and_then(|p| title_in_name(std::path::Path::new(p), &id));
            let timestamp = record.timestamp;
            ArchiveEntry {
                title: v.as_ref().map(|v| v.title.clone()).or(title_on_disk).unwrap_or_else(|| id.clone()),
                id,
                duration: v.as_ref().and_then(|v| v.duration).or(record.duration),
                view_count: v.as_ref().and_then(|v| v.view_count).or(record.view_count),
                url,
                channel,
                channel_url: Some(owner_url),
                timestamp,
                availability: v.as_ref().and_then(|v| v.availability.clone()).or(record.availability),
                dir: owner_dir,
                path,
            }
        })
        .collect()
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
        have.extend(load_records(dir)?.into_iter().map(|r| r.id));
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
        // The folder's record is videos.json; the lines above are kept in
        // step with it for a folder that hasn't been imported yet.
        let mut ids: Vec<&String> = have.iter().collect();
        ids.sort();
        update_records(dir, |records| ids.iter().fold(false, |c, id| upsert(records, Record::blank(id)) || c))?;
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
        assert_eq!(normalize_input("youtube.com/@foo"), "https://youtube.com/@foo");
        assert_eq!(normalize_input("www.youtube.com/watch?v=abc"), "https://www.youtube.com/watch?v=abc");
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

    #[test]
    fn video_links_are_told_from_channels() {
        for raw in [
            "https://www.youtube.com/watch?v=abc12345678",
            "youtube.com/watch?feature=share&v=abc12345678",
            "https://m.youtube.com/watch?v=abc12345678&t=30s",
            "https://youtu.be/abc12345678?si=x",
            "https://www.youtube.com/shorts/abc12345678",
            "https://www.youtube.com/live/abc12345678",
            "https://www.youtube-nocookie.com/embed/abc12345678",
        ] {
            assert!(is_video_url(raw), "{raw} is a video");
        }
        for raw in [
            "@foo",
            "https://www.youtube.com/@foo",
            "https://www.youtube.com/@foo/shorts",
            "https://www.youtube.com/@foo/streams",
            "https://www.youtube.com/channel/UCXuqSBlHAE6Xw-yeJA0Tunw",
            "https://www.youtube.com/playlist?list=PLabc",
            "https://www.youtube.com/watch",
            "https://www.youtube.com/shorts/",
            "https://example.com/watch?v=abc12345678",
        ] {
            assert!(!is_video_url(raw), "{raw} is not a video");
        }
    }

    #[test]
    fn a_video_names_its_channel_both_ways() {
        let root = serde_json::json!({
            "id": "abc12345678", "title": "a video",
            "uploader_id": "@FoO", "channel_id": "UCXuqSBlHAE6Xw-yeJA0Tunw", "availability": "unlisted",
        });
        assert_eq!(video_ref(&root), Some(VideoRef {
            id: "abc12345678".into(),
            title: "a video".into(),
            url: "https://www.youtube.com/watch?v=abc12345678".into(),
            channel_url: "https://www.youtube.com/@foo".into(),
            channel_urls: vec![
                "https://www.youtube.com/@foo".into(),
                "https://www.youtube.com/channel/UCXuqSBlHAE6Xw-yeJA0Tunw".into(),
            ],
            availability: Some("unlisted".into()),
        }));
        // An old channel with no handle is known by its id alone.
        let root = serde_json::json!({ "id": "abc12345678", "title": "t", "uploader_id": "oldname", "channel_id": "UCx" });
        assert_eq!(video_ref(&root).unwrap().channel_url, "https://www.youtube.com/channel/UCx");
        assert_eq!(video_ref(&serde_json::json!({ "id": "abc12345678" })), None);
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
        fn run(v: &Video, dir: &std::path::Path) -> (bool, bool) {
            let (tx, rx) = mpsc::channel();
            let req = DownloadRequest { id: v.id.clone(), url: v.url.clone(), dir: dir.to_path_buf(), cookies: None };
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

        assert_eq!(run(first, &dir), (true, false));
        assert_eq!(run(second, &dir), (true, false));
        let entries = archive_entries(&state);
        assert_eq!(entries.len(), 2, "only the two downloaded videos are archived");
        for e in &entries {
            let path = e.path.as_deref().expect("a downloaded video is on disk");
            assert!(path.ends_with(".mp4"), "merged into one mp4: {path}");
            assert_eq!(e.channel_url.as_deref(), Some(info.url.as_str()));
        }
        for e in &entries {
            let t = e.timestamp.expect("a download records the video's upload date");
            assert!(t > 1_100_000_000, "a real date, got {t}");
        }
        for r in load_records(&dir).unwrap() {
            assert!(r.duration.is_some() && r.downloaded_at.is_some(), "a full record: {r:?}");
        }
        assert!(!channel_archive_path(&dir).exists(), "yt-dlp's own archive is no longer written");
        let leftovers: Vec<_> = std::fs::read_dir(dir.join(".vault")).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(leftovers, vec![std::ffi::OsString::from("videos.json")], "nothing else left in .vault");
        println!("downloaded: {:?}", entries.iter().map(|e| e.path.clone().unwrap()).collect::<Vec<_>>());

        // Lose one file. The archive still records it, so it reads as missing.
        let lost = entries.iter().find(|e| e.id == first.id).unwrap().path.clone().unwrap();
        std::fs::remove_file(&lost).unwrap();
        let entries = archive_entries(&state);
        let missing = entries.iter().find(|e| e.id == first.id).unwrap();
        assert!(missing.path.is_none() && missing.dir.is_some());
        assert!(entries.iter().find(|e| e.id == second.id).unwrap().path.is_some());

        // Downloading it again brings the file back and keeps one record.
        assert_eq!(run(first, &dir), (true, false));
        assert!(archive_entries(&state).iter().find(|e| e.id == first.id).unwrap().path.is_some());
        assert_eq!(load_records(&dir).unwrap().iter().filter(|r| r.id == first.id).count(), 1);

        // With the file in place, yt-dlp leaves it and says so.
        assert_eq!(run(first, &dir), (true, true));

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
            classify_failure("ERROR: [youtube] abc: Sign in to confirm you’re not a bot. Use --cookies-from-browser"),
            Some("YouTube bot check")
        );
        // Progress lines must not be mistaken for failures.
        assert_eq!(classify_failure("[download]  42.0% of 10.00MiB at 2.00MiB/s"), None);
        assert_eq!(classify_failure("[youtube] Extracting URL"), None);
    }

    #[test]
    fn a_video_link_gives_its_id_in_every_form() {
        for link in [
            "https://www.youtube.com/watch?v=jNQXAC9IVRw",
            "youtube.com/watch?feature=share&v=jNQXAC9IVRw&t=4",
            "https://youtu.be/jNQXAC9IVRw?si=abc",
            "https://m.youtube.com/shorts/jNQXAC9IVRw",
            "https://www.youtube.com/live/jNQXAC9IVRw",
            "https://www.youtube-nocookie.com/embed/jNQXAC9IVRw",
        ] {
            assert_eq!(video_id(link).as_deref(), Some("jNQXAC9IVRw"), "{link}");
        }
        assert_eq!(video_id("https://www.youtube.com/@jawed"), None);
        assert_eq!(video_id("https://www.youtube.com/watch?v=short"), None);
    }

    // The parts of a members-only video's page that name it and its owner,
    // as YouTube served them on 2026-10-14 (the page is over a megabyte).
    const MEMBERS_PAGE: &str = r#"<meta name="title" content="Q&amp;A &quot;night&quot;"><link rel="canonical" href="https://www.youtube.com/watch?v=38QwNqMBQzg">
        "playabilityStatus":{"status":"UNPLAYABLE"},"canonicalBaseUrl":"/@someoneelse",
        "ownerProfileUrl":"http://www.youtube.com/@nara.asmr.","externalChannelId":"UCJXuQ9aRkS_HaFEBhiHFpWw","isUnlisted":false"#;

    #[test]
    fn a_video_page_names_its_owner_even_when_it_wont_play() {
        let r = page_ref(MEMBERS_PAGE, "38QwNqMBQzg").expect("the page names its owner");
        assert_eq!(r.channel_url, "https://www.youtube.com/@nara.asmr.");
        assert_eq!(r.channel_urls[1], "https://www.youtube.com/channel/UCJXuQ9aRkS_HaFEBhiHFpWw");
        assert_eq!(r.title, "Q&A \"night\"");
        assert_eq!(r.availability, None);
        let unlisted = MEMBERS_PAGE.replace("\"isUnlisted\":false", "\"isUnlisted\":true");
        assert_eq!(page_ref(&unlisted, "38QwNqMBQzg").unwrap().availability.as_deref(), Some("unlisted"));
        // Another video's page, or a page without the owner, is no answer.
        assert_eq!(page_ref(MEMBERS_PAGE, "jNQXAC9IVRw"), None);
        assert_eq!(page_ref(&MEMBERS_PAGE.replace("ownerProfileUrl", "x").replace("externalChannelId", "y"), "38QwNqMBQzg"), None);
    }

    #[test]
    fn oembed_names_the_channel_by_handle() {
        let r = oembed_ref(r#"{"title":"Me at the zoo","author_url":"https://www.youtube.com/@jawed"}"#, "jNQXAC9IVRw").unwrap();
        assert_eq!((r.channel_url.as_str(), r.title.as_str()), ("https://www.youtube.com/@jawed", "Me at the zoo"));
        assert_eq!(oembed_ref("Unauthorized", "jNQXAC9IVRw"), None);
    }

    #[test]
    #[ignore]
    fn a_real_video_page_names_its_channel() {
        let page = get_page("https://www.youtube.com/watch?v=jNQXAC9IVRw").expect("the page loads");
        let r = page_ref(&page, "jNQXAC9IVRw").expect("the page names its owner");
        assert_eq!(r.channel_url, "https://www.youtube.com/@jawed");
        assert_eq!(r.title, "Me at the zoo");
    }

    #[test]
    fn a_bot_check_on_lookup_says_what_to_do_in_the_app() {
        // Word for word as yt-dlp 2026.08.19 printed it, curly apostrophe and all.
        let bot = "WARNING: x\nERROR: [youtube] abc: Sign in to confirm you’re not a bot. Use --cookies-from-browser or --cookies";
        let none = lookup_failure(bot, None);
        assert!(none.contains("cookie menu") && !none.contains("--cookies"), "{none}");
        assert_eq!(lookup_failure(bot, Some("")), none);
        let set = lookup_failure(bot, Some("firefox"));
        assert!(set.contains("youtube.com in Firefox"), "{set}");
        // Anything else is yt-dlp's own last line.
        assert_eq!(lookup_failure("ERROR: [youtube] abc: Video unavailable\n", None), "ERROR: [youtube] abc: Video unavailable");
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
    fn a_rescan_adopts_finished_files_and_nothing_else() {
        let dir = std::env::temp_dir().join(format!("vault-adopt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = channel_archive_path(&dir);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        // One record whose file is gone: it stays, so the card can say so.
        std::fs::write(&file, "youtube missing0001\n").unwrap();
        std::fs::write(dir.join("Copied in by hand [copiedin001].mp4"), b"x").unwrap();
        std::fs::write(dir.join("Audio only [audioonly01].m4a"), b"x").unwrap();
        // Leftovers of a download that never merged, and things that only
        // look tagged.
        std::fs::write(dir.join("Half [fragment001].f137.mp4"), b"x").unwrap();
        std::fs::write(dir.join("Half [partpartpar].mp4.part"), b"x").unwrap();
        std::fs::write(dir.join("Cover [thumbnail01].jpg"), b"x").unwrap();
        std::fs::write(dir.join("Song [Official Video].mp4"), b"x").unwrap();

        let mut s = PersistedState::default();
        s.channels.push(SavedChannel {
            url: "https://www.youtube.com/@foo".into(),
            title: "Foo".into(),
            download_dir: Some(dir.to_string_lossy().into_owned()),
            ..Default::default()
        });
        assert_eq!(adopt_untracked(&s), 2);
        let ids = || load_records(&dir).unwrap().into_iter().map(|r| r.id).collect::<Vec<_>>();
        assert_eq!(ids(), vec!["missing0001", "audioonly01", "copiedin001"]);
        // Nothing new the second time.
        assert_eq!(adopt_untracked(&s), 0);
        assert_eq!(ids(), vec!["missing0001", "audioonly01", "copiedin001"]);
        // The folder it came from is left as it was.
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "youtube missing0001\n");

        let entries = archive_entries(&s);
        let by_id = |id: &str| entries.iter().find(|e| e.id == id).unwrap();
        assert_eq!(by_id("copiedin001").title, "Copied in by hand");
        assert!(by_id("missing0001").path.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_rescan_leaves_a_folder_with_nothing_to_adopt_untouched() {
        let dir = std::env::temp_dir().join(format!("vault-adopt-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        let mut s = PersistedState::default();
        s.channels.push(SavedChannel {
            url: "https://www.youtube.com/@foo".into(),
            download_dir: Some(dir.to_string_lossy().into_owned()),
            ..Default::default()
        });
        assert_eq!(adopt_untracked(&s), 0);
        assert!(!dir.join(".vault").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vault-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".vault")).unwrap();
        dir
    }

    fn channel_at(dir: &std::path::Path, videos: Vec<Video>) -> PersistedState {
        let mut s = PersistedState::default();
        s.channels.push(SavedChannel {
            url: "https://www.youtube.com/@foo".into(),
            title: "Foo".into(),
            download_dir: Some(dir.to_string_lossy().into_owned()),
            videos,
            ..Default::default()
        });
        s
    }

    #[test]
    fn a_folder_from_before_videos_json_is_imported_and_its_old_files_kept() {
        let dir = folder("import");
        let archive = "youtube dated000001\nyoutube undated0001\nyoutube dated000001\n";
        std::fs::write(channel_archive_path(&dir), archive).unwrap();
        std::fs::write(channel_dates_path(&dir), "dated000001 1700000000\nundated0001 NA\n").unwrap();

        // Readable before anything is written.
        assert!(!records_path(&dir).exists());
        assert_eq!(archive_entries(&channel_at(&dir, vec![])).len(), 2);

        // The first rescan writes it, with the dates carried over.
        adopt_untracked(&channel_at(&dir, vec![]));
        let records = load_records(&dir).unwrap();
        assert_eq!(records.len(), 2, "a repeated id is one record");
        assert_eq!((records[0].id.as_str(), records[0].timestamp), ("dated000001", Some(1700000000)));
        assert_eq!((records[1].id.as_str(), records[1].timestamp), ("undated0001", None));
        assert!(records_path(&dir).is_file());
        assert_eq!(std::fs::read_to_string(channel_archive_path(&dir)).unwrap(), archive);

        // From then on videos.json is the record, whatever the old file says.
        std::fs::write(channel_archive_path(&dir), "youtube latecomer01\n").unwrap();
        assert_eq!(load_records(&dir).unwrap().len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_damaged_videos_json_is_never_replaced() {
        let dir = folder("damaged");
        std::fs::write(records_path(&dir), "{ \"videos\": [ {\"id\": \"cut off").unwrap();
        std::fs::write(dir.join("New [copiedin001].mp4"), b"x").unwrap();
        let before = std::fs::read(records_path(&dir)).unwrap();

        let err = load_records(&dir).unwrap_err();
        assert!(err.contains("videos.json") && err.contains("move the file aside"), "{err}");
        assert_eq!(adopt_untracked(&channel_at(&dir, vec![])), 0);
        assert!(update_records(&dir, |r| upsert(r, Record::blank("abcdefghijk"))).is_err());
        assert!(record_download(&dir, "copiedin001", &dir.join("none")).is_err());
        assert_eq!(std::fs::read(records_path(&dir)).unwrap(), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_record_keeps_what_the_listing_knew_after_the_video_is_delisted() {
        let dir = folder("delisted");
        std::fs::write(dir.join("Name on disk [abcdefghijk].mp4"), b"x").unwrap();
        let listed = Video {
            id: "abcdefghijk".into(),
            title: "Listed title".into(),
            duration: Some(61.0),
            view_count: Some(1000),
            url: "https://www.youtube.com/watch?v=abcdefghijk".into(),
            availability: Some("subscriber_only".into()),
            timestamp: Some(5),
        };
        adopt_untracked(&channel_at(&dir, vec![listed]));
        let r = &load_records(&dir).unwrap()[0];
        assert_eq!(r.duration, Some(61.0));
        assert_eq!(r.availability.as_deref(), Some("subscriber_only"));
        assert_eq!(r.timestamp, None, "the listing's date is approximate and isn't kept as the real one");

        // The channel drops it. Length and views are still there.
        let entries = archive_entries(&channel_at(&dir, vec![]));
        assert_eq!((entries[0].duration, entries[0].view_count), (Some(61.0), Some(1000)));
        assert_eq!(entries[0].title, "Name on disk", "a delisted video is titled by its file");
        assert!(!std::fs::read_to_string(records_path(&dir)).unwrap().contains("title"), "no title is recorded");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_lookup_completes_records_and_archives_nothing() {
        let dir = folder("lookup");
        update_records(&dir, |r| upsert(r, Record::blank("undated0001")) | upsert(r, Record::blank("gone0000001"))).unwrap();
        assert_eq!(undated(&dir), vec!["undated0001", "gone0000001"]);

        let printed = r#"{"id": "undated0001", "title": "Found", "timestamp": 1600000000.0, "duration": 12, "view_count": 7, "availability": "public"}
{"id": "stranger001", "title": "Not ours", "timestamp": 1}
ERROR: [youtube] gone0000001: Video unavailable"#;
        assert_eq!(record_lookups(&dir, printed).unwrap(), 1);
        let records = load_records(&dir).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!((records[0].timestamp, records[0].duration, records[0].view_count), (Some(1600000000), Some(12.0), Some(7)));
        assert_eq!(undated(&dir), vec!["gone0000001"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_download_is_recorded_from_what_yt_dlp_printed_or_from_the_file() {
        let dir = folder("landed");
        let landed = dir.join(".vault").join(".landed-abcdefghijk.json");
        std::fs::write(&landed, r#"{"id": "abcdefghijk", "title": "Printed", "timestamp": 1440470706, "duration": 9}"#).unwrap();
        record_download(&dir, "abcdefghijk", &landed).unwrap();
        // Nothing printed, but the file is there.
        std::fs::write(dir.join("On disk [zzzzzzzzzzz].mp4"), b"x").unwrap();
        record_download(&dir, "zzzzzzzzzzz", &dir.join("none")).unwrap();
        // Neither: not archived, and the reason says what to do.
        let err = record_download(&dir, "nofile00001", &dir.join("none")).unwrap_err();
        assert!(err.contains("no video file") && err.contains("retry"), "{err}");

        let records = load_records(&dir).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!((records[0].timestamp, records[0].duration), (Some(1440470706), Some(9.0)));
        assert_eq!((records[1].id.as_str(), records[1].timestamp), ("zzzzzzzzzzz", None));
        assert!(records.iter().all(|r| r.downloaded_at.is_some()));
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
                timestamp: Some(1_700_000_000),
            }],
            ..Default::default()
        });
        let json = serde_json::to_string(&s).unwrap();
        let back: PersistedState = serde_json::from_str(&json).unwrap();
        assert_eq!(back.channels[0].videos[0].timestamp, Some(1_700_000_000));
        assert_eq!(back.active.as_deref(), Some("@kurzgesagt"));
        assert_eq!(back.channels[0].download_dir.as_deref(), Some("/tmp/videos"));
        assert_eq!(back.channels[0].videos[0].id, "abc12345678");
    }

    // The frontend sends the whole state back on every save, and a field this
    // side doesn't know is dropped on the way through.
    #[test]
    fn a_subscription_survives_what_the_frontend_saves() {
        let from_frontend = r#"{"active":null,"channels":[{"url":"u","title":"t","uploader":"t",
            "subscribe":{"videos":true,"shorts":false,"streams":true}},
            {"url":"v","title":"t","uploader":"t","subscribe":null}]}"#;
        let s: PersistedState = serde_json::from_str(from_frontend).unwrap();
        let back: PersistedState = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.channels[0].subscribe, Some(Subscription { videos: true, shorts: false, streams: true }));
        assert_eq!(back.channels[1].subscribe, None);
    }

    #[test]
    fn the_listing_carries_each_videos_approximate_date() {
        let root = serde_json::json!({ "entries": [
            { "id": "abcdefghijk", "title": "dated", "timestamp": 1790035200 },
            { "id": "zzzzzzzzzzz", "title": "undated" },
        ]});
        let info = parse_listing(&root, "u".into(), "videos".into());
        assert_eq!(info.videos[0].timestamp, Some(1790035200));
        assert_eq!(info.videos[1].timestamp, None);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .setup(|app| {
            // Without the player the rest of Vault still works, and Play says
            // why it can't.
            match app.get_webview_window("main") {
                Some(window) => {
                    if let Err(e) = player::init(&window) {
                        eprintln!("Player unavailable: {e}");
                    }
                }
                None => eprintln!("Player unavailable: the main window wasn't there at setup."),
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            fetch_channel,
            is_video_link,
            resolve_video,
            fetch_dates,
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
            player::player_load,
            player::player_set,
            player::player_state,
            player::player_place,
            player::player_fullscreen,
            player::player_still
        ])
        .run(tauri::generate_context!())
        .expect("error while running Vault");
}