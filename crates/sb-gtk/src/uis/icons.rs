//! The icons of the web version (ui/src/components/Icon.tsx): the same 24×24 stroke paths, drawn in any colour.

use gtk::gdk;
use gtk::glib;

pub const MUTED: &str = "#8f97bd";
pub const FAINT: &str = "#5c6490";
pub const TEXT: &str = "#e8eaf6";
pub const ACCENT: &str = "#ff7a45";
pub const SERVICE: &str = "#7886ff";
pub const TOPIC: &str = "#ffb04a";
pub const OK: &str = "#3ecf8e";
pub const WARN: &str = "#ffc53d";
pub const ERR: &str = "#ff6678";

fn path(name: &str) -> &'static str {
    match name {
        "minus" => "M5 12h14",
        "plus" => "M12 5v14M5 12h14",
        "trash" => "M3 6h18M8 6V4h8v2M6 6l1 14h10l1-14M10 11v6M14 11v6",
        "up" => "M6 15l6-6 6 6",
        "down" => "M6 9l6 6 6-6",
        "right" => "M9 6l6 6-6 6",
        "x" => "M6 6l12 12M18 6L6 18",
        "service" => "M4 5h16v6H4zM4 13h16v6H4zM8 8h.01M8 16h.01",
        "topic" => "M3 7l9-4 9 4-9 4-9-4zM3 12l9 4 9-4M3 17l9 4 9-4",
        "schema" => {
            "M8 4c-2 0-3 1-3 3v2c0 1-1 2-2 3 1 1 2 2 2 3v2c0 2 1 3 3 3M16 4c2 0 3 1 3 3v2c0 1 1 2 2 3-1 1-2 2-2 3v2c0 2-1 3-3 3"
        }
        "log" => "M6 3h9l4 4v14H6zM14 3v5h5M9 13h7M9 17h7",
        "let" => "M5 9h14M5 15h14",
        "publish" => "M22 2L11 13M22 2l-7 20-4-9-9-4z",
        "publishLive" => {
            "M12 13a1 1 0 100-2 1 1 0 000 2zM8.5 8.5a5 5 0 000 7M15.5 8.5a5 5 0 010 7M5.5 5.5a9 9 0 000 13M18.5 5.5a9 9 0 010 13"
        }
        "callRpc" => "M7 7h13l-4-4M17 17H4l4 4",
        "branch" => {
            "M6 3v12M18 9a3 3 0 100-6 3 3 0 000 6zM6 21a3 3 0 100-6 3 3 0 000 6zM18 9c0 6-12 3-12 6"
        }
        "respond" => "M20 4v7a4 4 0 01-4 4H4M9 10l-5 5 5 5",
        "signJwt" => "M4 7h16v10H4zM8 12h8M14 9l3 3-3 3",
        "verifyJwt" => "M4 7h16v10H4zM8 12l2.5 2.5L16 9",
        "jwks" => "M12 3a4 4 0 100 8 4 4 0 000-8zM12 11v10M12 17h3M12 20h2",
        "totpGenerate" => "M7 3h10v18H7zM11 18h2M10 8h4v4h-4z",
        "totpVerify" => "M7 3h10v18H7zM11 18h2M9.5 11l2 2 3.5-4",
        "recoveryCodes" => "M6 3h12v18H6zM9 8h6M9 12h6M9 16h4",
        "recoveryCodeUse" => "M6 3h12v18H6zM9 12l2 2 4-4",
        "passkeyRegisterStart" => "M9 11a3 3 0 100-6 3 3 0 000 6zM3 20c0-3 3-5 6-5M15 15h6M18 12v6",
        "passkeyRegisterFinish" => {
            "M9 11a3 3 0 100-6 3 3 0 000 6zM3 20c0-3 3-5 6-5M14 17l2.5 2.5L21 14"
        }
        "passkeyLoginStart" => {
            "M9 11a3 3 0 100-6 3 3 0 000 6zM3 20c0-3 3-5 6-5M15 12h6M18 9l3 3-3 3"
        }
        "passkeyLoginFinish" => "M9 11a3 3 0 100-6 3 3 0 000 6zM3 20c0-3 3-5 6-5M14 14l3 3 5-6",
        "imageSniff" => "M4 5h16v14H4zM4 16l5-5 4 4 3-3 4 4M9 9.5a1 1 0 100-2 1 1 0 000 2z",
        "objectPut" => "M12 16V4M7 9l5-5 5 5M4 20h16",
        "objectGet" => "M12 4v12M7 11l5 5 5-5M4 20h16",
        "objectPresignGet" => {
            "M10 14a4 4 0 005.7 0l3-3a4 4 0 00-5.7-5.7l-1 1M14 10a4 4 0 00-5.7 0l-3 3a4 4 0 005.7 5.7l1-1"
        }
        "objectPresignPut" => {
            "M10 14a4 4 0 005.7 0l3-3a4 4 0 00-5.7-5.7l-1 1M14 10a4 4 0 00-5.7 0l-3 3a4 4 0 005.7 5.7l1-1M12 21v-5M9.500 18.500L12 16l2.500 2.500"
        }
        "objectDelete" => "M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13M10 11v5M14 11v5",
        "encrypt" => "M7 11V8a5 5 0 0110 0v3M5 11h14v10H5zM12 14v4M10 16h4",
        "decrypt" => "M7 11V8a5 5 0 019.6-2M5 11h14v10H5zM12 14v4",
        "verifyHmac" => "M12 3l8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6zM9 11h6M12 11v5",
        "rateLimit" => "M12 22a10 10 0 100-20 10 10 0 000 20zM12 6v6l4 2",
        "hashPassword" => "M7 11V8a5 5 0 0110 0v3M5 11h14v10H5zM12 15v2",
        "verifyPassword" => "M12 3l8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6zM8.5 12l2.5 2.5L15.5 10",
        "randomToken" => "M3 8h18v8H3zM7 12h.01M12 12h.01M17 12h.01",
        "sha256" => "M4 9h16M4 15h16M10 3L8 21M16 3l-2 18",
        "fail" => "M12 22a10 10 0 100-20 10 10 0 000 20zM5 5l14 14",
        "code" => "M16 18l6-6-6-6M8 6l-6 6 6 6",
        "sql" => {
            "M4 6c0-1.7 3.6-3 8-3s8 1.3 8 3-3.6 3-8 3-8-1.3-8-3zM4 6v12c0 1.7 3.6 3 8 3s8-1.3 8-3V6M4 12c0 1.7 3.6 3 8 3s8-1.3 8-3"
        }
        "table" => "M3 5h18v14H3zM3 10h18M9 5v14",
        "open" => "M3 7a2 2 0 012-2h4l2 2h8a2 2 0 012 2v8a2 2 0 01-2 2H5a2 2 0 01-2-2z",
        "save" => "M5 3h11l3 3v15H5zM8 3v6h8V3M8 21v-7h8v7",
        "export" => "M12 3v12M7 10l5 5 5-5M5 21h14",
        "folder" => "M3 7a2 2 0 012-2h4l2 2h8a2 2 0 012 2v8a2 2 0 01-2 2H5a2 2 0 01-2-2z",
        "folderOpen" => {
            "M3 18V7a2 2 0 012-2h4l2 2h7a2 2 0 012 2v2M3 18l2.5-7H21l-2.5 7a2 2 0 01-1.9 1.4H5a2 2 0 01-2-1.4z"
        }
        "file" => "M6 3h9l4 4v14H6zM14 3v5h5",
        "warn" => "M12 3l10 18H2zM12 10v5M12 18h.01",
        "error" => "M12 22a10 10 0 100-20 10 10 0 000 20zM12 7v6M12 16h.01",
        "ok" => "M12 22a10 10 0 100-20 10 10 0 000 20zM8 12l3 3 5-6",
        "copy" => "M9 9h11v11H9zM5 15V4h11",
        "sparkles" => {
            "M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8zM19 16l.9 2.1L22 19l-2.1.9L19 22l-.9-2.1L16 19l2.1-.9z"
        }
        "grip" => "M9 5h.01M9 12h.01M9 19h.01M15 5h.01M15 12h.01M15 19h.01",
        "graph" => "M5 5h5v5H5zM14 14h5v5h-5zM10 7.5h4a3 3 0 013 3V14",
        "plug" => "M9 2v6M15 2v6M6 8h12v4a6 6 0 01-12 0zM12 18v4",
        "update" => "M21 12a9 9 0 11-3-6.7M21 4v5h-5M12 8v5M9.5 10.5L12 8l2.5 2.5",
        "new" => "M6 3h9l4 4v14H6zM14 3v5h5M12 11v6M9 14h6",
        _ => "M12 12h.01",
    }
}

/// A picture of the icon, `size` pixels wide, stroked with `color` (`#rrggbb`).
pub fn icon(name: &str, size: i32, color: &str) -> gtk::Image {
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="{px}" height="{px}" fill="none" stroke="{color}" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="{d}"/></svg>"#,
        px = size * 2,
        d = path(name),
    );
    let image = gtk::Image::new();
    if let Ok(texture) = gdk::Texture::from_bytes(&glib::Bytes::from_owned(svg.into_bytes())) {
        image.set_paintable(Some(&texture));
    }
    image.set_pixel_size(size);
    image
}

/// An icon centred in a badge of `badge` pixels (a tile, a step): margins make up the difference, so that the
/// badge keeps its size and does not grow.
pub fn centered(name: &str, size: i32, color: &str, badge: i32) -> gtk::Image {
    use gtk::prelude::*;
    let image = icon(name, size, color);
    let gap = ((badge - size) / 2).max(0);
    image.set_margin_start(gap);
    image.set_margin_end(gap);
    image.set_margin_top(gap);
    image.set_margin_bottom(gap);
    image
}
