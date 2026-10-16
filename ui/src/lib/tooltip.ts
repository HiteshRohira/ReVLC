// Hints for elements with data-tip, drawn like the app's menus. The
// webview's own, from title=, are Windows tooltips in system colours that
// can outlive the pointer.

/** How long the pointer rests before the first hint shows. */
const DELAY = 600;
/** Within this long of one hiding, the next shows at once, as native ones do. */
const WARM = 400;
const GAP = 6;

export function installTooltips() {
  const tip = document.createElement("div");
  tip.className = "tip";
  tip.setAttribute("role", "tooltip");
  document.body.append(tip);

  let target: HTMLElement | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let hiddenAt = 0;

  function place(el: HTMLElement) {
    tip.textContent = el.dataset.tip ?? "";
    tip.classList.add("on");
    const r = el.getBoundingClientRect();
    const { offsetWidth: w, offsetHeight: h } = tip;
    // Below the element, or above it near the bottom of the window.
    let top = r.bottom + GAP;
    if (top + h > innerHeight - GAP) top = r.top - h - GAP;
    const left = Math.max(GAP, Math.min(r.left + r.width / 2 - w / 2, innerWidth - w - GAP));
    tip.style.left = `${left}px`;
    tip.style.top = `${Math.max(GAP, top)}px`;
  }

  function show(el: HTMLElement) {
    clearTimeout(timer);
    target = el;
    const go = () => {
      if (target !== el || !el.isConnected || !el.dataset.tip) return;
      // A name that is its own hint only needs one when it's cut off.
      if (el.dataset.tip === el.textContent?.trim() && el.scrollWidth <= el.clientWidth) return;
      place(el);
    };
    if (performance.now() - hiddenAt < WARM) go();
    else timer = setTimeout(go, DELAY);
  }

  function hide() {
    clearTimeout(timer);
    if (tip.classList.contains("on")) hiddenAt = performance.now();
    tip.classList.remove("on");
    target = null;
  }

  document.addEventListener("pointerover", (e) => {
    const el = (e.target as Element).closest<HTMLElement>("[data-tip]");
    if (el === target) return;
    hide();
    if (el) show(el);
  });
  // Leaving the window.
  document.addEventListener("pointerout", (e) => {
    if (!e.relatedTarget) hide();
  });
  document.addEventListener("focusin", (e) => {
    const el = (e.target as Element).closest<HTMLElement>("[data-tip]");
    if (el?.matches(":focus-visible")) show(el);
  });
  document.addEventListener("focusout", hide);
  document.addEventListener("pointerdown", hide, true);
  document.addEventListener("keydown", hide, true);
  document.addEventListener("scroll", hide, true);
  window.addEventListener("blur", hide);
}
