/** The OS the app runs on. macOS has a menu bar and traffic lights; Windows gets the UI's own. */
export const isMac = navigator.userAgent.includes("Mac OS");
export const isWindows = navigator.userAgent.includes("Windows");

/** The primary shortcut modifier: ⌘ on macOS, Ctrl elsewhere. */
export const modKey = isMac ? "⌘" : "Ctrl+";
