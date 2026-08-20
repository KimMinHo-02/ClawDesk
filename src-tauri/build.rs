//! Build script: ensures the scaffold app icon exists, then runs tauri-build.
//!
//! tauri-build embeds `icons/icon.ico` into the Windows executable resource,
//! so the file must exist before it runs. The Phase 0 scaffold never shipped
//! an icon, so this script writes a deterministic 8x8 placeholder icon when
//! the file is missing. It never overwrites an existing icon (a real brand
//! icon may be added in a later phase, e.g. Phase 10).

fn main() {
    ensure_scaffold_icon();
    tauri_build::build()
}

/// Minimal valid 8x8, 1-bit color depth ICO (solid dark placeholder).
const SCAFFOLD_ICON: &[u8] = &[
    // ICONDIR
    0x00, 0x00, // reserved
    0x01, 0x00, // type: icon
    0x01, 0x00, // image count
    // ICONDIRENTRY
    0x08, // width (8)
    0x08, // height (8)
    0x00, // color count (0 = use biClrUsed)
    0x00, // reserved
    0x01, 0x00, // planes
    0x01, 0x00, // bit count
    0x70, 0x00, 0x00, 0x00, // bytes in resource (112)
    0x16, 0x00, 0x00, 0x00, // offset (22)
    // BITMAPINFOHEADER
    0x28, 0x00, 0x00, 0x00, // biSize (40)
    0x08, 0x00, 0x00, 0x00, // biWidth (8)
    0x10, 0x00, 0x00, 0x00, // biHeight (16: image + AND mask)
    0x01, 0x00, // biPlanes (1)
    0x01, 0x00, // biBitCount (1)
    0x00, 0x00, 0x00, 0x00, // biCompression (BI_RGB)
    0x00, 0x00, 0x00, 0x00, // biSizeImage (0 for BI_RGB)
    0x00, 0x00, 0x00, 0x00, // biXPelsPerMeter
    0x00, 0x00, 0x00, 0x00, // biYPelsPerMeter
    0x00, 0x00, 0x00, 0x00, // biClrUsed
    0x00, 0x00, 0x00, 0x00, // biClrImportant
    // palette (2 entries)
    0x00, 0x00, 0x00, 0x00, // color 0 (black)
    0x08, 0x08, 0x08, 0x00, // color 1 (dark gray)
    // pixel data (8 rows x 1 byte + 3 bytes padding), all index 0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // AND mask (8 rows x 1 byte + 3 bytes padding), all 0 = opaque
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// Writes the placeholder icon when `icons/icon.ico` is missing.
///
/// An existing icon is never overwritten (a later phase may ship a real
/// brand icon). Write failures are ignored: tauri-build reports the missing
/// icon itself.
fn ensure_scaffold_icon() {
    let manifest_dir = match std::env::var("CARGO_MANIFEST_DIR") {
        Ok(dir) => dir,
        Err(_) => return,
    };
    let icon_path = std::path::Path::new(&manifest_dir).join("icons/icon.ico");
    if icon_path.exists() {
        return;
    }
    let icons_dir = icon_path
        .parent()
        .expect("icon path has a parent directory");
    if std::fs::create_dir_all(icons_dir).is_err() {
        return;
    }
    let _ = std::fs::write(&icon_path, SCAFFOLD_ICON);
}
