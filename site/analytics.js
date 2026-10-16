// Anonymous analytics (PostHog): page views and download clicks. Nothing is
// stored in the browser, so there's no cookie banner; each visit counts on
// its own.

// The PostHog project key (the same one the app uses). Empty turns this off.
const POSTHOG_KEY = "";
const POSTHOG_HOST = "https://us.i.posthog.com";

if (POSTHOG_KEY) {
  // PostHog's loader: queues calls until the library arrives.
  // prettier-ignore
  !function(t,e){var o,n,p,r;e.__SV||(window.posthog=e,e._i=[],e.init=function(i,s,a){function g(t,e){var o=e.split(".");2==o.length&&(t=t[o[0]],e=o[1]),t[e]=function(){t.push([e].concat(Array.prototype.slice.call(arguments,0)))}}(p=t.createElement("script")).type="text/javascript",p.crossOrigin="anonymous",p.async=!0,p.src=s.api_host.replace(".i.posthog.com","-assets.i.posthog.com")+"/static/array.js",(r=t.getElementsByTagName("script")[0]).parentNode.insertBefore(p,r);var u=e;for(void 0!==a?u=e[a]=[]:a="posthog",u.people=u.people||[],u.toString=function(t){var e="posthog";return"posthog"!==a&&(e+="."+a),t||(e+=" (stub)"),e},u.people.toString=function(){return u.toString(1)+".people (stub)"},o="init capture register register_once unregister opt_out_capturing has_opted_out_capturing opt_in_capturing reset identify alias set_config".split(" "),n=0;n<o.length;n++)g(u,o[n]);e._i.push([i,s,a])},e.__SV=1)}(document,window.posthog||[]);

  posthog.init(POSTHOG_KEY, {
    api_host: POSTHOG_HOST,
    persistence: "memory",
    person_profiles: "never",
    autocapture: false,
    capture_pageleave: false,
  });

  // Release files are named like ReVLC-0.2.3-alpha-arm64.dmg and
  // ReVLC-0.2.3-alpha-x64-setup.exe.
  document.addEventListener("click", (e) => {
    const link = e.target.closest?.('a[href*="/releases/download/"]');
    if (!link) return;
    const file = decodeURIComponent(link.pathname.split("/").pop() || "");
    const match = file.match(/^ReVLC-(.+?)-(arm64|x64|aarch64|x86_64)/i);
    posthog.capture(
      "download_clicked",
      {
        file,
        platform: /\.dmg$/i.test(file) ? "mac" : /\.(exe|msi)$/i.test(file) ? "windows" : "other",
        version: match?.[1] ?? null,
        arch: match?.[2] ?? null,
        // Which button: "top" (the hero) or "download".
        placement: link.closest("section[id]")?.id ?? null,
      },
      { transport: "sendBeacon" },
    );
  });
}
