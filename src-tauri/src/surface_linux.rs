//! Composite libVLC's CPU frames and WebKit's transparent controls in one
//! GTK draw. WebKit clears its background, so a sibling underlay is erased.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gtk::cairo::{Format, ImageSurface, Operator};
use gtk::glib::{ControlFlow, Propagation};
use gtk::prelude::*;
use webkit2gtk::{HardwareAccelerationPolicy, SettingsExt, WebViewExt};

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

/// GTK and Cairo stay on the main thread; video callbacks only hand over
/// owned pixels. No external video child window can cover the controls.
pub fn attach(webview: &webkit2gtk::WebView, frames: Arc<Mutex<Option<PresentedFrame>>>) -> Result<(), String> {
    let native_draw = webview.class().upcast_ref::<gtk::Widget>().as_ref().draw.ok_or("WebKit's GTK draw method is missing")?;
    if let Some(settings) = WebViewExt::settings(webview) {
        // Our composition needs WebKit's Cairo output, including its alpha.
        settings.set_hardware_acceleration_policy(HardwareAccelerationPolicy::Never);
    }
    webview.set_background_color(&gtk::gdk::RGBA::new(0.0, 0.0, 0.0, 0.0));
    let surface: Rc<RefCell<Option<ImageSurface>>> = Rc::default();
    let drawing = surface.clone();
    webview.connect_draw(move |view, cr| {
        if cr.save().is_err() {
            return Propagation::Proceed;
        }
        // WebKit uses SOURCE when clearing its surface. Capture that draw
        // in a group, then place the resulting alpha layer over the video.
        cr.push_group();
        // SAFETY: this is the view's own GtkWidget draw vfunc, called on
        // the GTK thread with the active draw context. Stop the signal
        // afterwards so GTK does not call that default handler twice.
        unsafe { native_draw(view.as_ptr().cast(), cr.to_raw_none()) };
        let controls = cr.pop_group();
        cr.set_operator(Operator::Over);
        cr.set_source_rgb(0.0, 0.0, 0.0);
        let _ = cr.paint();
        if let Some(image) = drawing.borrow().as_ref() {
            let (w, h) = (view.allocated_width() as f64, view.allocated_height() as f64);
            let (iw, ih) = (image.width() as f64, image.height() as f64);
            let scale = (w / iw).min(h / ih);
            if cr.save().is_ok() {
                cr.translate((w - iw * scale) / 2.0, (h - ih * scale) / 2.0);
                cr.scale(scale, scale);
                if cr.set_source_surface(image, 0.0, 0.0).is_ok() {
                    let _ = cr.paint();
                }
                let _ = cr.restore();
            }
        }
        if let Ok(controls) = controls
            && cr.set_source(&controls).is_ok()
        {
            let _ = cr.paint();
        }
        let _ = cr.restore();
        Propagation::Stop
    });
    let weak_view = webview.downgrade();
    gtk::glib::timeout_add_local(Duration::from_millis(16), move || {
        let Some(view) = weak_view.upgrade() else { return ControlFlow::Break };
        let frame = frames.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(frame) = frame
            && let Ok(image) = ImageSurface::create_for_data(frame.pixels, Format::Rgb24, frame.width, frame.height, frame.stride)
        {
            *surface.borrow_mut() = Some(image);
            view.queue_draw();
        }
        ControlFlow::Continue
    });
    Ok(())
}
