import { test, expect, open, calls, isMacProject } from "./fixtures";

// The sidebar's sections beyond Home and Videos: Music, Playlists, Network
// shares and Internet radio, on the mocked backend (ui/tests/mock.ts).

const section = (page: import("@playwright/test").Page, name: string) => page.locator("aside").getByRole("button", { name, exact: false });

test("every sidebar section opens, and none is marked Soon", async ({ page }) => {
  await open(page, "home");
  await expect(page.locator("aside").getByText("Soon")).toHaveCount(0);
  for (const name of ["Music", "Playlists", "Network shares", "Internet radio", "Videos", "Home"]) {
    await section(page, name).click();
    await expect(page.getByRole("heading", { level: 1, name, exact: true })).toBeVisible();
    await expect(section(page, name)).toHaveAttribute("aria-current", "page");
  }
});

test("Music sorts songs into albums and plays an album from the song picked", async ({ page }) => {
  await open(page, "home");
  await section(page, "Music").click();
  // Tagged songs make an album; untagged ones an album of their folder.
  await page.getByRole("button", { name: "Mezzanine by Massive Attack" }).click();
  const songs = page.getByRole("list", { name: "Songs on Mezzanine" }).getByRole("button");
  await expect(songs).toHaveText([/1\s*Angel\s*6:19/, /2\s*Risingson\s*4:58/]);
  await songs.nth(1).click();
  expect(await calls(page, "open")).toEqual([{ uris: ["/media/Music/02 Risingson.flac"], play: true }]);
  await page.getByRole("button", { name: "Play", exact: true }).click();
  expect((await calls(page, "open"))[1]).toEqual({ uris: ["/media/Music/01 Angel.flac", "/media/Music/02 Risingson.flac"], play: true });

  await page.getByRole("button", { name: "Albums" }).click();
  await page.getByRole("radio", { name: "Songs" }).click();
  await expect(page.getByRole("list", { name: "Songs" }).getByRole("listitem")).toHaveCount(3);
  await expect(page.getByText("Field recording")).toBeVisible();
});

test("a playlist is made, renamed, played and deleted", async ({ page }) => {
  await open(page, "home");
  await section(page, "Playlists").click();
  await expect(page.getByRole("list", { name: "Playlists" }).getByRole("listitem")).toHaveCount(1);

  await page.getByRole("button", { name: "New playlist" }).click();
  const name = page.getByRole("textbox", { name: "Playlist name" });
  await expect(name).toBeFocused();
  // A taken name is said so, and the box stays to change it.
  await name.fill("road trip");
  await name.press("Enter");
  await expect(page.getByText("There's already a playlist called")).toBeVisible();
  await expect(page.getByRole("dialog", { name: "New playlist" })).toBeVisible();
  await name.fill("Evening");
  await page.getByRole("button", { name: "Create" }).click();
  expect((await calls(page, "playlists_create")).at(-1)).toEqual({ name: "Evening", fromQueue: false });

  // It opens on the new, empty playlist.
  await expect(page.getByRole("heading", { level: 2, name: "Evening" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Play", exact: true })).toBeDisabled();
  await page.getByRole("button", { name: "Rename" }).click();
  await page.getByRole("textbox", { name: "Playlist name" }).fill("Late evening");
  await page.getByRole("button", { name: "Rename", exact: true }).last().click();
  await expect(page.getByRole("heading", { level: 2, name: "Late evening" })).toBeVisible();

  await page.getByRole("button", { name: "Playlists", exact: true }).last().click();
  await page.getByRole("button", { name: /^Road trip/ }).click();
  await page.getByRole("list", { name: "Items in Road trip" }).getByRole("button", { name: /Sintel/ }).first().click();
  expect(await calls(page, "playlists_play")).toEqual([{ id: "p1", index: 1 }]);
  await page.getByRole("button", { name: "Remove Sintel from the playlist" }).click();
  await expect(page.getByRole("list", { name: "Items in Road trip" }).getByRole("listitem")).toHaveCount(1);

  // Deleting takes a second click.
  await page.getByRole("button", { name: "Delete" }).click();
  expect(await calls(page, "playlists_delete")).toEqual([]);
  await page.getByRole("button", { name: "Really delete?" }).click();
  expect(await calls(page, "playlists_delete")).toEqual([{ id: "p1" }]);
  await expect(page.getByRole("list", { name: "Playlists" }).getByRole("listitem")).toHaveCount(1);
});

test("a mounted share is browsed folder by folder and played from the file picked", async ({ page }) => {
  await open(page, "home");
  await section(page, "Network shares").click();
  await page.getByRole("list", { name: "Mounted shares" }).getByRole("button", { name: /films/ }).click();
  const contents = page.getByRole("list", { name: "Contents of films" });
  await expect(contents.getByRole("listitem")).toHaveCount(3);
  // The file picked, then the rest of the folder; not the folder above it.
  await contents.getByRole("button", { name: "Pilot.mkv" }).click();
  expect(await calls(page, "open")).toEqual([{ uris: ["/Volumes/films/Pilot.mkv", "/Volumes/films/Theme.mp3"], play: true }]);
  await page.getByRole("button", { name: "Add to library" }).click();
  expect(await calls(page, "library_add")).toEqual([{ path: "/Volumes/films" }]);

  await contents.getByRole("button", { name: "Season 1" }).click();
  await expect(page.getByRole("list", { name: "Contents of Season 1" }).getByRole("button", { name: "Episode 1.mkv" })).toBeVisible();
  // The path leads back up, and to the start.
  await page.getByRole("button", { name: "films", exact: true }).click();
  await expect(contents.getByRole("listitem")).toHaveCount(3);
  await page.locator(".crumbs").getByRole("button", { name: "Network shares" }).click();
  await expect(page.getByRole("list", { name: "On this network" }).getByRole("listitem")).toHaveCount(2);
});

test("a saved server asks for its password, and a wrong one can be tried again", async ({ page }) => {
  await open(page, "home");
  await section(page, "Network shares").click();
  await page.getByRole("list", { name: "Saved servers" }).getByRole("button", { name: /^Films/ }).click();
  const dialog = page.getByRole("dialog", { name: "Connect to server" });
  await expect(dialog.getByRole("textbox", { name: "Server address" })).toHaveValue("ftp://nas.local/films");
  await expect(dialog.getByLabel("User name")).toHaveValue("me");
  await dialog.getByLabel("Password", { exact: true }).fill("wrong");
  await dialog.getByRole("button", { name: "Connect" }).click();
  await expect(page.getByRole("alert")).toContainText("Couldn't open that folder");

  await page.getByRole("button", { name: "Sign in…" }).click();
  await dialog.getByLabel("User name").fill("me");
  await dialog.getByLabel("Password", { exact: true }).fill("secret");
  await dialog.getByRole("button", { name: "Connect" }).click();
  await expect(page.getByRole("button", { name: "Trailer.mp4" })).toBeVisible();
  expect((await calls(page, "network_browse")).at(-1)).toEqual({ location: "ftp://nas.local/films", user: "me", password: "secret" });
  // Passwords go to the one request and nowhere else.
  expect(await calls(page, "network_save")).toEqual([]);

  await page.locator(".crumbs").getByRole("button", { name: "Network shares" }).click();
  await page.getByRole("button", { name: "Forget Films" }).click();
  await expect(page.getByRole("list", { name: "Saved servers" })).toHaveCount(0);
});

test("a stream found on the network plays, and leaving stops the search for servers", async ({ page }) => {
  await open(page, "home");
  await section(page, "Network shares").click();
  await page.getByRole("button", { name: /Lobby camera/ }).click();
  expect(await calls(page, "open")).toEqual([{ uris: ["rtp://239.0.0.1:5004"], play: true }]);
  await section(page, "Home").click();
  await expect.poll(() => calls(page, "network_leave")).toHaveLength(1);
});

test("radio stations are searched, starred and played by name", async ({ page }) => {
  await open(page, "home");
  await section(page, "Internet radio").click();
  // No favourites yet: it opens on the directory's most played.
  const stations = page.getByRole("list", { name: "Stations", exact: true });
  await expect(stations.getByRole("listitem")).toHaveCount(3);
  await page.getByRole("button", { name: "Jazz", exact: true }).click();
  await expect(stations.getByRole("listitem")).toHaveCount(1);
  expect((await calls(page, "radio_stations")).at(-1)).toEqual({ query: "", tag: "Jazz" });

  await page.getByRole("button", { name: "Add Jazz FM to favourites" }).click();
  await page.getByRole("button", { name: "Favourites", exact: true }).click();
  const favourites = page.getByRole("list", { name: "Favourite stations" });
  await favourites.getByRole("button", { name: /^Jazz FM/ }).click();
  expect(await calls(page, "radio_play")).toEqual([{ station: { id: "s2", name: "Jazz FM", url: "https://radio.example/s2", country: "GB", tags: ["jazz"], codec: "MP3", bitrate: 128 } }]);
  await page.getByRole("button", { name: "Remove Jazz FM from favourites" }).click();
  await expect(page.getByText("No favourites yet")).toBeVisible();

  const search = page.getByRole("textbox", { name: "Search stations" });
  await search.fill("zzz");
  await search.press("Enter");
  await expect(page.getByText("No stations match that search.")).toBeVisible();
});

test("Space on a station's star stars it, as Return does", async ({ page }, info) => {
  await open(page, "home");
  await section(page, "Internet radio").click();
  // Tab from the station to its star. (On a Mac, WebKit's plain Tab skips buttons.)
  await page.getByRole("list", { name: "Stations", exact: true }).getByRole("button", { name: /^Jazz FM/ }).focus();
  await page.keyboard.press(isMacProject(info.project.name) && process.platform === "darwin" ? "Alt+Tab" : "Tab");
  await expect(page.getByRole("button", { name: "Add Jazz FM to favourites" })).toBeFocused();
  await page.keyboard.press("Space");
  await expect(page.getByRole("button", { name: "Remove Jazz FM from favourites" })).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("button", { name: "Add Jazz FM to favourites" })).toBeVisible();
  expect(await calls(page, "radio_favorite")).toHaveLength(2);
  // Nothing started playing.
  expect(await calls(page, "toggle_pause")).toHaveLength(0);
  expect(await calls(page, "open")).toHaveLength(0);
});

test("an unreachable radio directory says so and can be tried again", async ({ page }) => {
  await open(page, "home");
  await page.evaluate(() => (window.__mock.failing.radio_stations = "Couldn't reach the radio directory. Check your internet connection"));
  await section(page, "Internet radio").click();
  await expect(page.getByRole("alert")).toContainText("Couldn't reach the radio directory");
  // It isn't asked over and over while it's down.
  await page.waitForTimeout(300);
  expect(await calls(page, "radio_stations")).toHaveLength(1);
  await page.evaluate(() => delete window.__mock.failing.radio_stations);
  await page.getByRole("button", { name: "Try again" }).click();
  await expect(page.getByRole("list", { name: "Stations", exact: true }).getByRole("listitem")).toHaveCount(3);
});
