//! Network shares: mounted network volumes, saved servers, and browsing
//! folders on a server.
//!
//! libVLC browses FTP, SFTP, NFS and UPnP itself. SMB goes through the
//! operating system instead: VLC 3's macOS build has no SMB support, and the
//! system's own mounting brings its sign-in window and keychain with it.
//! macOS mounts the share and Windows reads it as a `\\server\share` path;
//! either way it's then browsed as an ordinary folder.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::library::{is_media, natural_cmp};
use crate::playlist::{percent_decode, title_for};
use crate::engine::{BrowseEntry, Engine};

/// How long a server gets to list a folder.
const BROWSE_TIMEOUT_MS: i32 = 20_000;
/// Folders with more entries than this are cut short.
const MAX_ENTRIES: usize = 5_000;

/// Addresses the "Connect to server" box takes.
const SCHEMES: &[&str] = &["smb", "afp", "ftp", "ftps", "ftpes", "sftp", "nfs", "upnp"];

/// A server the user saved.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Server {
    pub url: String,
    pub name: String,
    /// The user name to sign in with. Passwords are never saved.
    #[serde(default)]
    pub user: Option<String>,
}

/// A network volume the system has mounted.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Place {
    pub name: String,
    pub path: String,
}

/// The contents of a folder.
#[derive(Clone, Debug, Serialize)]
pub struct Listing {
    /// Where this is: a local path or a URL without a password.
    pub location: String,
    pub name: String,
    /// A folder on this computer (a mounted share), which can join the library.
    pub local: bool,
    pub entries: Vec<BrowseEntry>,
}

pub struct Servers {
    file: PathBuf,
    pub list: Vec<Server>,
}

impl Servers {
    pub fn load(data_dir: &Path) -> Self {
        let file = data_dir.join("servers.json");
        let list = crate::store::read_list(&file);
        Self { file, list }
    }

    fn replace(&mut self, list: Vec<Server>) -> Result<(), String> {
        let old = std::mem::replace(&mut self.list, list);
        crate::store::write_json(&self.file, &self.list, false).inspect_err(|_| self.list = old)
    }

    /// Saves a server, replacing one with the same address.
    pub fn save(&mut self, url: &str, name: &str, user: Option<String>) -> Result<(), String> {
        let url = strip_credentials(&clean_url(url)?);
        let name = name.trim();
        let name = if name.is_empty() { display_name(&url) } else { name.to_string() };
        let user = user.map(|u| u.trim().to_string()).filter(|u| !u.is_empty());
        let mut list: Vec<Server> = self.list.iter().filter(|s| s.url != url).cloned().collect();
        list.push(Server { url, name, user });
        list.sort_by(|a, b| natural_cmp(&a.name, &b.name));
        self.replace(list)
    }

    pub fn remove(&mut self, url: &str) -> Result<(), String> {
        let list = self.list.iter().filter(|s| s.url != url).cloned().collect();
        self.replace(list)
    }
}

/// The scheme of a URL, lower case.
fn scheme(url: &str) -> Option<String> {
    url.split_once("://").map(|(s, _)| s.to_ascii_lowercase())
}

/// An address as typed, made into a URL: `\\nas\films` and `nas/films`
/// are SMB shares.
pub fn clean_url(input: &str) -> Result<String, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("Enter the server's address".into());
    }
    let url = match scheme(input) {
        Some(_) => input.to_string(),
        None => format!("smb://{}", input.trim_start_matches(['\\', '/']).replace('\\', "/")),
    };
    let scheme = scheme(&url).unwrap_or_default();
    if !SCHEMES.contains(&scheme.as_str()) {
        return Err(format!("ReVLC can't browse {scheme}:// addresses. Use smb://, ftp://, sftp://, nfs:// or upnp://"));
    }
    let (_, rest) = url.split_once("://").unwrap_or_default();
    if rest.split(['/', '?']).next().unwrap_or_default().is_empty() {
        return Err("Enter the server's name or address".into());
    }
    Ok(format!("{scheme}://{rest}"))
}

/// Percent-encodes a user name or password for a URL.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Where a URL's `user:password@` ends, if it has one.
fn userinfo_end(rest: &str) -> Option<usize> {
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    authority.rfind('@')
}

/// The URL without any user name and password.
pub fn strip_credentials(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else { return url.to_string() };
    // UPnP addresses carry a whole URL after the scheme; leave them be.
    if scheme.eq_ignore_ascii_case("upnp") {
        return url.to_string();
    }
    match userinfo_end(rest) {
        Some(at) => format!("{scheme}://{}", &rest[at + 1..]),
        None => url.to_string(),
    }
}

/// The URL signing in as `user`, for libVLC. Not kept anywhere.
pub fn with_credentials(url: &str, user: &str, password: &str) -> String {
    let user = user.trim();
    if user.is_empty() {
        return url.to_string();
    }
    let clean = strip_credentials(url);
    let Some((scheme, rest)) = clean.split_once("://") else { return clean };
    let secret = if password.is_empty() { String::new() } else { format!(":{}", encode(password)) };
    format!("{scheme}://{}{secret}@{rest}", encode(user))
}

/// A short name for a place: the last folder, or the server.
pub fn display_name(location: &str) -> String {
    let clean = strip_credentials(location);
    let trimmed = clean.trim_end_matches(['/', '\\']);
    match trimmed.split_once("://") {
        Some((_, rest)) if !rest.contains('/') => rest.to_string(),
        _ => title_for(trimmed),
    }
}

/// The Windows path of an SMB share: `smb://nas/films/new` is
/// `\\nas\films\new`. `None` without a share, which Windows can't list.
pub fn unc_path(url: &str) -> Option<String> {
    let clean = strip_credentials(url);
    let rest = clean.split_once("://")?.1.trim_end_matches('/');
    let (host, path) = rest.split_once('/')?;
    let host = match host.strip_prefix('[').and_then(|h| h.split_once(']')) {
        // Windows paths can't hold an IPv6 address's colons; it has a name for them instead.
        Some((address, _port)) => format!("{}.ipv6-literal.net", address.replace(':', "-").replace("%25", "s").replace('%', "s")),
        // A port isn't part of a Windows path.
        None => host.split(':').next().unwrap_or(host).to_string(),
    };
    if host.is_empty() || host.starts_with('.') || path.is_empty() {
        return None;
    }
    Some(format!("\\\\{host}\\{}", percent_decode(path).replace('/', "\\")))
}

/// Network volumes the system has mounted.
pub fn mounted() -> Vec<Place> {
    let mut places = mounts();
    places.sort_by(|a, b| natural_cmp(&a.name, &b.name));
    places.dedup();
    places
}

#[cfg(target_os = "macos")]
fn mounts() -> Vec<Place> {
    const NETWORK: &[&str] = &["smbfs", "nfs", "afpfs", "webdav", "ftp", "cifs"];
    let text = |chars: &[libc::c_char]| {
        let bytes: Vec<u8> = chars.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
        String::from_utf8_lossy(&bytes).into_owned()
    };
    // getfsstat fills a table of our own. (getmntinfo's is shared by every
    // caller in the process, and two listings at once would trample it.)
    // SAFETY: a null table asks only for the number of mounts.
    let count = unsafe { libc::getfsstat(std::ptr::null_mut(), 0, libc::MNT_NOWAIT) };
    if count <= 0 {
        return Vec::new();
    }
    // Room for a few more, in case something is mounted in between.
    let room = count as usize + 8;
    let mut table: Vec<libc::statfs> = Vec::with_capacity(room);
    let bytes = (room * std::mem::size_of::<libc::statfs>()) as libc::c_int;
    // SAFETY: the table has room for `bytes`; getfsstat says how many entries it wrote.
    let filled = unsafe { libc::getfsstat(table.as_mut_ptr(), bytes, libc::MNT_NOWAIT) };
    if filled <= 0 {
        return Vec::new();
    }
    // SAFETY: the first `filled` entries were written, and never more than fit.
    unsafe { table.set_len((filled as usize).min(room)) };
    table
        .iter()
        .filter(|m| NETWORK.contains(&text(&m.f_fstypename).as_str()))
        .map(|m| text(&m.f_mntonname))
        // Shares the system mounts for itself aren't the user's.
        .filter(|path| path.starts_with("/Volumes/"))
        .map(|path| Place { name: title_for(&path), path })
        .collect()
}

#[cfg(target_os = "linux")]
fn mounts() -> Vec<Place> {
    const NETWORK: &[&str] = &["cifs", "smb3", "nfs", "nfs4", "fuse.sshfs", "davfs", "fuse.gvfsd-fuse"];
    let Ok(table) = std::fs::read_to_string("/proc/mounts") else { return Vec::new() };
    table
        .lines()
        .filter_map(|line| {
            let mut fields = line.split(' ');
            let (path, kind) = (fields.nth(1)?, fields.next()?);
            // Spaces in a mount point are written as \040.
            let path = path.replace("\\040", " ");
            NETWORK.contains(&kind).then(|| Place { name: title_for(&path), path })
        })
        .collect()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn mounts() -> Vec<Place> {
    Vec::new()
}

/// Mounts a share the way Finder's "Connect to Server" does, with the
/// system's sign-in window if it needs one, and returns where it is.
#[cfg(target_os = "macos")]
fn mount(url: &str) -> Result<PathBuf, String> {
    // The address is an argument, not part of the script, so nothing in it runs.
    let output = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", "on run argv", "-e", "return POSIX path of (mount volume (item 1 of argv))", "-e", "end run", "--", url])
        .output()
        .map_err(|e| format!("Couldn't connect: {e}"))?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        // -128: the sign-in window was cancelled.
        return Err(if error.contains("-128") { "Cancelled".into() } else { "Couldn't connect to that server. Check the address and that it's turned on".to_string() });
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() || !Path::new(&path).is_dir() {
        return Err("The server connected, but its folder couldn't be found".into());
    }
    Ok(PathBuf::from(path.trim_end_matches('/')))
}

/// The folders and media files in a local folder, folders first.
pub fn list_dir(dir: &Path) -> Result<Vec<BrowseEntry>, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("Couldn't open {}: {e}", dir.display()))?;
    let mut out: Vec<BrowseEntry> = entries
        .flatten()
        .take(MAX_ENTRIES)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                return None;
            }
            let path = entry.path();
            let dir = path.is_dir();
            (dir || is_media(&path)).then(|| BrowseEntry { uri: path.to_string_lossy().into_owned(), name, dir })
        })
        .collect();
    sort(&mut out);
    Ok(out)
}

fn sort(entries: &mut [BrowseEntry]) {
    entries.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| natural_cmp(&a.name, &b.name)));
}

fn local(path: &Path) -> Result<Listing, String> {
    let location = path.to_string_lossy().into_owned();
    Ok(Listing { name: display_name(&location), entries: list_dir(path)?, location, local: true })
}

/// Lists a folder: a local path, or a server's address. `user` and
/// `password` sign in to servers the engine browses; the system asks for its own.
pub fn browse(engine: &dyn Engine, location: &str, user: &str, password: &str) -> Result<Listing, String> {
    if !location.contains("://") && Path::new(location).is_absolute() {
        return local(Path::new(location));
    }
    let url = clean_url(location)?;
    let scheme = scheme(&url).unwrap_or_default();
    #[cfg(target_os = "macos")]
    {
        if scheme == "smb" || scheme == "afp" {
            return local(&mount(&url)?);
        }
    }
    if cfg!(windows) && scheme == "smb" {
        let path = unc_path(&url).ok_or("Add the shared folder's name, like smb://computer/folder")?;
        return local(Path::new(&path));
    }
    let listed = engine.browse(&with_credentials(&url, user, password), BROWSE_TIMEOUT_MS)?;
    // File servers list everything; media servers (UPnP) only list media,
    // often at addresses with no file name to go by.
    let files = scheme != "upnp";
    let mut entries: Vec<BrowseEntry> = listed
        .into_iter()
        .take(MAX_ENTRIES)
        .map(|e| BrowseEntry { uri: strip_credentials(&e.uri), ..e })
        .filter(|e| !e.name.starts_with('.'))
        .filter(|e| e.dir || !files || is_media(Path::new(&percent_decode(e.uri.split(['?', '#']).next().unwrap_or_default()))))
        .collect();
    if files {
        sort(&mut entries);
    }
    let location = strip_credentials(&url);
    Ok(Listing { name: display_name(&location), location, local: false, entries })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_as_typed() {
        assert_eq!(clean_url("  SMB://nas/films ").unwrap(), "smb://nas/films");
        assert_eq!(clean_url("\\\\nas\\films").unwrap(), "smb://nas/films");
        assert_eq!(clean_url("nas.local/films").unwrap(), "smb://nas.local/films");
        assert_eq!(clean_url("ftp://ftp.example.org").unwrap(), "ftp://ftp.example.org");
        assert!(clean_url("").is_err());
        assert!(clean_url("smb://").is_err());
        assert!(clean_url("https://example.org/").is_err());
        assert!(clean_url("file:///etc").is_err());
    }

    #[test]
    fn passwords_go_in_and_come_out() {
        let url = with_credentials("ftp://nas/films", "me", "p@ss/w:rd");
        assert_eq!(url, "ftp://me:p%40ss%2Fw%3Ard@nas/films");
        assert_eq!(strip_credentials(&url), "ftp://nas/films");
        // A new sign-in replaces the old one.
        assert_eq!(with_credentials(&url, "you", ""), "ftp://you@nas/films");
        assert_eq!(with_credentials("ftp://nas/films", " ", "x"), "ftp://nas/films");
        // An @ in a path isn't a user name.
        assert_eq!(strip_credentials("ftp://nas/a@b/c"), "ftp://nas/a@b/c");
        assert_eq!(strip_credentials("upnp://http://192.168.1.2:8200/desc.xml?ObjectID=0"), "upnp://http://192.168.1.2:8200/desc.xml?ObjectID=0");
    }

    #[test]
    fn names_and_windows_paths() {
        assert_eq!(display_name("smb://me:pw@nas/films/New%20ones/"), "New ones");
        assert_eq!(display_name("ftp://nas.local"), "nas.local");
        assert_eq!(display_name("/Volumes/films"), "films");
        assert_eq!(unc_path("smb://me@nas:445/films/New%20ones/").as_deref(), Some("\\\\nas\\films\\New ones"));
        assert_eq!(unc_path("smb://[2001:db8::1]:445/films").as_deref(), Some("\\\\2001-db8--1.ipv6-literal.net\\films"));
        assert_eq!(unc_path("smb://[fe80::1%25eth0]/films").as_deref(), Some("\\\\fe80--1seth0.ipv6-literal.net\\films"));
        assert_eq!(unc_path("smb://nas"), None);
        assert_eq!(unc_path("smb://nas/"), None);
    }

    #[test]
    fn folders_list_their_folders_and_media() {
        let dir = std::env::temp_dir().join(format!("revlc-network-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Season 2")).unwrap();
        std::fs::create_dir_all(dir.join(".hidden")).unwrap();
        for name in ["Episode 10.mkv", "Episode 2.mkv", "notes.txt", "Theme.mp3", ".DS_Store"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let listed = list_dir(&dir).unwrap();
        let names: Vec<_> = listed.iter().map(|e| (e.name.as_str(), e.dir)).collect();
        assert_eq!(names, [("Season 2", true), ("Episode 2.mkv", false), ("Episode 10.mkv", false), ("Theme.mp3", false)]);
        assert!(list_dir(&dir.join("missing")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn saved_servers_are_kept_without_passwords() {
        let dir = std::env::temp_dir().join(format!("revlc-servers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut servers = Servers::load(&dir);
        servers.save("ftp://me:secret@nas/films", "", Some(" me ".into())).unwrap();
        servers.save("smb://attic", "Attic", None).unwrap();
        // Saving the same address again replaces it.
        servers.save("ftp://nas/films", "Films", Some("me".into())).unwrap();
        let back = Servers::load(&dir);
        assert_eq!(back.list, [
            Server { url: "smb://attic".into(), name: "Attic".into(), user: None },
            Server { url: "ftp://nas/films".into(), name: "Films".into(), user: Some("me".into()) },
        ]);
        assert!(!std::fs::read_to_string(dir.join("servers.json")).unwrap().contains("secret"));
        servers.remove("smb://attic").unwrap();
        assert_eq!(Servers::load(&dir).list.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
