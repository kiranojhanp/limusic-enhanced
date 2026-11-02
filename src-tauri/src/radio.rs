//! Internet radio, powered by [radio-browser.info](https://www.radio-browser.info).
//!
//! radio-browser is a community-maintained directory of ~50k public streams, with no key and no
//! account. That is the whole reason a radio page can exist without asking the user to hunt for a
//! stream URL: the browse grid *is* the directory.
//!
//! Three rules keep us a decent guest on a free service:
//! - a descriptive User-Agent (generic ones are blocked);
//! - a cache in front of every call (the facet lists are large and change daily, and a search the
//!   user typed twice is the same search);
//! - mirror fallback, because one mirror being down should not empty the page.
//!
//! A station plays through the same seam as a local file (see `local.rs`): its synthetic videoId is
//! `RADIO:<stationuuid>`, [`crate::state::AppState::resolve`] intercepts it, and the queue, media
//! keys and Shortcuts never learn that YouTube was not involved. The stream URL itself is
//! deliberately not serialized to the UI — the UI plays the id and Rust resolves it, so a station
//! that moved its stream keeps working.
//!
//! Two things go beyond the directory. A station can be saved as a favorite, or added as a custom
//! stream URL (probed first, then kept in SQLite), and the state layer resolves those without the
//! directory. [`stream_headers`] asks Icecast/Shoutcast for ICY metadata, which mpv surfaces through
//! its `metadata` property; [`split_stream_title`] and [`enrich`] turn that into the live now-playing
//! line and artwork.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use base64::Engine as _;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};

/// A station's synthetic videoId: this prefix + `stationuuid`. Same shape as `LOCAL:`.
pub const SONG_PREFIX: &str = "RADIO:";

/// The directory's mirrors. Tried in rotation, so a dead one costs a failed request rather than
/// every request on the page.
///
/// `all.api.radio-browser.info` is the directory's own round-robin name and is the one that keeps
/// working as individual mirrors come and go. The hostnames the first version listed
/// (`at1`/`nl1`/`us1`/`uk1`/`jp1`/`br1`/`eu1`/`ca1`) no longer resolve at all; keeping them only
/// made every request pay eight DNS failures before reaching a live one.
const MIRRORS: &[&str] = &[
    "https://de1.api.radio-browser.info",
    "https://de2.api.radio-browser.info",
    "https://all.api.radio-browser.info",
];

/// A mirror that just failed sits this long before it is tried in front of a healthy one. The
/// directory's hostnames come and go; probing one that is gone is free (DNS fails immediately),
/// but rotating it ahead of `de1` on every request is not.
const MIRROR_COOLDOWN: Duration = Duration::from_secs(10 * 60);

/// A browser User-Agent for stream requests. A good number of Icecast/Shoutcast servers refuse
/// mpv's own UA or serve it a lower bitrate; the directory's stream URLs are plain HTTP and expect
/// to be fetched by a player. The `Icy-MetaData` header is what asks the station to interleave its
/// "now playing" title, which mpv then surfaces through its `metadata` property.
pub const STREAM_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15";

/// Headers mpv is handed for a station: a browser UA, and the ICY request that carries the live
/// song title back. Everything else stays ffmpeg's defaults.
pub fn stream_headers() -> HashMap<String, String> {
    HashMap::from([
        ("User-Agent".to_owned(), STREAM_UA.to_owned()),
        ("Icy-MetaData".to_owned(), "1".to_owned()),
        ("Accept".to_owned(), "*/*".to_owned()),
    ])
}

/// How big a station logo may be before it is not worth proxying. The directory links to whatever
/// the broadcaster uploaded; a handful are multi-megabyte print masters.
const FAVICON_MAX: usize = 2 * 1024 * 1024;
const FAVICON_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// A failed logo is retried after this long, not on every recycled card.
const FAVICON_FAIL_TTL: Duration = Duration::from_secs(10 * 60);

/// They ask for the app's name, version and a way to reach it.
const UA: &str = concat!(
    "Limusic/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/kiranojhanp/limusic-enhanced)"
);

const TIMEOUT: Duration = Duration::from_secs(10);
/// Countries, languages and genres change at the pace of the directory's editors.
const FACET_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// Long enough that typing a name costs one request per pause, short enough that "most voted" is
/// not yesterday's list.
const SEARCH_TTL: Duration = Duration::from_secs(5 * 60);
/// How many query/response pairs to keep. A page of 60 stations is ~20 KB, so this is a few MB at
/// worst and covers a session's worth of filter combinations.
const CACHE_MAX: usize = 64;

/// One station as the directory describes it.
///
/// `url` and `url_resolved` are read from the directory but never sent to the UI: the UI plays
/// `RADIO:<stationuuid>` (see the module docs). Every other field is display metadata.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Station {
    pub stationuuid: String,
    pub name: String,
    #[serde(default, skip_serializing)]
    pub url: String,
    /// The playable URL: what `url` resolves to after the directory follows a `.pls`/`.m3u`.
    #[serde(default, skip_serializing)]
    pub url_resolved: String,
    #[serde(default)]
    pub homepage: String,
    #[serde(default)]
    pub favicon: String,
    #[serde(default)]
    pub tags: String,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub countrycode: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub votes: u32,
    #[serde(default)]
    pub codec: String,
    #[serde(default)]
    pub bitrate: u32,
    /// 0/1. An HLS station plays, but it carries no ICY metadata, so no live song title.
    #[serde(default)]
    pub hls: u8,
    /// 0/1: the directory's last reachability check.
    #[serde(default)]
    pub lastcheckok: u8,
    #[serde(default)]
    pub clickcount: u32,
}

impl Station {
    /// The URL to hand mpv: the resolved one, falling back to the raw one for rows the directory
    /// never resolved.
    pub fn stream_url(&self) -> &str {
        if self.url_resolved.is_empty() {
            &self.url
        } else {
            &self.url_resolved
        }
    }

    /// A station the user added by pasting a stream URL. It has no directory identity, so its
    /// `stationuuid` is a `custom-…` id this module owns; `url_resolved` carries the URL exactly as
    /// the user typed it (after [`probe`] followed its redirects). Everything else is what the
    /// player needs to draw a row.
    pub fn custom(uuid: String, name: String, url: String, codec: String) -> Self {
        Self {
            stationuuid: uuid,
            name,
            url: url.clone(),
            url_resolved: url,
            codec,
            lastcheckok: 1,
            ..Default::default()
        }
    }
}

/// A synthetic id for a station the user added themselves. `custom-` keeps it visibly apart from
/// the directory's UUIDs (and from anything a future version might generate).
pub fn custom_uuid() -> String {
    format!("custom-{:032x}", rand::random::<u128>())
}

pub fn is_custom_uuid(uuid: &str) -> bool {
    uuid.starts_with("custom-")
}

/// One entry of a filter list. `iso_3166_1` is set for countries only.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Facet {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub iso_3166_1: String,
    #[serde(default)]
    pub stationcount: u32,
}

/// Everything the browse page's filter row needs, in one round trip.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Facets {
    pub countries: Vec<Facet>,
    pub languages: Vec<Facet>,
    pub tags: Vec<Facet>,
}

/// What the user narrowed the grid by. Every field is optional; all empty means "most voted".
#[derive(Debug, Clone, Default)]
pub struct Query {
    pub name: Option<String>,
    pub country: Option<String>,
    pub language: Option<String>,
    pub tag: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

/// `RADIO:` ids are ours; nothing else in the app should ever see one as a videoId.
pub fn is_radio_song(video_id: &str) -> bool {
    video_id.starts_with(SONG_PREFIX)
}

/// The `stationuuid` inside a `RADIO:` id.
pub fn song_id(video_id: &str) -> Option<&str> {
    video_id.strip_prefix(SONG_PREFIX).filter(|id| !id.is_empty())
}

// --- the directory ----------------------------------------------------------------------------

struct Entry {
    at: Instant,
    body: String,
}

fn cache() -> &'static Mutex<HashMap<String, Entry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cache_get(key: &str, ttl: Duration) -> Option<String> {
    let mut c = cache().lock().ok()?;
    let hit = c.get(key)?;
    if hit.at.elapsed() > ttl {
        c.remove(key);
        return None;
    }
    Some(hit.body.clone())
}

fn cache_put(key: &str, body: &str) {
    let Ok(mut c) = cache().lock() else { return };
    if c.len() >= CACHE_MAX {
        // Drop expired entries first; if everything is still fresh, drop the oldest.
        c.retain(|_, e| e.at.elapsed() < SEARCH_TTL);
        if c.len() >= CACHE_MAX {
            if let Some(oldest) = c.iter().min_by_key(|(_, e)| e.at).map(|(k, _)| k.clone()) {
                c.remove(&oldest);
            }
        }
    }
    c.insert(key.to_owned(), Entry { at: Instant::now(), body: body.to_owned() });
}

/// When each mirror may be tried again, indexed like [`MIRRORS`]. Written only by [`fetch`].
fn mirror_health() -> &'static Mutex<[Option<Instant>; MIRRORS.len()]> {
    static HEALTH: OnceLock<Mutex<[Option<Instant>; MIRRORS.len()]>> = OnceLock::new();
    HEALTH.get_or_init(|| Mutex::new([None; MIRRORS.len()]))
}

fn mirror_ok(index: usize) {
    if let Ok(mut health) = mirror_health().lock() {
        health[index] = None;
    }
}

fn mirror_failed(index: usize) {
    if let Ok(mut health) = mirror_health().lock() {
        health[index] = Some(Instant::now() + MIRROR_COOLDOWN);
    }
}

fn mirror_available(index: usize) -> bool {
    mirror_health()
        .lock()
        .map(|health| health[index].is_none_or(|until| Instant::now() >= until))
        .unwrap_or(true)
}

/// One GET against the directory, cached and mirror-rotated.
async fn fetch(path: &str, ttl: Duration) -> Result<String, String> {
    if let Some(hit) = cache_get(path, ttl) {
        return Ok(hit);
    }
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let start = NEXT.fetch_add(1, Ordering::Relaxed) % MIRRORS.len();
    // Rotation still spreads requests, but a mirror that failed recently is tried after every
    // healthy one. If they are all cooling, the original order stands and each gets a chance.
    let mut order: Vec<usize> = (0..MIRRORS.len()).map(|i| (start + i) % MIRRORS.len()).collect();
    order.sort_by_key(|i| !mirror_available(*i));
    let mut last = String::from("radio-browser.info did not answer");
    for index in order {
        let base = MIRRORS[index];
        let sent = crate::http::client()
            .get(format!("{base}/json{path}"))
            .header(reqwest::header::USER_AGENT, UA)
            .timeout(TIMEOUT)
            .send()
            .await;
        match sent {
            Ok(r) if r.status().is_success() => match r.text().await {
                Ok(body) => {
                    mirror_ok(index);
                    cache_put(path, &body);
                    return Ok(body);
                }
                Err(e) => {
                    mirror_failed(index);
                    last = format!("{base}: request failed ({e})");
                }
            },
            Ok(r) => {
                mirror_failed(index);
                last = format!("{base} answered {}", r.status());
            }
            Err(e) => {
                mirror_failed(index);
                last = format!("{base}: {e}");
            }
        }
    }
    Err(last)
}

fn parse<T: for<'de> Deserialize<'de>>(body: &str) -> Result<T, String> {
    serde_json::from_str(body).map_err(|e| format!("unexpected reply from radio-browser.info: {e}"))
}

fn encode(params: &[(&str, String)]) -> String {
    params
        .iter()
        .map(|(k, v)| format!("{k}={}", urlencoding::encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// The landing list: the directory's most-voted stations, broken ones filtered out.
pub async fn discover(limit: u32) -> Result<Vec<Station>, String> {
    search(Query { limit: Some(limit), ..Default::default() }).await
}

/// Stations matching every filter that is set. `hidebroken` and the vote ordering are always on:
/// the first is what keeps a quarter of the directory from showing up as dead rows, and the second
/// is what makes an unfiltered request a "popular" list rather than an arbitrary page.
pub async fn search(q: Query) -> Result<Vec<Station>, String> {
    let mut params: Vec<(&str, String)> = vec![
        ("hidebroken", "true".into()),
        ("order", "votes".into()),
        ("reverse", "true".into()),
        ("limit", q.limit.unwrap_or(60).clamp(1, 200).to_string()),
        ("offset", q.offset.unwrap_or(0).to_string()),
    ];
    for (key, value) in
        [("name", q.name), ("country", q.country), ("language", q.language), ("tag", q.tag)]
    {
        if let Some(v) = value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()) {
            params.push((key, v));
        }
    }
    let path = format!("/stations/search?{}", encode(&params));
    let body = fetch(&path, SEARCH_TTL).await?;
    parse::<Vec<Station>>(&body)
}

/// One station by its stable id. A saved favorite is a snapshot; its stream may have moved since,
/// so playing one asks the directory again rather than trusting the stored URL.
pub async fn station(uuid: &str) -> Result<Option<Station>, String> {
    let path = format!("/stations/byuuid/{}", urlencoding::encode(uuid));
    let body = fetch(&path, SEARCH_TTL).await?;
    let list = parse::<Vec<Station>>(&body)?;
    Ok(list.into_iter().next())
}

/// Countries, languages and genres for the filter row, fetched together.
pub async fn facets() -> Result<Facets, String> {
    let countries_path = "/countries?hidebroken=true&order=stationcount&reverse=true";
    let languages_path = "/languages?hidebroken=true&order=stationcount&reverse=true";
    // The long tail of genres is thousands of near-empty tags; the head is what a chip row can
    // show and what anyone actually filters by.
    let tags_path = "/tags?hidebroken=true&order=stationcount&reverse=true&limit=80";

    let (countries, languages, tags) = tokio::join!(
        fetch(countries_path, FACET_TTL),
        fetch(languages_path, FACET_TTL),
        fetch(tags_path, FACET_TTL),
    );

    let countries = parse::<Vec<Facet>>(&countries?)?;
    let languages = parse::<Vec<Facet>>(&languages?)?;
    let tags = parse::<Vec<Facet>>(&tags?)?;

    Ok(Facets {
        // Same floor the directory's own UI uses: below a handful of stations a country chip
        // filters the grid down to nothing.
        countries: countries.into_iter().filter(|c| c.stationcount >= 5).collect(),
        languages,
        tags,
    })
}

/// What a pasted URL turned out to be. The caller stores `url` (after redirects) rather than what
/// the user typed, so a station that answers `302` to its real edge is saved at its final address.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Probe {
    pub url: String,
    pub content_type: String,
    pub icy_name: Option<String>,
}

/// Does a `Content-Type` (or, when the server lies, a URL path) describe something mpv can play?
fn looks_playable(content_type: &str, url: &str) -> bool {
    let ct = content_type.to_ascii_lowercase();
    if ct.starts_with("audio/")
        || ct.contains("mpegurl")
        || ct.contains("application/ogg")
        || ct.contains("audio/aac")
        || ct.contains("audio/ogg")
        || ct.contains("aacp")
        // Some Icecast servers answer a bare `application/octet-stream` and then speak MP3.
        || ct.starts_with("application/octet-stream")
    {
        return true;
    }
    // `.pls`/`.m3u` are often served as `text/plain`. The extension is the only signal left, and
    // ffmpeg reads both as a playlist.
    let path = url.split(['?', '#']).next().unwrap_or(url).to_ascii_lowercase();
    matches!(path.rsplit('.').next(), Some("m3u" | "m3u8" | "pls" | "mp3" | "aac" | "ogg" | "opus"))
}

/// Check a stream URL before it is saved (or played from the "add station" dialog). Follows
/// redirects, asks for one small range rather than a whole stream, and rejects the HTML error page
/// that a dead shoutcast host answers with — the failure a user otherwise only sees as a silent
/// player.
pub async fn probe(url: &str) -> Result<Probe, String> {
    let url = url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("The stream URL has to start with http:// or https://".to_owned());
    }
    let response = crate::http::client()
        .get(url)
        .header(reqwest::header::USER_AGENT, STREAM_UA)
        .header("Icy-MetaData", "1")
        .header(reqwest::header::ACCEPT, "*/*")
        .header(reqwest::header::RANGE, "bytes=0-4095")
        .timeout(Duration::from_secs(8))
        .send()
        .await
        .map_err(|e| format!("Could not reach that stream ({e})."))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("That stream answered HTTP {status}."));
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let final_url = response.url().to_string();
    let icy_name = response
        .headers()
        .get("icy-name")
        .or_else(|| response.headers().get("ice-name"))
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned);

    if !looks_playable(&content_type, &final_url) {
        let shown = if content_type.is_empty() { "no content type" } else { &content_type };
        return Err(format!("That URL is {shown}, not an audio stream."));
    }
    Ok(Probe { url: final_url, content_type, icy_name })
}

/// A station logo as a `data:` URL, for hosts the webview refuses to load (plain `http://` under a
/// secure origin, or a hotlink-blocked image). Cached in memory; a failure is cached briefly so a
/// grid of broken logos does not keep retrying.
pub async fn favicon_data_url(url: &str) -> Result<String, String> {
    fn cache() -> &'static Mutex<HashMap<String, (Instant, Option<String>)>> {
        static CACHE: OnceLock<Mutex<HashMap<String, (Instant, Option<String>)>>> = OnceLock::new();
        CACHE.get_or_init(|| Mutex::new(HashMap::new()))
    }

    let url = url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("not an http image".to_owned());
    }
    if let Ok(c) = cache().lock() {
        if let Some((at, hit)) = c.get(url) {
            let ttl = if hit.is_some() { FAVICON_TTL } else { FAVICON_FAIL_TTL };
            if at.elapsed() < ttl {
                return hit.clone().ok_or_else(|| "logo unavailable".to_owned());
            }
        }
    }

    let fetched = async {
        let response = crate::http::client()
            .get(url)
            .header(reqwest::header::USER_AGENT, STREAM_UA)
            .header(reqwest::header::ACCEPT, "image/*")
            .timeout(Duration::from_secs(8))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("logo answered {}", response.status()));
        }
        if let Some(length) = response.content_length() {
            if length as usize > FAVICON_MAX {
                return Err("logo is too large".to_owned());
            }
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !content_type.is_empty() && !content_type.starts_with("image/") {
            return Err(format!("logo is {content_type}"));
        }

        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| e.to_string())?;
            if bytes.len() + chunk.len() > FAVICON_MAX {
                return Err("logo is too large".to_owned());
            }
            bytes.extend_from_slice(&chunk);
        }
        if bytes.is_empty() {
            return Err("empty logo".to_owned());
        }

        // The server's type is the best answer, but only when it named an image. Otherwise sniff
        // the magic bytes so the data URL still has a type an <img> will render.
        let mime = if content_type.starts_with("image/") {
            content_type.split(';').next().unwrap_or("image/png").trim().to_owned()
        } else if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
            "image/png".to_owned()
        } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
            "image/jpeg".to_owned()
        } else if bytes.starts_with(b"GIF8") {
            "image/gif".to_owned()
        } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
            "image/webp".to_owned()
        } else if bytes.iter().take(64).copied().any(|b| b == b'<') {
            "image/svg+xml".to_owned()
        } else {
            "image/png".to_owned()
        };
        Ok::<String, String>(format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&bytes)
        ))
    }
    .await;

    if let Ok(mut c) = cache().lock() {
        if c.len() >= 256 {
            c.clear();
        }
        c.insert(url.to_owned(), (Instant::now(), fetched.as_ref().ok().cloned()));
    }
    fetched
}

/// A stream's ICY title split into `(artist, title)`. Most stations send `Artist - Title`; some
/// prefix it, and a few send only a title. Both separators the broadcast world uses (hyphen and
/// en/em dash) and a pipe are accepted, because choosing one leaves the other glued to the artist
/// name and ruins the artwork lookup.
pub fn split_stream_title(raw: &str) -> (Option<String>, Option<String>) {
    let raw = raw.trim().trim_matches('\0').trim();
    let raw = raw
        .strip_prefix("Now playing:")
        .or_else(|| raw.strip_prefix("Now Playing:"))
        .unwrap_or(raw)
        .trim();
    if raw.is_empty() {
        return (None, None);
    }
    for separator in [" - ", " – ", " — ", " | "] {
        if let Some((artist, title)) = raw.split_once(separator) {
            let (artist, title) = (artist.trim(), title.trim());
            if !artist.is_empty() && !title.is_empty() {
                return (Some(artist.to_owned()), Some(title.to_owned()));
            }
        }
    }
    (None, Some(raw.to_owned()))
}

/// What iTunes knew about an ICY title. Every field is optional: the lookup is an enhancement, and
/// a miss must leave the raw stream title on screen rather than blank it.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Enriched {
    pub artist: Option<String>,
    pub title: Option<String>,
    pub album: Option<String>,
    pub cover: Option<String>,
    pub link: Option<String>,
}

/// iTunes Search lookup for a radio "now playing" line (the same enrichment Radiocove does). Cached
/// by `(artist, title)`: a station repeats the same song for minutes, and the lookup fires on every
/// ICY change, not every position tick.
pub async fn enrich(artist: &str, title: &str) -> Option<Enriched> {
    fn cache() -> &'static Mutex<HashMap<String, (Instant, Option<Enriched>)>> {
        static CACHE: OnceLock<Mutex<HashMap<String, (Instant, Option<Enriched>)>>> =
            OnceLock::new();
        CACHE.get_or_init(|| Mutex::new(HashMap::new()))
    }
    const TTL: Duration = Duration::from_secs(24 * 60 * 60);

    let key = format!("{}\u{1}{}", artist.trim().to_lowercase(), title.trim().to_lowercase());
    if let Ok(c) = cache().lock() {
        if let Some((at, hit)) = c.get(&key) {
            if at.elapsed() < TTL {
                return hit.clone();
            }
        }
    }

    let term = format!("{} {}", artist.trim(), title.trim());
    let response = crate::http::client()
        .get("https://itunes.apple.com/search")
        .query(&[("term", term.as_str()), ("limit", "1"), ("media", "music")])
        .header(reqwest::header::USER_AGENT, UA)
        .timeout(Duration::from_secs(6))
        .send()
        .await
        .ok()?;
    let json: serde_json::Value = response.json().await.ok()?;
    let result = json.get("results")?.as_array()?.first()?;
    let field = |name: &str| {
        result
            .get(name)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
    };
    let enriched = Enriched {
        artist: field("artistName"),
        title: field("trackName"),
        album: field("collectionName"),
        cover: field("artworkUrl100").map(|u| u.replace("100x100", "600x600")),
        link: field("trackViewUrl"),
    };

    if let Ok(mut c) = cache().lock() {
        if c.len() >= 512 {
            c.clear();
        }
        c.insert(key, (Instant::now(), Some(enriched.clone())));
    }
    Some(enriched)
}

/// The `PlaybackData` for a station, shaped like [`crate::local::playback_data`]: no expiry, no
/// watch-history ping, no loudness metadata, and a `stream_client` that says where it came from.
pub fn playback_data(video_id: &str, station: &Station) -> crate::orchestrator::PlaybackData {
    crate::orchestrator::PlaybackData {
        video_id: video_id.to_owned(),
        stream_url: station.stream_url().to_owned(),
        itag: 0,
        headers: stream_headers(),
        // A live stream never expires and never enters the URL cache (see `AppState::resolve`).
        expires_in_seconds: i64::MAX / 2,
        loudness_db: None,
        playback_ping: None,
        title: Some(station.name.clone()),
        artists: None,
        // No length: a station has no end. The player bar hides its timeline on that.
        duration: None,
        thumbnail: Some(station.favicon.clone()).filter(|f| !f.is_empty()),
        is_video: None,
        is_episode: None,
        stream_client: "radio".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn song_ids_round_trip() {
        let id = format!("{SONG_PREFIX}961f4c9b-2b3a-4a5e-9d0f-1c2d3e4f5a6b");
        assert!(is_radio_song(&id));
        assert_eq!(song_id(&id), Some("961f4c9b-2b3a-4a5e-9d0f-1c2d3e4f5a6b"));
        // The prefix alone is not a station.
        assert_eq!(song_id(SONG_PREFIX), None);
        assert!(!is_radio_song("dQw4w9WgXcQ"));
        assert!(!is_radio_song("LOCAL:/music/a.mp3"));
    }

    #[test]
    fn a_row_without_a_resolved_url_falls_back_to_the_raw_one() {
        let mut s = Station { url: "http://a/stream".into(), ..Default::default() };
        assert_eq!(s.stream_url(), "http://a/stream");
        s.url_resolved = "https://a/stream".into();
        assert_eq!(s.stream_url(), "https://a/stream");
    }

    #[test]
    fn the_stream_url_is_not_sent_to_the_ui() {
        let s = Station {
            stationuuid: "u".into(),
            name: "Jazz FM".into(),
            url: "http://secret/stream".into(),
            url_resolved: "http://secret/stream".into(),
            ..Default::default()
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("secret"), "the UI plays RADIO:<uuid>, it never sees a stream URL");
        assert!(json.contains("Jazz FM"));
    }

    #[test]
    fn icy_titles_split_on_the_separators_stations_actually_send() {
        assert_eq!(
            split_stream_title("Daft Punk - Get Lucky"),
            (Some("Daft Punk".into()), Some("Get Lucky".into()))
        );
        assert_eq!(
            split_stream_title("  Now playing: Daft Punk – Get Lucky  "),
            (Some("Daft Punk".into()), Some("Get Lucky".into()))
        );
        assert_eq!(
            split_stream_title("Nils Frahm | Says"),
            (Some("Nils Frahm".into()), Some("Says".into()))
        );
        // A station that sends only a title keeps it, with no invented artist.
        assert_eq!(split_stream_title("Evening Jazz"), (None, Some("Evening Jazz".into())));
        assert_eq!(split_stream_title("   "), (None, None));
    }

    #[test]
    fn playable_content_types_and_playlist_extensions_are_accepted() {
        assert!(looks_playable("audio/mpeg", "http://a/stream"));
        assert!(looks_playable("audio/aacp", "http://a/stream"));
        assert!(looks_playable("application/vnd.apple.mpegurl", "http://a/list"));
        assert!(looks_playable("text/plain", "http://a/station.pls"));
        assert!(looks_playable("application/octet-stream", "http://a/stream"));
        // The classic dead-station answer: an HTML error page on the old stream URL.
        assert!(!looks_playable("text/html; charset=utf-8", "http://a/stream"));
        assert!(!looks_playable("image/png", "http://a/logo.png"));
        assert!(!looks_playable("application/json", "http://a/api"));
    }

    #[test]
    fn custom_ids_are_ours_and_do_not_collide() {
        let a = custom_uuid();
        let b = custom_uuid();
        assert!(a.starts_with("custom-"));
        assert!(is_custom_uuid(&a));
        assert_ne!(a, b);
        assert!(!is_custom_uuid("961f4c9b-2b3a-4a5e-9d0f-1c2d3e4f5a6b"));
    }

    #[test]
    fn stream_headers_ask_for_icy_metadata() {
        let headers = stream_headers();
        assert_eq!(headers.get("Icy-MetaData").map(String::as_str), Some("1"));
        assert!(headers.get("User-Agent").is_some_and(|ua| ua.contains("Mozilla")));
    }
}
