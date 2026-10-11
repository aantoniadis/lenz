//! The Lenz logo in the macOS Dock and app switcher.
//!
//! The Dock shows a bare executable's generic icon. Release packages therefore put the GUI in
//! `Lenz.app` (packaging/package.sh), whose icon shows from launch to quit. Outside a bundle (a
//! development build, `cargo run`) the GUI sets the application icon at runtime instead, from a
//! 256×256 export of `assets/lenz-icon.png` made by `scripts/export-about-assets.py`: the largest a
//! Dock or app-switcher tile draws (128 pt on Retina). It is set once, after the first frame has been
//! presented, so first paint does no image work; the generic icon still shows until then and while
//! the process quits. Linux identifies the app through `app_id` and the installed desktop entry.

use objc2::{AllocAnyThread, MainThreadMarker, rc::Retained};
use objc2_app_kit::{NSApplication, NSImage};
use objc2_foundation::NSData;

const PNG: &[u8] = include_bytes!("../../../assets/lenz-dock.png");

fn decode(png: &[u8]) -> Option<Retained<NSImage>> {
    NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(png))
}

/// Is `executable` the program of an app bundle (`….app/Contents/MacOS/…`)? Its bundle icon is
/// already in the Dock from launch to quit.
fn bundled(executable: &std::path::Path) -> bool {
    executable.parent().is_some_and(|macos| {
        macos.file_name() == Some("MacOS".as_ref())
            && macos.parent().is_some_and(|contents| {
                contents.file_name() == Some("Contents".as_ref())
                    && contents.parent().and_then(|app| app.extension()) == Some("app".as_ref())
            })
    })
}

/// Show the logo in the Dock, for an executable outside a bundle (a development build). Main thread
/// only; false (and nothing done) inside a bundle, elsewhere, or on a failed decode.
#[allow(unsafe_code)] // one AppKit setter, see SAFETY
pub fn install() -> bool {
    // Launched through a symlink (Homebrew's `bin/lenz`), the resolved path is the bundle's.
    if std::env::current_exe().and_then(std::fs::canonicalize).is_ok_and(|exe| bundled(&exe)) {
        return false;
    }
    let (Some(mtm), Some(image)) = (MainThreadMarker::new(), decode(PNG)) else { return false };
    // SAFETY: on the main thread (`mtm`), with a live, retained NSImage; AppKit copies what it needs.
    unsafe { NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&image)) };
    true
}

#[cfg(test)]
fn image_size(png: &[u8]) -> Option<objc2_foundation::NSSize> {
    decode(png).map(|image| image.size())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dock_icon_is_the_logo_at_dock_resolution() {
        let image = image::load_from_memory(PNG).unwrap();
        assert_eq!((image.width(), image.height()), (256, 256));
        assert!(image.color().has_alpha(), "the rounded corners are transparent");
    }

    #[test]
    fn a_bundled_executable_keeps_the_bundle_icon() {
        use std::path::Path;
        assert!(bundled(Path::new("/opt/homebrew/Cellar/lenz/0.2.0/Lenz.app/Contents/MacOS/lenz")));
        assert!(!bundled(Path::new("/Users/me/dev/lenz/target/release/lenz")));
        assert!(!bundled(Path::new("/tmp/Lenz.app/lenz")), "only the bundle's executable directory");
    }

    #[test]
    fn install_refuses_off_the_main_thread() {
        assert!(!install(), "libtest threads are not the main thread");
    }

    #[test]
    fn appkit_decodes_the_embedded_icon() {
        let size = image_size(PNG).expect("NSImage from the embedded PNG");
        assert_eq!((size.width, size.height), (256.0, 256.0));
    }
}
