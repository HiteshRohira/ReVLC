# Extensions: design and roadmap

Written 2026-11-11 against commit `c190987`. This is the reasoning behind
[plan 005](005-extensions-lua-host.md) and the outline of the phases after
it. It is for the maintainer; executors work from the numbered plans, which
repeat what they need.

## Summary

- **VLC's extensions do not work in ReVLC today**, and nothing in ReVLC tries
  to load them. VLC's Lua plugin and one compiled extension are in the bundle,
  unused.
- **VLC's own extension host cannot be reused cleanly.** It is reachable only
  through libVLC's private plugin interface, and its playback functions talk
  to a playlist that ReVLC does not use.
- **Recommendation: keep Lua, and host it in ReVLC's Rust core.** Embed Lua
  5.1 (the version VLC 3 ships) with the `mlua` crate and implement VLC's
  `vlc.*` extension API on top of ReVLC's own player. Existing VLC extensions
  then run unmodified wherever the API they use is covered.
- **Cost measured in a prototype**: about 0.35 MB added to a release binary
  (the libVLC bundle is about 90 MB). No effect on playback performance:
  extensions run on their own threads and only when a hook fires.
- Plan 005 lands the host with no user-visible change. Three more phases
  make it a feature: menu and management, dialogs, then files and network.

## What I checked

| Question | Finding | Evidence |
|---|---|---|
| Does ReVLC load extensions? | No. No code refers to them, and the parity list has it unchecked. | `docs/parity.md:107` (`- [ ] Lua extensions`); no `extension` host code in `src-tauri/src` |
| Is VLC's Lua plugin in the bundle? | Yes, with one extension, compiled. | `src-tauri/vlc/plugins/liblua_plugin.dylib`, `src-tauri/vlc/share/lua/extensions/VLSub.luac` |
| Can libVLC's public API start an extension? | No. There is no extension function in libVLC. VLC's Qt and macOS interfaces start them through `vlc_extensions.h`, a header for plugins inside VLC. | VLC 3.0 `include/vlc_extensions.h`; `libvlccore` exports `module_need` and `vlc_dialog_provider_set_ext_callback` but no extension call |
| Could ReVLC use that private interface anyway? | Partly, and badly. See below. | VLC 3.0 `modules/lua/libs/input.c:49-71`, `modules/lua/libs/playlist.c` |
| Which Lua do VLC extensions target? | Lua 5.1 on the builds ReVLC bundles; VLC's documentation says "VLC uses Lua 5.1". Linux distributions build VLC against Lua 5.2, so published extensions keep to what both accept. | version string inside `liblua_plugin.dylib` (`Lua 5.1.5`); VLC 3.0 `contrib/src/lua/rules.mak:3`; `share/lua/README.txt` |
| Can the bundled `.luac` files be loaded by another Lua? | No. They are 5.1 bytecode in a layout only VLC's patched Lua reads. Extensions must be `.lua` source. | header of `VLSub.luac` (`1b 4c 75 61 51 00 01 04 04 04 08 00`: 4-byte sizes on a 64-bit build) |

### Why not drive VLC's own host

It looks free: the plugin is already shipped. Three things rule it out.

1. **The playing media is invisible to it.** A script asks for the current
   input through `vlclua_get_input_internal`, which returns the input VLC's
   interface handed over with `extension_SetInput`, or else the core
   playlist's current input. ReVLC plays through `libvlc_media_player`, which
   never touches the core playlist. To hand over the input ReVLC would have
   to read a pointer out of `libvlc_media_player_t`, a private struct whose
   layout differs by platform and is not part of any stable interface.
2. **`vlc.playlist.*` and `vlc.volume.*` would act on a phantom.** They call
   `playlist_Play`, `playlist_Next`, `playlist_VolumeSet` and so on against
   libvlccore's playlist, which in ReVLC is empty and plays nothing. ReVLC's
   queue lives in `src-tauri/src/playlist.rs`.
3. **Dialogs need VLC's C structs mirrored in Rust** (`extension_dialog_t`,
   `extension_widget_t`, with their mutexes and condition variables), on
   three platforms, against the distribution's libVLC on Linux, and all of it
   changes in VLC 4.

So the only part of VLC's host that would work unmodified is the part that is
easy to write: loading a script and calling its hooks.

## Options

"Runs VLC extensions" means existing `.lua` extensions written for VLC,
unmodified.

| Option | Runs VLC extensions | Added size | Isolation | Main cost |
|---|---|---|---|---|
| VLC's host through libvlccore | In name only (see above) | 0 | None | Private structs on three platforms; breaks with VLC 4 |
| **Lua 5.1 in the Rust core (`mlua`)** | **Yes, as far as the `vlc.*` API is implemented** | **About 0.35 MB, measured** | Chosen libraries, time and memory limits; the interpreter itself is old and unmaintained | Re-implementing the API, in phases |
| Luau in the Rust core (`mlua`) | Most, but it has no `io`, no `os.execute`, no `require`; those would be written by hand | About 1 to 1.5 MB (estimate), needs a C++ compiler | Built for untrusted code; maintained and fuzzed | Same API work, plus the compatibility gaps |
| LuaJIT | Yes | About 0.5 MB (estimate) | None | Just-in-time code needs an extra entitlement on macOS; speed is not needed here |
| JavaScript in the web view | No | 0 | A sandboxed frame, but in the UI process, next to the app's command bridge | A new API and a new ecosystem from nothing; extensions stop when the window's page reloads |
| JavaScript in the core (QuickJS) | No | About 1 MB (estimate) | Reasonable | New ecosystem; no gain over Lua |
| WebAssembly | No | 1 to 10 MB depending on runtime (estimate) | Strongest | Authors need a compiler toolchain to write a ten-line extension |

### Recommendation: Lua 5.1 through `mlua`

- **It is the only option where existing extensions run.** "VLC-like
  extensions" in practice means `descriptor()`, `activate()`, `vlc.dialog`
  and the catalogue at addons.videolan.org. Any other language starts with
  zero extensions.
- **Exactly Lua 5.1, not a newer Lua.** From 5.3 on, `10 / 2` prints as
  `5.0`, which breaks scripts that build strings from arithmetic, and 5.1's
  `unpack`, `loadstring` and `table.getn` are gone. Matching VLC's version
  removes a class of bugs that cannot be shimmed.
- **Size and speed are non-issues.** 0.35 MB against a 90 MB bundle. Hooks run
  a few times per media change on their own thread.
- **Hosting in Rust, not the web view, fits the code.** Everything an
  extension asks for (current item, pause, volume, add to queue) already
  exists as a method on `Core` in `src-tauri/src/lib.rs`, callable from any
  thread. The host is testable with `cargo nextest` and no libVLC, the layer
  `docs/testing.md` prefers.

**Where Luau would be the better choice**: if extensions should run with
fewer rights than the user (no files, no programs), Luau's isolation is real
and Lua 5.1's is not. That goal conflicts with running existing extensions,
which use `io` and `os` freely. The `Host` trait in plan 005 keeps the
language behind one seam, so a second runtime could be added later without
touching the player.

## Architecture

```
src-tauri/src/extensions.rs
  scan(dir)            -> descriptors of every .lua file (bare Lua state, 2 s limit)
  Extension::activate  -> one OS thread and one Lua state per active extension
      commands in      <- menu(), trigger_menu(id), input_changed(),
                          playing_changed(status), meta_changed(), deactivate()
      calls out        -> trait Host (the app implements it on Core; tests use a recorder)
  trigger(descriptor)  -> runs a trigger-only extension once
```

- **One thread per active extension**, as in VLC. A slow script cannot stall
  playback or the UI. Hooks run one at a time, in the order they were asked
  for.
- **Limits.** Every hook has 10 seconds (VLC's own watchdog period;
  `vlc.keep_alive()` extends it), and each state has a 64 MiB memory limit. A
  script past its time is stopped and the extension ends. This catches
  accidental endless loops. It cannot interrupt a script that is blocked
  inside a host call, so later phases must give every blocking call (network,
  dialogs) its own timeout.
- **No Lua bytecode, ever.** Lua 5.1 does not verify bytecode, so a crafted
  chunk can corrupt memory. `.luac` files are not scanned, a `.lua` file
  holding bytecode is refused, `string.dump` is removed, and `loadstring`,
  `load`, `loadfile` and `dofile` refuse binary chunks.
- **`os.exit` is removed** (it would end ReVLC), and `debug` and `package`
  are not loaded. `require` reports that modules are not available until the
  phase that adds ReVLC's own text modules.

## Trust model (decision for the maintainer)

Proposed, and assumed by the phase outlines below:

- **An extension runs with the user's rights**, as in VLC: it can read and
  write files and start programs. This is what lets existing extensions work.
- **Unlike VLC, nothing runs unasked.** VLC loads every script in its folder
  at start. ReVLC would scan only its own folder
  (`<app data>/extensions/`), never VLC's, and run a script only when the
  user turns it on, after a confirmation that says what an extension can do.
  A script whose file has changed since it was approved asks again.
- Scanning still executes each script's top level and `descriptor()`, in a
  state with no libraries and a 2-second limit, so that the list can show
  titles. VLC does the same.

If you would rather have least-privilege extensions, decide before phase 2:
it changes the confirmation flow and argues for Luau.

## Phases

Effort is coarse. Each later phase becomes a numbered plan once the one
before it has merged, because its plan must quote code that does not exist
yet.

### Phase 1: the host, no user-visible change (plan 005, written)

Scan, lifecycle, limits, and the part of `vlc.*` that needs no window, files
or network: `vlc.msg`, `vlc.deactivate`, `vlc.keep_alive`, `vlc.input`
(`is_playing`, `item` with `uri`, `name`, `duration`, `metas`,
`is_preparsed`), `vlc.playlist` (`play`, `pause`, `stop`, `next`, `prev`,
`status`), `vlc.volume` (`get`, `set`), `vlc.osd.message`. 16 unit tests.
Effort M.

### Phase 2: wiring, menu and management (effort L)

Makes extensions usable for scripts that need no dialog.

- Implement `Host` for `Core` (`src-tauri/src/lib.rs`): `playing()` from
  `current_item()` (`lib.rs:645`) and the metadata cache; `toggle_pause`,
  `stop`, `next`, `previous` from the code behind the commands at
  `lib.rs:1588`, `1610`, `1854`, `1859`; volume through the code behind
  `set_volume` (`lib.rs:1885`), converting VLC's 256 = 100%; `message` and
  `log` through `Core::notice` (`lib.rs:818`) and the app's log.
- Send `input_changed`, `playing_changed` and `meta_changed` from
  `pump_events` (`lib.rs:1100` onward: the `PLAYING`, `PAUSED`,
  `END_REACHED`, `MEDIA_CHANGED` branches). Plan 003 edits the same function
  and should land first.
- Never call `Extension::menu` or `Extension::deactivate` while holding one
  of `Core`'s mutexes: both wait for the extension's thread, which may be
  waiting on that mutex inside a `Host` call.
- Commands `extensions_list`, `extension_set_active`, `extension_menu`,
  `extension_trigger`, an `extensions://changed` event, registered in
  `generate_handler!` (`lib.rs:2639`).
- State file `extensions.json` (approved file hashes) through `store.rs`,
  using `read_json` from plan 001.
- UI: an Extensions entry in the player menu (`ui/src/lib/SettingsMenu.svelte`,
  `menus.ts`, `actions.ts`) and in the macOS menu bar
  (`src-tauri/src/menu.rs`, which builds fixed submenus from tables today and
  would need a submenu rebuilt when the list changes); a management list in
  Settings (`SettingsApp.svelte`) with the confirmation, "Open extensions
  folder" and the scan's error list.
- Also `vlc.var.get/set` on the input for `time`, `position`, `length` and
  `rate`, `vlc.playlist.add/enqueue/clear`, `vlc.strings`, `vlc.config`
  directories: small, and most real extensions use them.
- Tests: Rust unit tests for the mapping; Playwright with the mocked backend
  (`ui/tests/mock.ts`, `tests/ui/`); one Cua scenario with a sample extension
  that pauses playback.
- Open questions: where the menu sits on Windows and Linux; whether active
  extensions are remembered across launches (VLC does not).

### Phase 3: dialogs (effort L)

Most published extensions open a dialog, so this is where the catalogue
starts working.

- `vlc.dialog` and its widgets (label, button, html, text input, password,
  check box, drop-down, list, image, spin icon), with VLC's grid arguments
  (`col`, `row`, `col_span`, `row_span`).
- The Lua side keeps the widget tree and sends a snapshot to the UI when a
  hook returns or `d:update()` is called (VLC's `lua_DialogFlush`). Clicks
  and edits come back as commands on the extension's queue; the `close()`
  hook runs when the user closes the dialog.
- UI: one Svelte component on a CSS grid, reusing `ui/src/lib/Dialog.svelte`.
  `add_html` text must be rendered through an allow-list sanitiser or as
  plain text: the UI has no `{@html}` today and extension text is untrusted.
- Open question: in-window dialogs (simple, consistent with the rest of the
  app) or separate windows (closer to VLC, lets a dialog stay beside the
  video).

### Phase 4: files, network and modules (effort L)

What subtitle finders, lyrics fetchers and scrobblers need.

- `vlc.stream` (HTTP GET, `read`, `readline`) on `reqwest`, which is already
  a dependency, with timeouts; `vlc.io` and `vlc.net.stat/opendir`;
  `vlc.input.add_subtitle` through `Core::add_subtitle` (`lib.rs:458`);
  `vlc.xml` and the `simplexml` helper; `require` for ReVLC's own text
  copies of VLC's helper modules (`common`, `simplexml`, `dkjson`), which
  are GPL-2.0-or-later or MIT and compatible with ReVLC's licence.
- Not planned: `vlc.httpd`, `vlc.vlm`, `vlc.sd`, raw sockets
  (`vlc.net.listen_tcp`, `connect_tcp`, `poll`), `vlc.win`. These serve
  interfaces and service discovery more than extensions, or duplicate what
  ReVLC does itself.
- Finish with a compatibility table in `docs/extensions.md`, tested against a
  set of real extensions, and tick `docs/parity.md:107`.

## Risks

- **Two copies of Lua in one process on Linux.** The distribution's VLC Lua
  plugin links the system's Lua 5.2; ReVLC would contain Lua 5.1. They only
  collide if ReVLC's program exports its `lua_*` symbols, which Rust
  programs do not do by default. Plan 005 has a command that checks, and the
  engine tests in CI play media through the Lua plugin's playlist scripts.
  On macOS and Windows the plugin carries its own private copy.
- **Lua 5.1 is unmaintained.** Acceptable while extensions are trusted code;
  refusing bytecode removes the known way to corrupt memory from a script.
- **Coverage is a long tail.** An extension that calls one unimplemented
  function fails at that call. The compatibility table in phase 4 is how
  users find out before installing.
- **The prototype was built and tested on macOS (Apple Silicon) only.** CI's
  three-platform matrix is the first Windows and Linux build of `mlua`'s
  bundled Lua in this project.
