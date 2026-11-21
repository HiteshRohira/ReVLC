// Progressive enhancements. The page reads fine without any of this.
import { detectDesktopOS } from "./download-platform.js";

const root = document.documentElement;

// The seek bar under the nav: fills with scroll, with a tick where each section starts.
const seek = document.querySelector(".seek");
if (seek) {
  const fill = seek.querySelector(".seek-fill");
  const ticks = seek.querySelector(".seek-ticks");
  const chapters = [...document.querySelectorAll("[data-chapter]")];
  let max = 1;

  const layout = () => {
    max = Math.max(1, root.scrollHeight - innerHeight);
    ticks.replaceChildren(
      ...chapters.map((el) => {
        const tick = document.createElement("span");
        const top = el.getBoundingClientRect().top + scrollY - parseFloat(getComputedStyle(root).scrollPaddingTop || 0);
        tick.style.left = `${Math.min(100, (top / max) * 100)}%`;
        return tick;
      }),
    );
    update();
  };
  const update = () => fill.style.setProperty("--progress", Math.min(1, scrollY / max).toFixed(4));

  addEventListener("scroll", update, { passive: true });
  addEventListener("resize", layout);
  addEventListener("load", layout);
  layout();
}

// Keep one source for release URLs and compatibility copy: the no-JS cards.
const os = detectDesktopOS({
  userAgentData: navigator.userAgentData,
  platform: navigator.platform,
  userAgent: navigator.userAgent,
  maxTouchPoints: navigator.maxTouchPoints,
});
const fallback = document.querySelector("[data-download-fallback]");
const dialog = document.querySelector("#downloads-dialog");
if (dialog && typeof dialog.showModal === "function" && fallback) {
  const cards = [...fallback.querySelectorAll("[data-os]")];
  const selected = cards.find((card) => card.dataset.os === os);
  const names = { mac: "Mac", windows: "Windows", linux: "Linux" };
  let opener;
  const openDownloads = (event) => {
    event.preventDefault();
    opener = event.currentTarget;
    if (!dialog.open) dialog.showModal();
  };
  for (const primary of document.querySelectorAll("[data-primary-download]")) {
    if (selected) {
      primary.href = selected.querySelector("a").href;
      primary.querySelector("[data-primary-label]").textContent = `Download for ${names[os]}`;
    } else {
      primary.setAttribute("aria-haspopup", "dialog");
      primary.setAttribute("aria-controls", dialog.id);
      primary.addEventListener("click", openDownloads);
    }
  }
  for (const button of document.querySelectorAll("[data-other-downloads]")) {
    button.hidden = !selected;
    button.addEventListener("click", openDownloads);
  }
  const requirements = selected?.querySelector(".platform-req").textContent || "macOS · Windows · Experimental Linux";
  document.querySelectorAll("[data-primary-requirements]").forEach((el) => el.textContent = requirements);
  document.querySelector("[data-download-title]").textContent = selected ? (os === "mac" ? "macOS" : names[os]) : "Choose your platform";
  const note = document.querySelector("[data-download-note]");
  if (selected) note.replaceChildren(...[...selected.querySelector(".platform-note").childNodes].map((el) => el.cloneNode(true)));
  else note.textContent = "Choose a package below. Check its system requirements before installing.";
  if (!selected) {
    dialog.querySelector("h2").textContent = "Choose your download";
    dialog.querySelector("#downloads-description").textContent = "Available for these desktop platforms.";
  }
  for (const card of cards) {
    if (card === selected) continue;
    const link = document.createElement("a");
    link.className = "download-option";
    link.href = card.querySelector("a").href;
    const title = document.createElement("strong");
    title.textContent = card.querySelector("h3").textContent;
    const detail = document.createElement("span");
    detail.textContent = card.querySelector(".platform-req").textContent;
    const arrow = document.createElement("span");
    arrow.className = "download-option-arrow";
    arrow.textContent = "↓";
    arrow.setAttribute("aria-hidden", "true");
    link.append(title, detail, arrow);
    dialog.querySelector("[data-download-options]").append(link);
  }
  dialog.querySelector(".download-close").addEventListener("click", () => dialog.close());
  dialog.addEventListener("click", (event) => {
    const box = dialog.getBoundingClientRect();
    if (event.target === dialog && (event.clientX < box.left || event.clientX > box.right || event.clientY < box.top || event.clientY > box.bottom)) dialog.close();
  });
  dialog.addEventListener("close", () => opener?.focus({ preventScroll: true }));
  document.querySelector("[data-download-feature]").hidden = false;
  fallback.hidden = true;
}

// The tour: the step nearest the middle of the screen picks the screenshot.
const steps = [...document.querySelectorAll(".step")];
const frames = [...document.querySelectorAll(".stage-frame img")];
const bars = [...document.querySelectorAll(".stage-bar span")];
if (steps.length && frames.length) {
  const show = (n) => {
    steps.forEach((el, i) => el.classList.toggle("is-active", i === n));
    frames.forEach((el, i) => el.classList.toggle("is-active", i === n));
    bars.forEach((el, i) => {
      el.classList.toggle("is-active", i === n);
      el.classList.toggle("is-done", i < n);
    });
  };
  const seen = new IntersectionObserver(
    (entries) => {
      for (const e of entries) if (e.isIntersecting) show(steps.indexOf(e.target));
    },
    { rootMargin: "-45% 0px -45% 0px" },
  );
  steps.forEach((el) => seen.observe(el));
  show(0);
}

// Accent switch, like the one in the app's Settings.
const swatches = [...document.querySelectorAll("[data-accent]")];
const setAccent = (name) => {
  if (name === "white") root.dataset.accent = "white";
  else delete root.dataset.accent;
  swatches.forEach((b) => b.setAttribute("aria-checked", String(b.dataset.accent === name)));
};
swatches.forEach((b) => b.addEventListener("click", () => setAccent(b.dataset.accent)));

// Keycaps light up when you press the matching key.
const keys = new Map([...document.querySelectorAll(".key")].map((el) => [el.dataset.key, el]));
const out = document.querySelector("[data-keys-out]");
const actions = {
  " ": "Space plays and pauses",
  arrowleft: "← jumps back 5 seconds",
  arrowright: "→ jumps forward 5 seconds",
  "[": "[ slows down by 0.25×",
  "]": "] speeds up by 0.25×",
  f: "F toggles fullscreen",
  m: "M mutes",
  v: "V turns subtitles on and off",
  "?": "? lists every shortcut",
};
const press = (k, down) => {
  const el = keys.get(k);
  if (!el) return false;
  el.classList.toggle("is-down", down);
  if (down && out) out.textContent = actions[k];
  return true;
};
addEventListener("keydown", (e) => {
  const k = e.key.toLowerCase();
  if (e.metaKey || e.ctrlKey || e.altKey || e.target.closest("input, textarea, select, dialog")) return;
  if (k === " " && e.target.closest("button, a")) return;
  if (press(k, true) && (k === " " || k.startsWith("arrow"))) {
    // Only take over Space and the arrows while the keys are on screen.
    const r = keys.get(k).getBoundingClientRect();
    if (r.top > 0 && r.bottom < innerHeight) e.preventDefault();
  }
});
addEventListener("keyup", (e) => press(e.key.toLowerCase(), false));
for (const [k, el] of keys) {
  el.addEventListener("pointerdown", () => press(k, true));
  el.addEventListener("pointerup", () => press(k, false));
  el.addEventListener("pointerleave", () => press(k, false));
}
