//! VLC-style Lua extensions: scripts with `descriptor()`, `activate()` and
//! the other hooks VLC calls, written against VLC's `vlc.*` API. Each runs
//! in its own Lua 5.1 interpreter (the Lua that VLC 3 ships) on its own
//! thread, and reaches the player only through [`Host`].

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use mlua::chunk::ChunkMode;
use mlua::{HookTriggers, Lua, LuaOptions, StdLib, Table, Value, VmState};

/// How often a running script is checked against its limits, in Lua instructions.
const CHECK_EVERY: u32 = 10_000;

/// What a script may use before it is stopped.
#[derive(Clone, Copy)]
struct Limits {
    /// For `descriptor()`, which runs for every file on each scan.
    scan: Duration,
    /// For any other hook. VLC asks the user after 10 seconds.
    call: Duration,
    /// Bytes of Lua memory.
    memory: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { scan: Duration::from_secs(2), call: Duration::from_secs(10), memory: 64 << 20 }
    }
}

/// The hooks an extension says it has, from `capabilities` in its descriptor.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Capabilities {
    pub menu: bool,
    pub trigger: bool,
    pub input_listener: bool,
    pub meta_listener: bool,
    pub playing_listener: bool,
}

/// What an extension's `descriptor()` returned.
#[derive(Clone, Debug, PartialEq)]
pub struct Descriptor {
    pub path: PathBuf,
    /// The file name, which identifies the extension.
    pub name: String,
    pub title: String,
    pub version: Option<String>,
    pub author: Option<String>,
    pub description: Option<String>,
    pub shortdesc: Option<String>,
    pub url: Option<String>,
    pub capabilities: Capabilities,
}

/// The extensions in a folder, and the files that could not be read as one.
#[derive(Debug, Default)]
pub struct Scan {
    pub found: Vec<Descriptor>,
    /// File name and what was wrong with it.
    pub failed: Vec<(String, String)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Level {
    Debug,
    Info,
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Status {
    Stopped,
    Playing,
    Paused,
}

/// What is playing, as `vlc.input.item()` shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Playing {
    pub uri: String,
    pub name: String,
    /// In seconds; negative when unknown.
    pub duration: f64,
    /// VLC's meta names (`title`, `artist`, `album`, ...) and their values.
    pub metas: Vec<(String, String)>,
}

/// What extensions can ask of the player. The app implements it; tests use
/// a recording stand-in.
pub trait Host: Send + Sync + 'static {
    fn log(&self, extension: &str, level: Level, text: &str);
    /// The extension is no longer running, whatever ended it.
    fn stopped(&self, extension: &str);
    fn playing(&self) -> Option<Playing>;
    fn status(&self) -> Status;
    fn play(&self);
    /// Pauses or resumes, as VLC's `playlist.pause()` does.
    fn toggle_pause(&self);
    fn stop(&self);
    fn next(&self);
    fn previous(&self);
    /// In VLC's units: 256 is 100%.
    fn volume(&self) -> i32;
    fn set_volume(&self, volume: i32);
    /// A short message over the video.
    fn message(&self, text: &str);
}

/// Ends a script that runs too long or is no longer wanted.
struct Watch {
    deadline: Mutex<Instant>,
    cancelled: AtomicBool,
}

impl Watch {
    fn new(allowed: Duration) -> Arc<Self> {
        Arc::new(Self { deadline: Mutex::new(Instant::now() + allowed), cancelled: AtomicBool::new(false) })
    }

    fn allow(&self, allowed: Duration) {
        *self.deadline.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now() + allowed;
    }

    fn over(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed) || Instant::now() > *self.deadline.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Runs before every script: no bytecode (Lua 5.1 does not check it, so a
/// crafted chunk can corrupt memory), no way to end the whole app, and no
/// modules yet.
const PRELUDE: &str = r#"
require = function(name)
  error("module '" .. tostring(name) .. "' is not available: ReVLC extensions cannot load modules yet", 2)
end
if os then os.exit = nil end
if not string then
  -- Reading a descriptor: no libraries, so nothing to load code with.
  loadstring, load, loadfile, dofile = nil, nil, nil, nil
  return
end
local loadstring_, io_open = loadstring, io and io.open
string.dump = nil
loadstring = function(chunk, name)
  if type(chunk) ~= "string" then return nil, "a string is expected" end
  if chunk:sub(1, 1) == "\27" then return nil, "binary chunks are not allowed" end
  return loadstring_(chunk, name)
end
load = function(pieces, name)
  local parts = {}
  while true do
    local piece = pieces()
    if piece == nil or piece == "" then break end
    parts[#parts + 1] = piece
  end
  return loadstring(table.concat(parts), name)
end
loadfile = function(path)
  if not io_open then return nil, "files are not available here" end
  local file, err = io_open(path, "rb")
  if not file then return nil, err end
  local chunk = file:read("*a")
  file:close()
  return loadstring(chunk, "@" .. path)
end
dofile = function(path)
  local chunk, err = loadfile(path)
  if not chunk then error(err, 2) end
  return chunk()
end
"#;

/// The script's text. Compiled Lua is refused: see [`PRELUDE`].
fn source(path: &Path) -> Result<Vec<u8>, String> {
    let source = std::fs::read(path).map_err(|e| e.to_string())?;
    if source.first() == Some(&0x1b) {
        return Err("compiled Lua is not supported; the extension must be a .lua source file".into());
    }
    Ok(source)
}

/// A Lua state with the given libraries, whose scripts are stopped at
/// `watch`'s limits.
fn state(libs: StdLib, memory: usize, watch: &Arc<Watch>) -> Result<Lua, String> {
    let lua = Lua::new_with(libs, LuaOptions::default()).map_err(|e| e.to_string())?;
    lua.set_memory_limit(memory).map_err(|e| e.to_string())?;
    let watching = watch.clone();
    lua.set_hook(HookTriggers::new().every_nth_instruction(CHECK_EVERY), move |_, _| {
        if watching.over() { Err(mlua::Error::runtime("the script took too long and was stopped")) } else { Ok(VmState::Continue) }
    })
    .map_err(|e| e.to_string())?;
    lua.load(PRELUDE).set_name("=prelude").exec().map_err(|e| e.to_string())?;
    Ok(lua)
}

/// Runs the script's top level, which defines its hooks.
fn load(lua: &Lua, path: &Path, source: &[u8]) -> Result<(), String> {
    lua.load(source).set_name(format!("@{}", file_name(path))).set_mode(ChunkMode::Text).exec().map_err(|e| e.to_string())
}

/// Calls the script's global function `name`.
fn call<R: mlua::FromLuaMulti>(lua: &Lua, name: &str, args: impl mlua::IntoLuaMulti) -> Result<R, String> {
    let hook: Value = lua.globals().get(name).map_err(|e| e.to_string())?;
    let Value::Function(hook) = hook else {
        return Err(format!("function {name}() not found"));
    };
    hook.call::<R>(args).map_err(|e| format!("function {name}(): {e}"))
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Reads one extension's descriptor, running the script with no libraries
/// beyond Lua's basic functions, as VLC does.
pub fn describe(path: &Path) -> Result<Descriptor, String> {
    describe_within(path, Limits::default())
}

fn describe_within(path: &Path, limits: Limits) -> Result<Descriptor, String> {
    let watch = Watch::new(limits.scan);
    let source = source(path)?;
    let lua = state(StdLib::NONE, limits.memory, &watch)?;
    load(&lua, path, &source)?;
    let table: Value = call(&lua, "descriptor", ())?;
    let Value::Table(table) = table else {
        return Err("function descriptor() did not return a table".into());
    };
    let text = |key: &str| -> Option<String> { table.get::<Option<String>>(key).ok().flatten() };
    let name = file_name(path);
    let mut capabilities = Capabilities::default();
    if let Ok(Some(list)) = table.get::<Option<Table>>("capabilities") {
        for capability in list.sequence_values::<String>().flatten() {
            match capability.as_str() {
                "menu" => capabilities.menu = true,
                "trigger" => capabilities.trigger = true,
                "input-listener" => capabilities.input_listener = true,
                "meta-listener" => capabilities.meta_listener = true,
                "playing-listener" => capabilities.playing_listener = true,
                _ => {}
            }
        }
    }
    Ok(Descriptor {
        path: path.to_path_buf(),
        // VLC falls back to the script's name when there is no title.
        title: text("title").unwrap_or_else(|| name.clone()),
        name,
        version: text("version"),
        author: text("author"),
        description: text("description"),
        shortdesc: text("shortdesc"),
        url: text("url"),
        capabilities,
    })
}

/// Every `.lua` file directly in `dir`, in name order. A missing folder is
/// an empty one.
pub fn scan(dir: &Path) -> Scan {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| entries.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    paths.retain(|p| p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("lua")));
    paths.sort();
    let mut scan = Scan::default();
    for path in paths {
        match describe(&path) {
            Ok(descriptor) => scan.found.push(descriptor),
            Err(e) => scan.failed.push((file_name(&path), e)),
        }
    }
    scan
}

/// The `vlc` table: the part of VLC's Lua API that needs no window, files
/// or network.
fn api(lua: &Lua, name: &str, host: &Arc<dyn Host>, watch: &Arc<Watch>, limits: Limits, leaving: &Arc<AtomicBool>) -> mlua::Result<()> {
    let vlc = lua.create_table()?;

    let msg = lua.create_table()?;
    for (function, level) in [("dbg", Level::Debug), ("info", Level::Info), ("warn", Level::Warning), ("err", Level::Error)] {
        let (host, name) = (host.clone(), name.to_string());
        // One line per argument, as VLC logs them.
        msg.set(function, lua.create_function(move |lua, args: mlua::Variadic<Value>| {
            for arg in args {
                let text = lua.coerce_string(arg)?.map(|s| s.to_string_lossy()).unwrap_or_default();
                host.log(&name, level, &text);
            }
            Ok(())
        })?)?;
    }
    vlc.set("msg", msg)?;

    // Takes effect once the running hook returns, as in VLC.
    let flag = leaving.clone();
    vlc.set("deactivate", lua.create_function(move |_, ()| {
        flag.store(true, Ordering::Relaxed);
        Ok(true)
    })?)?;
    let watching = watch.clone();
    vlc.set("keep_alive", lua.create_function(move |_, ()| {
        watching.allow(limits.call);
        Ok(true)
    })?)?;

    let input = lua.create_table()?;
    let h = host.clone();
    input.set("is_playing", lua.create_function(move |_, ()| Ok(h.playing().is_some()))?)?;
    let h = host.clone();
    input.set("item", lua.create_function(move |lua, ()| {
        let Some(playing) = h.playing() else { return Ok(Value::Nil) };
        // A copy of the item as it is now, with VLC's methods.
        let item = lua.create_table()?;
        let uri = playing.uri.clone();
        item.set("uri", lua.create_function(move |_, _: Value| Ok(uri.clone()))?)?;
        let name = playing.name.clone();
        item.set("name", lua.create_function(move |_, _: Value| Ok(name.clone()))?)?;
        let duration = playing.duration;
        item.set("duration", lua.create_function(move |_, _: Value| Ok(duration))?)?;
        item.set("is_preparsed", lua.create_function(|_, _: Value| Ok(true))?)?;
        let metas = playing.metas.clone();
        item.set("metas", lua.create_function(move |lua, _: Value| {
            let table = lua.create_table()?;
            for (key, value) in &metas {
                table.set(key.as_str(), value.as_str())?;
            }
            Ok(table)
        })?)?;
        Ok(Value::Table(item))
    })?)?;
    vlc.set("input", input)?;

    let playlist = lua.create_table()?;
    type Verb = fn(&dyn Host);
    let verbs: [(&str, Verb); 5] = [
        ("play", |h| h.play()),
        ("pause", |h| h.toggle_pause()),
        ("stop", |h| h.stop()),
        ("next", |h| h.next()),
        ("prev", |h| h.previous()),
    ];
    for (function, verb) in verbs {
        let h = host.clone();
        playlist.set(function, lua.create_function(move |_, ()| {
            verb(h.as_ref());
            Ok(())
        })?)?;
    }
    let h = host.clone();
    playlist.set("status", lua.create_function(move |_, ()| {
        Ok(match h.status() {
            Status::Stopped => "stopped",
            Status::Playing => "playing",
            Status::Paused => "paused",
        })
    })?)?;
    vlc.set("playlist", playlist)?;

    let volume = lua.create_table()?;
    let h = host.clone();
    volume.set("get", lua.create_function(move |_, ()| Ok(h.volume()))?)?;
    let h = host.clone();
    volume.set("set", lua.create_function(move |_, level: f64| {
        h.set_volume(level as i32);
        Ok(())
    })?)?;
    vlc.set("volume", volume)?;

    let osd = lua.create_table()?;
    let h = host.clone();
    // VLC's channel, position and duration arguments are accepted and ignored.
    osd.set("message", lua.create_function(move |_, (text, _rest): (String, mlua::Variadic<Value>)| {
        h.message(&text);
        Ok(())
    })?)?;
    vlc.set("osd", osd)?;

    lua.globals().set("vlc", vlc)
}

enum Command {
    Menu(Sender<Result<Vec<(i64, String)>, String>>),
    TriggerMenu(i64),
    InputChanged,
    PlayingChanged(i32),
    MetaChanged,
    Deactivate,
}

/// An activated extension. Dropping it ends the script without calling its
/// `deactivate()`.
pub struct Extension {
    commands: Sender<Command>,
    watch: Arc<Watch>,
    alive: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Extension {
    /// Starts the extension on its own thread and waits for its
    /// `activate()` to return.
    pub fn activate(descriptor: &Descriptor, host: Arc<dyn Host>) -> Result<Self, String> {
        Self::activate_within(descriptor, host, Limits::default())
    }

    fn activate_within(descriptor: &Descriptor, host: Arc<dyn Host>, limits: Limits) -> Result<Self, String> {
        let (commands, inbox) = mpsc::channel();
        let (started, start) = mpsc::channel();
        let watch = Watch::new(limits.call);
        let alive = Arc::new(AtomicBool::new(true));
        let (descriptor, watching, living) = (descriptor.clone(), watch.clone(), alive.clone());
        let thread = thread::Builder::new()
            .name(format!("extension {}", descriptor.name))
            .spawn(move || run(&descriptor, host, &watching, limits, &inbox, &started, &living))
            .map_err(|e| e.to_string())?;
        match start.recv() {
            Ok(Ok(())) => Ok(Self { commands, watch, alive, thread: Some(thread) }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("the extension stopped while starting".into()),
        }
    }

    /// False once the script has ended, by itself or because it was stopped.
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    /// The entries of the extension's menu, each with the id to pass to
    /// [`Extension::trigger_menu`].
    pub fn menu(&self) -> Result<Vec<(i64, String)>, String> {
        let (reply, answer) = mpsc::channel();
        self.commands.send(Command::Menu(reply)).map_err(|_| "the extension is not running".to_string())?;
        answer.recv().map_err(|_| "the extension is not running".to_string())?
    }

    pub fn trigger_menu(&self, id: i64) {
        let _ = self.commands.send(Command::TriggerMenu(id));
    }

    /// Something else is playing now.
    pub fn input_changed(&self) {
        let _ = self.commands.send(Command::InputChanged);
    }

    /// `status` is VLC's input state: 1 opening, 2 playing, 3 paused, 4 stopped or ended, 5 error.
    pub fn playing_changed(&self, status: i32) {
        let _ = self.commands.send(Command::PlayingChanged(status));
    }

    pub fn meta_changed(&self) {
        let _ = self.commands.send(Command::MetaChanged);
    }

    /// Runs the script's `deactivate()` after what is already queued, and
    /// waits for it to end.
    pub fn deactivate(mut self) {
        let _ = self.commands.send(Command::Deactivate);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Extension {
    fn drop(&mut self) {
        self.watch.cancelled.store(true, Ordering::Relaxed);
    }
}

/// Runs a trigger-only extension's `trigger()` once, on this thread.
pub fn trigger(descriptor: &Descriptor, host: Arc<dyn Host>) -> Result<(), String> {
    trigger_within(descriptor, host, Limits::default())
}

fn trigger_within(descriptor: &Descriptor, host: Arc<dyn Host>, limits: Limits) -> Result<(), String> {
    let watch = Watch::new(limits.call);
    let lua = prepared(descriptor, &host, &watch, limits, &Arc::new(AtomicBool::new(false)))?;
    let result = call::<()>(&lua, "trigger", ());
    drop(lua);
    host.stopped(&descriptor.name);
    result
}

/// A state with the standard libraries VLC gives extensions (all but
/// `debug` and `package`), the `vlc` table, and the script loaded.
fn prepared(descriptor: &Descriptor, host: &Arc<dyn Host>, watch: &Arc<Watch>, limits: Limits, leaving: &Arc<AtomicBool>) -> Result<Lua, String> {
    let source = source(&descriptor.path)?;
    let libs = StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::OS | StdLib::IO;
    let lua = state(libs, limits.memory, watch)?;
    api(&lua, &descriptor.name, host, watch, limits, leaving).map_err(|e| e.to_string())?;
    load(&lua, &descriptor.path, &source)?;
    Ok(lua)
}

/// The extension's thread: `activate()`, then one hook per command until it
/// is told to stop, stops itself, or runs past its limits.
fn run(descriptor: &Descriptor, host: Arc<dyn Host>, watch: &Arc<Watch>, limits: Limits, inbox: &Receiver<Command>, started: &Sender<Result<(), String>>, alive: &AtomicBool) {
    let name = descriptor.name.as_str();
    let leaving = Arc::new(AtomicBool::new(false));
    let lua = prepared(descriptor, &host, watch, limits, &leaving).and_then(|lua| {
        watch.allow(limits.call);
        call::<()>(&lua, "activate", ()).map(|()| lua)
    });
    let lua = match lua {
        Ok(lua) => lua,
        Err(e) => {
            alive.store(false, Ordering::Relaxed);
            host.stopped(name);
            let _ = started.send(Err(e));
            return;
        }
    };
    let _ = started.send(Ok(()));

    let capabilities = &descriptor.capabilities;
    // Whether the script should get its `deactivate()` on the way out.
    let mut orderly = false;
    loop {
        if leaving.load(Ordering::Relaxed) {
            orderly = true;
            break;
        }
        let Ok(command) = inbox.recv() else { break };
        watch.allow(limits.call);
        let result = match command {
            Command::Menu(reply) => {
                let entries = menu(&lua);
                let _ = reply.send(entries);
                Ok(())
            }
            Command::TriggerMenu(id) => call::<()>(&lua, "trigger_menu", id),
            Command::InputChanged if capabilities.input_listener => call::<()>(&lua, "input_changed", ()),
            Command::PlayingChanged(status) if capabilities.playing_listener => call::<()>(&lua, "playing_changed", status),
            Command::MetaChanged if capabilities.meta_listener => call::<()>(&lua, "meta_changed", ()),
            Command::InputChanged | Command::PlayingChanged(_) | Command::MetaChanged => Ok(()),
            Command::Deactivate => {
                orderly = true;
                break;
            }
        };
        if watch.cancelled.load(Ordering::Relaxed) {
            break;
        }
        if watch.over() {
            host.log(name, Level::Error, "The extension took too long and was stopped.");
            break;
        }
        // A hook that fails is reported; the extension stays active, as in VLC.
        if let Err(e) = result {
            host.log(name, Level::Warning, &e);
        }
    }
    if orderly {
        watch.allow(limits.call);
        if let Err(e) = call::<()>(&lua, "deactivate", ()) {
            host.log(name, Level::Warning, &e);
        }
    }
    drop(lua);
    alive.store(false, Ordering::Relaxed);
    host.stopped(name);
}

/// The script's `menu()`: a table of entries, whose keys are the ids.
fn menu(lua: &Lua) -> Result<Vec<(i64, String)>, String> {
    let table: Value = call(lua, "menu", ())?;
    let Value::Table(table) = table else {
        return Err("function menu() did not return a table".into());
    };
    let mut entries = Vec::new();
    for pair in table.pairs::<i64, String>() {
        entries.push(pair.map_err(|_| "an entry in the menu table is invalid".to_string())?);
    }
    entries.sort();
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records what scripts ask for.
    #[derive(Default)]
    struct Recorder {
        calls: Mutex<Vec<String>>,
        playing: Mutex<Option<Playing>>,
    }

    impl Recorder {
        fn note(&self, call: impl Into<String>) {
            self.calls.lock().unwrap().push(call.into());
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Host for Recorder {
        fn log(&self, _: &str, level: Level, text: &str) {
            self.note(format!("log {level:?} {text}"));
        }
        fn stopped(&self, extension: &str) {
            self.note(format!("stopped {extension}"));
        }
        fn playing(&self) -> Option<Playing> {
            self.playing.lock().unwrap().clone()
        }
        fn status(&self) -> Status {
            Status::Paused
        }
        fn play(&self) {
            self.note("play");
        }
        fn toggle_pause(&self) {
            self.note("toggle_pause");
        }
        fn stop(&self) {
            self.note("stop");
        }
        fn next(&self) {
            self.note("next");
        }
        fn previous(&self) {
            self.note("previous");
        }
        fn volume(&self) -> i32 {
            128
        }
        fn set_volume(&self, volume: i32) {
            self.note(format!("set_volume {volume}"));
        }
        fn message(&self, text: &str) {
            self.note(format!("message {text}"));
        }
    }

    /// A fresh folder for one test.
    fn folder(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("revlc-extensions-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn script(dir: &Path, name: &str, source: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, source).unwrap();
        path
    }

    fn quick() -> Limits {
        Limits { scan: Duration::from_millis(200), call: Duration::from_millis(200), memory: 8 << 20 }
    }

    const HELLO: &str = r#"
function descriptor()
  return { title = "Hello", version = "1.2", author = "A", description = "Says hello", shortdesc = "Hi",
           url = "https://example.org", capabilities = { "menu", "input-listener", "something-new" } }
end
function activate() vlc.msg.info("activated") end
function deactivate() vlc.msg.info("deactivated") end
function menu() return { "First", "Second" } end
function trigger_menu(id) vlc.msg.info("menu " .. id) end
function input_changed() vlc.msg.info("input changed") end
function playing_changed(status) vlc.msg.info("playing " .. status) end
"#;

    #[test]
    fn a_descriptor_is_read() {
        let dir = folder("descriptor");
        let d = describe(&script(&dir, "hello.lua", HELLO)).unwrap();
        assert_eq!(d.name, "hello.lua");
        assert_eq!(d.title, "Hello");
        assert_eq!(d.version.as_deref(), Some("1.2"));
        assert_eq!(d.author.as_deref(), Some("A"));
        assert_eq!(d.description.as_deref(), Some("Says hello"));
        assert_eq!(d.shortdesc.as_deref(), Some("Hi"));
        assert_eq!(d.url.as_deref(), Some("https://example.org"));
        assert_eq!(d.capabilities, Capabilities { menu: true, input_listener: true, ..Default::default() });
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_missing_title_is_the_file_name() {
        let dir = folder("untitled");
        let d = describe(&script(&dir, "plain.lua", "function descriptor() return {} end")).unwrap();
        assert_eq!(d.title, "plain.lua");
        assert_eq!(d.capabilities, Capabilities::default());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_scan_lists_lua_files_in_name_order_and_says_which_failed() {
        let dir = folder("scan");
        script(&dir, "b.lua", HELLO);
        script(&dir, "a.lua", "function descriptor() return { title = 'A' } end");
        script(&dir, "broken.lua", "function descriptor( return end");
        script(&dir, "none.lua", "x = 1");
        script(&dir, "wrong.lua", "function descriptor() return 3 end");
        std::fs::write(dir.join("compiled.lua"), b"\x1bLuaQ\0\x01\x04").unwrap();
        script(&dir, "notes.txt", "not an extension");
        script(&dir, "old.luac", "function descriptor() return { title = 'no' } end");
        std::fs::create_dir(dir.join("folder.lua")).unwrap();

        let scan = scan(&dir);
        assert_eq!(scan.found.iter().map(|d| d.title.as_str()).collect::<Vec<_>>(), ["A", "Hello"]);
        let failed: Vec<&str> = scan.failed.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(failed, ["broken.lua", "compiled.lua", "none.lua", "wrong.lua"]);
        assert!(scan.failed[1].1.contains("compiled Lua"), "{}", scan.failed[1].1);
        assert!(scan.failed[2].1.contains("descriptor() not found"), "{}", scan.failed[2].1);
        assert!(scan.failed[3].1.contains("did not return a table"), "{}", scan.failed[3].1);

        assert!(super::scan(&dir.join("missing")).found.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_descriptor_that_never_returns_is_stopped() {
        let dir = folder("endless-descriptor");
        let path = script(&dir, "loop.lua", "function descriptor() while true do end end");
        let e = describe_within(&path, quick()).unwrap_err();
        assert!(e.contains("took too long"), "{e}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_hooks_run_in_order_and_only_the_declared_listeners() {
        let dir = folder("lifecycle");
        let d = describe(&script(&dir, "hello.lua", HELLO)).unwrap();
        let host = Arc::new(Recorder::default());
        let extension = Extension::activate(&d, host.clone()).unwrap();
        assert!(extension.is_alive());
        assert_eq!(extension.menu().unwrap(), [(1, "First".to_string()), (2, "Second".to_string())]);
        extension.trigger_menu(2);
        extension.input_changed();
        // Not declared in capabilities, so never called.
        extension.playing_changed(2);
        extension.meta_changed();
        extension.deactivate();
        assert_eq!(
            host.calls(),
            ["log Info activated", "log Info menu 2", "log Info input changed", "log Info deactivated", "stopped hello.lua"]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn scripts_reach_the_player_through_the_host() {
        let dir = folder("host");
        let source = r#"
function descriptor() return { title = "Remote" } end
function activate()
  assert(vlc.input.is_playing())
  local item = vlc.input.item()
  vlc.msg.dbg(item:uri(), item:name(), item:duration(), item:metas().artist, tostring(item:is_preparsed()))
  vlc.msg.warn(vlc.playlist.status(), vlc.volume.get())
  vlc.playlist.play() vlc.playlist.pause() vlc.playlist.stop() vlc.playlist.next() vlc.playlist.prev()
  vlc.volume.set(256)
  vlc.osd.message("Hi", 1, "top", 1000000)
  vlc.msg.err("done")
end
function deactivate() end
"#;
        let d = describe(&script(&dir, "remote.lua", source)).unwrap();
        let host = Arc::new(Recorder::default());
        *host.playing.lock().unwrap() = Some(Playing {
            uri: "file:///a/b.mkv".into(),
            name: "b.mkv".into(),
            duration: 12.5,
            metas: vec![("artist".into(), "Someone".into())],
        });
        Extension::activate(&d, host.clone()).unwrap().deactivate();
        assert_eq!(
            host.calls(),
            [
                "log Debug file:///a/b.mkv", "log Debug b.mkv", "log Debug 12.5", "log Debug Someone", "log Debug true",
                "log Warning paused", "log Warning 128",
                "play", "toggle_pause", "stop", "next", "previous",
                "set_volume 256", "message Hi", "log Error done", "stopped remote.lua",
            ]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn nothing_playing_is_nil() {
        let dir = folder("idle");
        let source = "function descriptor() return { title = 'Idle' } end\n\
            function activate() vlc.msg.info(tostring(vlc.input.is_playing()), tostring(vlc.input.item())) end\n\
            function deactivate() end";
        let d = describe(&script(&dir, "idle.lua", source)).unwrap();
        let host = Arc::new(Recorder::default());
        Extension::activate(&d, host.clone()).unwrap().deactivate();
        assert_eq!(host.calls(), ["log Info false", "log Info nil", "stopped idle.lua"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_extension_that_cannot_activate_is_not_started() {
        let dir = folder("no-activate");
        let host = Arc::new(Recorder::default());
        let d = describe(&script(&dir, "none.lua", "function descriptor() return { title = 'N' } end")).unwrap();
        let e = Extension::activate(&d, host.clone()).err().unwrap();
        assert!(e.contains("activate() not found"), "{e}");
        let d = describe(&script(&dir, "fails.lua", "function descriptor() return { title = 'F' } end\nfunction activate() error('no luck') end")).unwrap();
        let e = Extension::activate(&d, host.clone()).err().unwrap();
        assert!(e.contains("no luck"), "{e}");
        assert_eq!(host.calls(), ["stopped none.lua", "stopped fails.lua"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failing_hook_is_reported_and_the_extension_stays() {
        let dir = folder("failing-hook");
        let source = "function descriptor() return { title = 'F', capabilities = { 'menu' } } end\n\
            function activate() end\nfunction deactivate() end\n\
            function menu() return { 'One' } end\n\
            function trigger_menu(id) error('bad click') end";
        let d = describe(&script(&dir, "f.lua", source)).unwrap();
        let host = Arc::new(Recorder::default());
        let extension = Extension::activate(&d, host.clone()).unwrap();
        extension.trigger_menu(1);
        assert_eq!(extension.menu().unwrap(), [(1, "One".to_string())]);
        assert!(extension.is_alive());
        extension.deactivate();
        let calls = host.calls();
        assert_eq!(calls.len(), 2, "{calls:?}");
        assert!(calls[0].starts_with("log Warning function trigger_menu(): ") && calls[0].contains("bad click"), "{calls:?}");
        assert_eq!(calls[1], "stopped f.lua");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_stuck_script_is_stopped_without_its_deactivate() {
        let dir = folder("stuck");
        let source = "function descriptor() return { title = 'S' } end\n\
            function activate() end\n\
            function deactivate() vlc.msg.info('deactivated') end\n\
            function trigger_menu(id) while true do end end";
        let d = describe(&script(&dir, "s.lua", source)).unwrap();
        let host = Arc::new(Recorder::default());
        let extension = Extension::activate_within(&d, host.clone(), quick()).unwrap();
        extension.trigger_menu(1);
        let waited = Instant::now();
        while extension.is_alive() && waited.elapsed() < Duration::from_secs(10) {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!extension.is_alive());
        assert!(extension.menu().is_err());
        assert_eq!(host.calls(), ["log Error The extension took too long and was stopped.", "stopped s.lua"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn keep_alive_gives_a_long_hook_more_time() {
        let dir = folder("keep-alive");
        let source = "function descriptor() return { title = 'K' } end\n\
            function activate()\n\
              local started = os.clock()\n\
              while os.clock() - started < 0.5 do vlc.keep_alive() end\n\
            end\n\
            function deactivate() end";
        let d = describe(&script(&dir, "k.lua", source)).unwrap();
        let host = Arc::new(Recorder::default());
        Extension::activate_within(&d, host.clone(), quick()).unwrap().deactivate();
        assert_eq!(host.calls(), ["stopped k.lua"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn vlc_deactivate_ends_the_extension_after_the_running_hook() {
        let dir = folder("self-deactivate");
        let source = "function descriptor() return { title = 'D' } end\n\
            function activate() end\n\
            function deactivate() vlc.msg.info('deactivated') end\n\
            function trigger_menu(id) vlc.deactivate() vlc.msg.info('still here') end";
        let d = describe(&script(&dir, "d.lua", source)).unwrap();
        let host = Arc::new(Recorder::default());
        let extension = Extension::activate(&d, host.clone()).unwrap();
        extension.trigger_menu(1);
        let waited = Instant::now();
        while extension.is_alive() && waited.elapsed() < Duration::from_secs(10) {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(host.calls(), ["log Info still here", "log Info deactivated", "stopped d.lua"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn dropping_an_extension_ends_its_thread() {
        let dir = folder("dropped");
        let source = "function descriptor() return { title = 'X' } end\nfunction activate() end\nfunction deactivate() vlc.msg.info('deactivated') end";
        let d = describe(&script(&dir, "x.lua", source)).unwrap();
        let host = Arc::new(Recorder::default());
        drop(Extension::activate(&d, host.clone()).unwrap());
        let waited = Instant::now();
        while host.calls().is_empty() && waited.elapsed() < Duration::from_secs(10) {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(host.calls(), ["stopped x.lua"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_trigger_only_extension_runs_once() {
        let dir = folder("trigger");
        let source = "function descriptor() return { title = 'T', capabilities = { 'trigger' } } end\nfunction trigger() vlc.playlist.next() end";
        let d = describe(&script(&dir, "t.lua", source)).unwrap();
        assert!(d.capabilities.trigger);
        let host = Arc::new(Recorder::default());
        trigger(&d, host.clone()).unwrap();
        assert_eq!(host.calls(), ["next", "stopped t.lua"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn scripts_cannot_load_bytecode_modules_or_end_the_app() {
        let dir = folder("limits");
        std::fs::write(dir.join("chunk.bin"), b"\x1bLuaQ").unwrap();
        // Forward slashes, so a Windows path needs no escaping in Lua.
        let chunk = dir.join("chunk.bin").to_string_lossy().replace('\\', "/");
        let source = r#"
function descriptor() return { title = "L" } end
function activate()
  local f, e = loadstring("\27LuaQ")
  vlc.msg.info(tostring(f), e)
  vlc.msg.info(tostring(loadstring("return 1 + 1")()))
  vlc.msg.info(type(load(function() return nil end)))
  vlc.msg.info(tostring(string.dump), tostring(os.exit), tostring(debug), tostring(package), type(io))
  vlc.msg.info(tostring(pcall(require, "socket")))
  local f, e = loadfile("CHUNK")
  vlc.msg.info(tostring(f), e, tostring(pcall(dofile, "CHUNK")))
end
function deactivate() end
"#
        .replace("CHUNK", &chunk);
        let d = describe(&script(&dir, "l.lua", &source)).unwrap();
        let host = Arc::new(Recorder::default());
        Extension::activate(&d, host.clone()).unwrap().deactivate();
        assert_eq!(
            host.calls(),
            [
                "log Info nil", "log Info binary chunks are not allowed",
                "log Info 2",
                "log Info function",
                "log Info nil", "log Info nil", "log Info nil", "log Info nil", "log Info table",
                "log Info false",
                "log Info nil", "log Info binary chunks are not allowed", "log Info false",
                "stopped l.lua",
            ]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_script_that_uses_too_much_memory_fails_its_hook() {
        let dir = folder("memory");
        let source = "function descriptor() return { title = 'M' } end\n\
            function activate() local t = {} for i = 1, 1e9 do t[i] = string.rep('x', 1024) .. i end end";
        let d = describe(&script(&dir, "m.lua", source)).unwrap();
        let host = Arc::new(Recorder::default());
        let limits = Limits { call: Duration::from_secs(30), ..quick() };
        let e = Extension::activate_within(&d, host.clone(), limits).err().unwrap();
        assert!(e.contains("memory"), "{e}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
