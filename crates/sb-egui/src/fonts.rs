//! The fonts of the web version: Inter for the text (four weights), the system monospace for the code.
//!
//! They are taken from the system, as GTK did (fontconfig picks the closest font when Inter is not installed); if
//! fontconfig is missing, egui's own fonts stay. egui has no font weights: each weight is a family of its own,
//! used through [`Weight`] (`RichText::new(..).bold()`), [`crate::theme::bold`] and friends.

use std::sync::Arc;

use eframe::egui::{self, FontData, FontDefinitions, FontFamily, RichText};

pub const MEDIUM: &str = "medium";
pub const SEMIBOLD: &str = "semibold";
pub const BOLD: &str = "bold";
pub const MONO_BOLD: &str = "mono-bold";

fn find(pattern: &str) -> Option<Vec<u8>> {
    let out = std::process::Command::new("fc-match")
        .args(["-f", "%{file}", pattern])
        .output()
        .ok()?;
    let path = String::from_utf8(out.stdout).ok()?;
    std::fs::read(path.trim()).ok()
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    // The fonts egui ships come after ours: they hold the glyphs ours may lack (symbols, emoji).
    let mut fallback_prop = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let mut fallback_mono = fonts
        .families
        .get(&FontFamily::Monospace)
        .cloned()
        .unwrap_or_default();
    // DejaVu Sans has the arrows, box drawing and most symbols that Inter and egui's fonts lack.
    if let Some(bytes) = find("DejaVu Sans") {
        fonts
            .font_data
            .insert("dejavu".into(), Arc::new(FontData::from_owned(bytes)));
        fallback_prop.insert(0, "dejavu".into());
        fallback_mono.insert(0, "dejavu".into());
    }
    let mut add = |key: &str, pattern: &str, family: FontFamily, fallback: &[String]| {
        let Some(bytes) = find(pattern) else {
            fonts
                .families
                .entry(family)
                .or_insert_with(|| fallback.to_vec());
            return;
        };
        fonts
            .font_data
            .insert(key.into(), Arc::new(FontData::from_owned(bytes)));
        let mut list = vec![key.to_string()];
        list.extend(fallback.iter().cloned());
        fonts.families.insert(family, list);
    };
    add(
        "inter-regular",
        "Inter:style=Regular",
        FontFamily::Proportional,
        &fallback_prop,
    );
    add(
        "inter-medium",
        "Inter:style=Medium",
        FontFamily::Name(MEDIUM.into()),
        &fallback_prop,
    );
    add(
        "inter-semibold",
        "Inter:style=SemiBold",
        FontFamily::Name(SEMIBOLD.into()),
        &fallback_prop,
    );
    add(
        "inter-bold",
        "Inter:style=Bold",
        FontFamily::Name(BOLD.into()),
        &fallback_prop,
    );
    add(
        "mono-regular",
        "monospace:style=Regular",
        FontFamily::Monospace,
        &fallback_mono,
    );
    add(
        "mono-bold",
        "monospace:style=Bold",
        FontFamily::Name(MONO_BOLD.into()),
        &fallback_mono,
    );
    ctx.set_fonts(fonts);
}

/// Font weights on `RichText`, which egui only knows as families. A monospace text stays monospace.
pub trait Weight {
    fn medium(self) -> Self;
    fn semibold(self) -> Self;
    fn bold(self) -> Self;
}

impl Weight for RichText {
    fn medium(self) -> Self {
        self.family(FontFamily::Name(MEDIUM.into()))
    }
    fn semibold(self) -> Self {
        self.family(FontFamily::Name(SEMIBOLD.into()))
    }
    fn bold(self) -> Self {
        self.family(FontFamily::Name(BOLD.into()))
    }
}
