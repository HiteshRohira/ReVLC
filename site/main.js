// Progressive enhancements. The page reads fine without any of this.

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

// Put the visitor's platform first.
const ua = navigator.userAgentData?.platform || navigator.platform || navigator.userAgent;
const os = /win/i.test(ua) ? "windows" : /mac/i.test(ua) ? "mac" : null;
if (os === "windows") {
  for (const group of document.querySelectorAll("[data-downloads]")) {
    const mine = group.querySelector('[data-os="windows"]');
    group.prepend(mine);
    for (const el of group.querySelectorAll("[data-os]")) {
      const btn = el.matches(".btn") ? el : el.querySelector(".btn");
      btn.classList.toggle("btn-primary", el === mine);
      btn.classList.toggle("btn-secondary", el !== mine);
    }
  }
}
if (os) document.querySelector(`.platform[data-os="${os}"]`)?.classList.add("is-yours");

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
  if (e.metaKey || e.ctrlKey || e.altKey || e.target.closest("input, textarea, select")) return;
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
