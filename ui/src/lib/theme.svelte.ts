/** Accent colours offered in onboarding and Settings. More can be added here. */
export const ACCENTS = {
  white: { name: "ReVLC White", color: "#ffffff", on: "#0b0b0c", soft: "rgb(255 255 255 / 0.12)" },
  orange: { name: "Classic Orange", color: "#ff7a1a", on: "#ffffff", soft: "rgb(255 122 26 / 0.16)" },
} as const;

export type AccentId = keyof typeof ACCENTS;

const ACCENT_KEY = "revlc.accent";
const ONBOARDED_KEY = "revlc.onboarded";

function apply(id: AccentId) {
  const a = ACCENTS[id];
  const root = document.documentElement.style;
  root.setProperty("--acc", a.color);
  root.setProperty("--on-acc", a.on);
  root.setProperty("--acc-soft", a.soft);
}

class Theme {
  accent = $state<AccentId>((localStorage.getItem(ACCENT_KEY) as AccentId) in ACCENTS
    ? (localStorage.getItem(ACCENT_KEY) as AccentId)
    : "white");
  onboarded = $state(localStorage.getItem(ONBOARDED_KEY) === "1");

  constructor() {
    apply(this.accent);
    // Keep every window in sync when another one changes the accent.
    window.addEventListener("storage", (e) => {
      if (e.key === ACCENT_KEY && e.newValue && e.newValue in ACCENTS) {
        this.accent = e.newValue as AccentId;
        apply(this.accent);
      }
    });
  }

  setAccent(id: AccentId) {
    this.accent = id;
    localStorage.setItem(ACCENT_KEY, id);
    apply(id);
  }

  finishOnboarding() {
    this.onboarded = true;
    localStorage.setItem(ONBOARDED_KEY, "1");
  }
}

export const theme = new Theme();
