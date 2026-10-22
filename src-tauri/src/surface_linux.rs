//! GTK composites CPU video frames below the transparent WebKit controls.
//! A separate native VLC child window would cover the windowless web view.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gtk::cairo::{Format, ImageSurface};
use gtk::glib::{ControlFlow, Propagation};
use gtk::prelude::*;
use webkit2gtk::WebViewExt;

use crate::vlc::PresentedFrame;

#[link(name = "X11")]
unsafe extern "C" {
    fn XInitThreads() -> std::ffi::c_int;
}

/// Before GTK or libVLC initialization; X11 screen capture also uses Xlib.
pub fn init() {
    unsafe { XInitThreads() };
    gtk::gdk::set_allowed_backends("x11");
}

/// GTK owns all widgets and Cairo surfaces on its main thread. The video
/// callbacks only publish owned pixels, never touching a GTK object.
pub fn attach(webview: &webkit2gtk::WebView, frames: Arc<Mutex<Option<PresentedFrame>>>) -> Result<(), String> {
    let parent = webview.parent().and_then(|p| p.downcast::<gtk::Box>().ok()).ok_or("web view's GTK box is missing")?;
    let overlay = gtk::Overlay::new();
    overlay.set_hexpand(true);
    overlay.set_vexpand(true);
    let video = gtk::DrawingArea::new();
    // Paint in the same GTK window as WebKit, so its transparent pixels
    // reveal the video and its controls remain above it.
    video.set_has_window(false);
    video.set_hexpand(true);
    video.set_vexpand(true);
    let surface: Rc<RefCell<Option<ImageSurface>>> = Rc::default();
    let drawing = surface.clone();
    video.connect_draw(move |area, cr| {
        if cr.save().is_err() {
            return Propagation::Proceed;
        }
        cr.set_source_rgb(0.0, 0.0, 0.0);
        let _ = cr.paint();
        if let Some(image) = drawing.borrow().as_ref() {
            let (w, h) = (area.allocated_width() as f64, area.allocated_height() as f64);
            let (iw, ih) = (image.width() as f64, image.height() as f64);
            let scale = (w / iw).min(h / ih);
            cr.translate((w - iw * scale) / 2.0, (h - ih * scale) / 2.0);
            cr.scale(scale, scale);
            if cr.set_source_surface(image, 0.0, 0.0).is_ok() {
                let _ = cr.paint();
            }
        }
        let _ = cr.restore();
        Propagation::Proceed
    });
    let weak_video = video.downgrade();
    gtk::glib::timeout_add_local(Duration::from_millis(16), move || {
        let Some(video) = weak_video.upgrade() else { return ControlFlow::Break };
        let frame = frames.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(frame) = frame
            && let Ok(image) = ImageSurface::create_for_data(frame.pixels, Format::Rgb24, frame.width, frame.height, frame.stride)
        {
            *surface.borrow_mut() = Some(image);
            video.queue_draw();
        }
        ControlFlow::Continue
    });
    overlay.add(&video);
    parent.remove(webview);
    overlay.add_overlay(webview);
    parent.pack_start(&overlay, true, true, 0);
    webview.set_background_color(&gtk::gdk::RGBA::new(0.0, 0.0, 0.0, 0.0));
    overlay.show_all();
    webview.grab_focus();
    Ok(())
}
