//! Internet radio: stations from the Radio Browser directory
//! (radio-browser.info, a free community list), and the user's favourites.
//!
//! The directory is only asked when the user opens Internet radio or
//! searches it. What's sent is the search itself; no account or id.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// The directory's servers, in the order tried. The first is a round robin
/// over all of them; the others are for when it can't be reached.
const HOSTS: &[&str] = &["all.api.radio-browser.info", "de1.api.radio-browser.info", "de2.api.radio-browser.info"];
const LIMIT: usize = 80;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Station {
    /// The directory's id for the station.
    pub id: String,
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub codec: String,
    /// kbit/s, 0 when unknown.
    #[serde(default)]
    pub bitrate: u32,
}

/// A station as the directory lists it.
#[derive(Deserialize)]
struct Listed {
    #[serde(default)]
    stationuuid: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    url_resolved: String,
    #[serde(default)]
    countrycode: String,
    #[serde(default)]
    tags: String,
    #[serde(default)]
    codec: String,
    #[serde(default)]
    bitrate: i64,
}

/// Whether a stream address is one to hand to the player.
pub fn playable(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// Reads the directory's answer. Entries without a name or a web address
/// are left out, and a station listed twice is kept once.
fn parse(json: &[u8]) -> Result<Vec<Station>, String> {
    let listed: Vec<Listed> = serde_json::from_slice(json).map_err(|_| "The radio directory sent something unexpected".to_string())?;
    let mut stations: Vec<Station> = Vec::new();
    for s in listed {
        let name = s.name.split_whitespace().collect::<Vec<_>>().join(" ");
        // The resolved address is the stream itself, past any playlist file.
        let url = if playable(&s.url_resolved) { s.url_resolved } else { s.url };
        if name.is_empty() || s.stationuuid.is_empty() || !playable(&url) || stations.iter().any(|x| x.id == s.stationuuid || x.url == url) {
            continue;
        }
        stations.push(Station {
            id: s.stationuuid,
            name: name.chars().take(120).collect(),
            url,
            country: s.countrycode.to_ascii_uppercase(),
            tags: s.tags.split(',').map(str::trim).filter(|t| !t.is_empty()).take(4).map(String::from).collect(),
            codec: if s.codec.eq_ignore_ascii_case("unknown") { String::new() } else { s.codec },
            bitrate: s.bitrate.clamp(0, 100_000) as u32,
        });
    }
    Ok(stations)
}

/// Percent-encodes a search for a query string.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The request for a search by name, a genre, or (with neither) the most
/// played stations.
fn request_path(query: &str, tag: &str) -> String {
    let mut path = format!("/json/stations/search?limit={LIMIT}&hidebroken=true&order=clickcount&reverse=true");
    let (query, tag) = (query.trim(), tag.trim());
    if !query.is_empty() {
        path.push_str(&format!("&name={}", encode(query)));
    }
    if !tag.is_empty() {
        path.push_str(&format!("&tag={}", encode(&tag.to_lowercase())));
    }
    path
}

fn client() -> Option<&'static reqwest::Client> {
    static CLIENT: OnceLock<Option<reqwest::Client>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            // The updater and analytics install the same provider; whichever runs first wins.
            if rustls::crypto::CryptoProvider::get_default().is_none() {
                let _ = rustls::crypto::ring::default_provider().install_default();
            }
            // The directory asks apps to name themselves.
            let agent = concat!("ReVLC/", env!("CARGO_PKG_VERSION"));
            reqwest::Client::builder().timeout(Duration::from_secs(10)).user_agent(agent).build().ok()
        })
        .as_ref()
}

/// Stations matching a name and a genre; with neither, the most played.
pub async fn stations(query: &str, tag: &str) -> Result<Vec<Station>, String> {
    let offline = "Couldn't reach the radio directory. Check your internet connection";
    let client = client().ok_or(offline)?;
    let path = request_path(query, tag);
    for host in HOSTS {
        let Ok(response) = client.get(format!("https://{host}{path}")).send().await else { continue };
        if !response.status().is_success() {
            continue;
        }
        if let Ok(body) = response.bytes().await {
            return parse(&body);
        }
    }
    Err(offline.into())
}

pub struct Favorites {
    file: PathBuf,
    pub list: Vec<Station>,
}

impl Favorites {
    pub fn load(data_dir: &Path) -> Self {
        let file = data_dir.join("radio.json");
        let list = crate::store::read_list(&file);
        Self { file, list }
    }

    /// Adds or removes a favourite, keeping the old list if saving fails.
    pub fn set(&mut self, station: Station, on: bool) -> Result<(), String> {
        if !playable(&station.url) {
            return Err("That station has no address to play".into());
        }
        let mut list: Vec<Station> = self.list.iter().filter(|s| s.id != station.id).cloned().collect();
        if on {
            list.push(station);
            list.sort_by(|a, b| crate::library::natural_cmp(&a.name, &b.name));
        }
        let old = std::mem::replace(&mut self.list, list);
        crate::store::write_json(&self.file, &self.list, false).inspect_err(|_| self.list = old)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANSWER: &str = r#"[
        {"stationuuid":"a1","name":"  BBC  World Service ","url":"http://example.org/bbc.m3u","url_resolved":"https://stream.example.org/bbc",
         "countrycode":"gb","tags":"news, talk,,world,bbc,english","codec":"MP3","bitrate":56,"votes":17541,"something_new":true},
        {"stationuuid":"a2","name":"Jazz FM","url":"https://example.org/jazz","url_resolved":"","countrycode":"","tags":"","codec":"UNKNOWN","bitrate":-1},
        {"stationuuid":"a2","name":"Jazz FM again","url":"https://example.org/jazz2","url_resolved":""},
        {"stationuuid":"a3","name":"Same stream","url":"https://example.org/jazz","url_resolved":""},
        {"stationuuid":"a4","name":"","url":"https://example.org/unnamed"},
        {"stationuuid":"a5","name":"Not the web","url":"file:///etc/passwd","url_resolved":"mms://example.org/x"}
    ]"#;

    #[test]
    fn the_directory_answer_becomes_stations() {
        let stations = parse(ANSWER.as_bytes()).unwrap();
        assert_eq!(stations, [
            Station {
                id: "a1".into(),
                name: "BBC World Service".into(),
                url: "https://stream.example.org/bbc".into(),
                country: "GB".into(),
                tags: vec!["news".into(), "talk".into(), "world".into(), "bbc".into()],
                codec: "MP3".into(),
                bitrate: 56,
            },
            Station { id: "a2".into(), name: "Jazz FM".into(), url: "https://example.org/jazz".into(), country: String::new(), tags: Vec::new(), codec: String::new(), bitrate: 0 },
        ]);
        assert!(parse(b"<html>down for maintenance</html>").is_err());
        assert_eq!(parse(b"[]").unwrap(), []);
    }

    #[test]
    fn searches_are_encoded() {
        assert_eq!(request_path("", ""), format!("/json/stations/search?limit={LIMIT}&hidebroken=true&order=clickcount&reverse=true"));
        assert!(request_path(" radio 1 & 2 ", "").ends_with("&name=radio%201%20%26%202"));
        assert!(request_path("", " Hip Hop ").ends_with("&tag=hip%20hop"));
        assert!(request_path("fm", "jazz").ends_with("&name=fm&tag=jazz"));
    }

    #[test]
    fn favourites_are_kept_between_launches() {
        let dir = std::env::temp_dir().join(format!("revlc-radio-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let station = |id: &str, name: &str| Station {
            id: id.into(),
            name: name.into(),
            url: format!("https://example.org/{id}"),
            country: "IN".into(),
            tags: vec!["news".into()],
            codec: "AAC".into(),
            bitrate: 64,
        };
        let mut favorites = Favorites::load(&dir);
        favorites.set(station("b", "Radio Two"), true).unwrap();
        favorites.set(station("a", "Radio One"), true).unwrap();
        // Starring one twice keeps it once.
        favorites.set(station("a", "Radio One"), true).unwrap();
        assert_eq!(Favorites::load(&dir).list, [station("a", "Radio One"), station("b", "Radio Two")]);
        favorites.set(station("b", "Radio Two"), false).unwrap();
        assert_eq!(Favorites::load(&dir).list, [station("a", "Radio One")]);
        // Nothing that isn't a web stream gets saved, whatever asks.
        let mut local = station("c", "Local file");
        local.url = "file:///etc/passwd".into();
        assert!(favorites.set(local, true).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
