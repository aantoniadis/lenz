//! gpui's X11 backend draws a frame after input but presents it only on its next refresh tick, up to one
//! frame later. It presents at once after an Expose, so after input Lenz sends its own window one.
//! Wayland presents promptly by itself and is left alone.

use gpui::Window;

#[cfg(not(target_os = "macos"))]
pub(super) fn soon(window: &Window) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::sync::OnceLock;
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ConnectionExt, EventMask, ExposeEvent, EXPOSE_EVENT};
    use x11rb::rust_connection::RustConnection;

    let Ok(handle) = HasWindowHandle::window_handle(window) else { return };
    let RawWindowHandle::Xcb(handle) = handle.as_raw() else { return };
    // A connection of our own: gpui's is not shared with the application.
    static CONNECTION: OnceLock<Option<RustConnection>> = OnceLock::new();
    let Some(connection) = CONNECTION.get_or_init(|| x11rb::connect(None).ok().map(|(c, _)| c)) else { return };
    let window = handle.window.get();
    let expose = ExposeEvent { response_type: EXPOSE_EVENT, sequence: 0, window, x: 0, y: 0, width: 1, height: 1, count: 0 };
    let _ = connection.send_event(false, window, EventMask::EXPOSURE, expose);
    let _ = connection.flush();
    // nothing is selected on this connection; drop any errors queued for a window that has closed
    while let Ok(Some(_)) = connection.poll_for_event() {}
}

#[cfg(target_os = "macos")]
pub(super) fn soon(_: &Window) {}
