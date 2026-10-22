//! An X11 video drawable below the transparent WebKitGTK controls.
//! Mint's default X11 session works directly; Wayland sessions need XWayland.

use gtk::glib::translate::ToGlibPtr;
use gtk::prelude::*;
use webkit2gtk::WebViewExt;

#[link(name = "X11")]
unsafe extern "C" {
    fn XInitThreads() -> std::ffi::c_int;
}

#[link(name = "gdk-3")]
unsafe extern "C" {
    fn gdk_x11_window_get_xid(window: *mut gtk::gdk::ffi::GdkWindow) -> std::ffi::c_ulong;
}

/// Must run before GTK, libVLC or any other Xlib user is initialized.
pub fn init() {
    unsafe { XInitThreads() };
    gtk::gdk::set_allowed_backends("x11");
}

/// Runs on the GTK main thread. GTK owns the widgets and keeps the drawable
/// the size of the web view, including fullscreen and HiDPI changes.
pub fn attach(webview: &webkit2gtk::WebView) -> Result<u32, String> {
    let parent = webview.parent().and_then(|p| p.downcast::<gtk::Box>().ok()).ok_or("web view's GTK box is missing")?;
    let overlay = gtk::Overlay::new();
    overlay.set_hexpand(true);
    overlay.set_vexpand(true);
    let video = gtk::DrawingArea::new();
    video.set_hexpand(true);
    video.set_vexpand(true);
    // libVLC paints this window itself; GTK must not erase a paused frame.
    video.set_app_paintable(true);
    overlay.add(&video);
    parent.remove(webview);
    overlay.add_overlay(webview);
    parent.pack_start(&overlay, true, true, 0);
    webview.set_background_color(&gtk::gdk::RGBA::new(0.0, 0.0, 0.0, 0.0));
    overlay.show_all();
    webview.grab_focus();
    video.realize();
    let window = video.window().ok_or("video drawable was not realized")?;
    // init() restricts GTK to X11, so this is always a GdkX11Window.
    let xid = unsafe { gdk_x11_window_get_xid(window.to_glib_none().0) };
    u32::try_from(xid).ok().filter(|&id| id != 0).ok_or("invalid X11 video drawable".into())
}
