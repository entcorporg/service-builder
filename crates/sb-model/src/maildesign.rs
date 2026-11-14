//! The visual form of a mail: blocks (heading, paragraph, button, image, divider, spacer, columns) and the settings of
//! the whole mail. The HTML and the plain text of the mail are *derived* from it ([`render_html`], [`render_text`]),
//! never typed: the author composes, the project stores the composition, the generator renders it.
//!
//! The text of a block can hold variables of the event, written `{{ name }}` (the syntax of the Tera templates the mail
//! service renders). In the HTML they come out escaped, because Tera escapes the `.html` template; in the plain text and
//! the subject they do not.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The mail composed as blocks. When a [`crate::MailTemplate`] has one, its `html` and `text` are ignored: they are
/// derived from this.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MailDesign {
    #[serde(default)]
    pub settings: MailSettings,
    /// The blocks, from top to bottom.
    #[serde(default)]
    pub blocks: Vec<MailBlock>,
}

/// What holds for the whole mail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MailSettings {
    /// Width of the content in pixels (320 to 800). Narrower screens shrink it.
    #[serde(default = "default_width")]
    pub width: u32,
    /// Colour of the page around the content, `#rrggbb`.
    #[serde(default = "default_page")]
    pub background: String,
    /// Colour of the content, `#rrggbb`.
    #[serde(default = "default_content")]
    pub content_background: String,
    /// Colour of the text, `#rrggbb`.
    #[serde(default = "default_text_color")]
    pub text_color: String,
    #[serde(default)]
    pub font: MailFont,
    /// A line shown by the mailbox next to the subject, before the mail is opened. Variables allowed.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub preheader: String,
    /// A small grey text under the content (sender, why the mail is received...). Variables allowed.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub footer: String,
}

fn default_width() -> u32 {
    600
}
fn default_page() -> String {
    "#f4f4f5".into()
}
fn default_content() -> String {
    "#ffffff".into()
}
fn default_text_color() -> String {
    "#222222".into()
}

impl Default for MailSettings {
    fn default() -> Self {
        Self {
            width: default_width(),
            background: default_page(),
            content_background: default_content(),
            text_color: default_text_color(),
            font: MailFont::default(),
            preheader: String::new(),
            footer: String::new(),
        }
    }
}

/// The families a mail client is sure to have: a mail cannot ship a font.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum MailFont {
    /// Helvetica, Arial.
    #[default]
    Sans,
    /// Georgia, Times New Roman.
    Serif,
    /// Courier New.
    Mono,
}

impl MailFont {
    fn css(self) -> &'static str {
        match self {
            MailFont::Sans => "Helvetica, Arial, sans-serif",
            MailFont::Serif => "Georgia, 'Times New Roman', serif",
            MailFont::Mono => "'Courier New', Courier, monospace",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum MailAlign {
    #[default]
    Left,
    Center,
    Right,
}

impl MailAlign {
    pub fn css(self) -> &'static str {
        match self {
            MailAlign::Left => "left",
            MailAlign::Center => "center",
            MailAlign::Right => "right",
        }
    }
}

/// One block of the mail. Colours are `#rrggbb`; an absent colour is the one of the settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum MailBlock {
    /// A title. `level` is 1 (largest) to 3.
    Heading {
        text: String,
        #[serde(default = "default_level")]
        level: u8,
        #[serde(default)]
        align: MailAlign,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<String>,
    },
    /// A paragraph. A line break in `text` is a line break in the mail.
    Paragraph {
        text: String,
        #[serde(default)]
        align: MailAlign,
        /// Font size in pixels (10 to 32).
        #[serde(default = "default_size")]
        size: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<String>,
    },
    /// A button that opens `url`. The url is `https://`, `http://`, `mailto:`, or a variable (`{{ link }}`).
    Button {
        label: String,
        url: String,
        #[serde(default = "center")]
        align: MailAlign,
        /// Colour of the button, `#rrggbb`.
        #[serde(default = "default_button")]
        background: String,
        /// Colour of its label, `#rrggbb`.
        #[serde(default = "default_button_text")]
        color: String,
        /// Rounded corners in pixels (0 to 40).
        #[serde(default = "default_radius")]
        radius: u32,
    },
    /// An image fetched by the mail client from `url` (an address, not an attachment).
    Image {
        url: String,
        /// What is shown when the image is not loaded, and read by screen readers.
        #[serde(default)]
        alt: String,
        /// Width in pixels; absent, the image takes the width of its place.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width: Option<u32>,
        #[serde(default = "center")]
        align: MailAlign,
        /// Where a click on the image goes; same rule as a button.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        link: Option<String>,
    },
    /// A horizontal line.
    Divider {
        #[serde(default = "default_divider")]
        color: String,
        /// Thickness in pixels (1 to 10).
        #[serde(default = "default_thickness")]
        thickness: u32,
    },
    /// Empty vertical space, in pixels (1 to 200).
    Spacer {
        #[serde(default = "default_spacer")]
        height: u32,
    },
    /// Two or three columns side by side, each with blocks of its own (but no columns inside).
    Columns { columns: Vec<MailColumn> },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MailColumn {
    #[serde(default)]
    pub blocks: Vec<MailBlock>,
}

fn default_level() -> u8 {
    1
}
fn default_size() -> u32 {
    16
}
fn center() -> MailAlign {
    MailAlign::Center
}
fn default_button() -> String {
    "#2563eb".into()
}
fn default_button_text() -> String {
    "#ffffff".into()
}
fn default_radius() -> u32 {
    6
}
fn default_divider() -> String {
    "#e4e4e7".into()
}
fn default_thickness() -> u32 {
    1
}
fn default_spacer() -> u32 {
    24
}

impl MailBlock {
    pub fn heading(text: &str) -> Self {
        MailBlock::Heading {
            text: text.into(),
            level: 1,
            align: MailAlign::Left,
            color: None,
        }
    }
    pub fn paragraph(text: &str) -> Self {
        MailBlock::Paragraph {
            text: text.into(),
            align: MailAlign::Left,
            size: default_size(),
            color: None,
        }
    }
    pub fn button(label: &str, url: &str) -> Self {
        MailBlock::Button {
            label: label.into(),
            url: url.into(),
            align: MailAlign::Center,
            background: default_button(),
            color: default_button_text(),
            radius: default_radius(),
        }
    }
    pub fn image(url: &str, alt: &str) -> Self {
        MailBlock::Image {
            url: url.into(),
            alt: alt.into(),
            width: None,
            align: MailAlign::Center,
            link: None,
        }
    }
    pub fn divider() -> Self {
        MailBlock::Divider {
            color: default_divider(),
            thickness: default_thickness(),
        }
    }
    pub fn spacer() -> Self {
        MailBlock::Spacer {
            height: default_spacer(),
        }
    }
    pub fn columns(count: usize) -> Self {
        MailBlock::Columns {
            columns: (0..count)
                .map(|_| MailColumn {
                    blocks: vec![MailBlock::paragraph("")],
                })
                .collect(),
        }
    }
}

// ---- text and variables ---------------------------------------------------------------

/// A piece of the text of a block: literal text, or a variable of the event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment<'a> {
    Text(&'a str),
    Var(&'a str),
}

fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Splits a text into literals and `{{ name }}` variables. A `{{` that is not a well-formed variable, and `{%` or `{#`
/// (Tera's statements and comments), are refused: they would be code in the mail, and the author writes none.
pub fn segments(text: &str) -> Result<Vec<Segment<'_>>, String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find('{') {
        let after = &rest[at + 1..];
        if after.starts_with('%') || after.starts_with('#') {
            return Err("`{%` and `{#` are not allowed: use the variable picker".into());
        }
        if !after.starts_with('{') {
            // A lone brace is plain text.
            out.push(Segment::Text(&rest[..at + 1]));
            rest = after;
            continue;
        }
        let Some(end) = after.find("}}") else {
            return Err("a `{{` is never closed".into());
        };
        let name = after[1..end].trim();
        if !is_ident(name) {
            return Err(format!(
                "`{{{{{}}}}}` is not a variable: a name is letters, digits and `_`",
                &after[1..end]
            ));
        }
        if at > 0 {
            out.push(Segment::Text(&rest[..at]));
        }
        out.push(Segment::Var(name));
        rest = &after[end + 2..];
    }
    if !rest.is_empty() {
        out.push(Segment::Text(rest));
    }
    Ok(out)
}

/// The variables a text uses (none when it does not parse).
pub fn variables(text: &str) -> Vec<String> {
    segments(text)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|s| match s {
            Segment::Var(v) => Some(v.to_string()),
            Segment::Text(_) => None,
        })
        .collect()
}

/// The text with each variable replaced by what `value` says (a sample, for a preview).
pub fn fill(text: &str, value: impl Fn(&str) -> String) -> String {
    segments(text)
        .unwrap_or_else(|_| vec![Segment::Text(text)])
        .into_iter()
        .map(|s| match s {
            Segment::Text(t) => t.to_string(),
            Segment::Var(v) => value(v),
        })
        .collect()
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// A text as HTML: literals escaped here, variables left to Tera (which escapes the `.html` template), line breaks as `<br>`.
fn html_text(text: &str) -> String {
    let mut out = String::new();
    for seg in segments(text).unwrap_or_else(|_| vec![Segment::Text(text)]) {
        match seg {
            Segment::Text(t) => {
                out.push_str(&escape(t).replace("\r\n", "\n").replace('\n', "<br>"))
            }
            Segment::Var(v) => out.push_str(&format!("{{{{ {v} }}}}")),
        }
    }
    out
}

/// A text as an attribute value (an address, an alt text): same as [`html_text`] without the line breaks.
fn html_attr(text: &str) -> String {
    let mut out = String::new();
    for seg in segments(text).unwrap_or_else(|_| vec![Segment::Text(text)]) {
        match seg {
            Segment::Text(t) => out.push_str(&escape(t)),
            Segment::Var(v) => out.push_str(&format!("{{{{ {v} }}}}")),
        }
    }
    out
}

/// A text as plain text: literals as they are, variables left to Tera.
fn plain(text: &str) -> String {
    let mut out = String::new();
    for seg in segments(text).unwrap_or_else(|_| vec![Segment::Text(text)]) {
        match seg {
            Segment::Text(t) => out.push_str(t),
            Segment::Var(v) => out.push_str(&format!("{{{{ {v} }}}}")),
        }
    }
    out
}

// ---- colours ----------------------------------------------------------------------------

/// `#rrggbb` as its three channels.
pub fn parse_color(s: &str) -> Option<[u8; 3]> {
    let hex = s.strip_prefix('#')?;
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    let c = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some([c(0)?, c(2)?, c(4)?])
}

/// The WCAG contrast ratio of two colours (1 to 21).
pub fn contrast(a: [u8; 3], b: [u8; 3]) -> f64 {
    let lum = |c: [u8; 3]| {
        let f = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2])
    };
    let (la, lb) = (lum(a), lum(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

// ---- HTML -------------------------------------------------------------------------------

/// The HTML of the mail: tables and inline styles, which is what mail clients render; no script, no external style.
/// The variables are `{{ name }}` placeholders for Tera.
pub fn render_html(design: &MailDesign) -> String {
    let s = &design.settings;
    let font = s.font.css();
    let mut out = String::new();
    out.push_str("<!DOCTYPE html>\n<html lang=\"fr\">\n<head>\n<meta charset=\"utf-8\">\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    // Columns stack on narrow screens (clients that ignore the query keep them side by side).
    out.push_str("<style>@media (max-width:480px) { .col { display:block !important; width:100% !important; } }</style>\n");
    out.push_str("</head>\n");
    out.push_str(&format!(
        "<body style=\"margin:0;padding:0;background-color:{bg};\">\n",
        bg = s.background
    ));
    if !s.preheader.trim().is_empty() {
        out.push_str(&format!(
            "<div style=\"display:none;max-height:0;overflow:hidden;opacity:0;color:{bg};\">{}</div>\n",
            html_text(&s.preheader),
            bg = s.background
        ));
    }
    out.push_str(&format!(
        "<table role=\"presentation\" width=\"100%\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\" bgcolor=\"{bg}\" style=\"background-color:{bg};\"><tr><td align=\"center\" style=\"padding:24px 12px;\">\n",
        bg = s.background
    ));
    out.push_str(&format!(
        "<table role=\"presentation\" width=\"{w}\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\" bgcolor=\"{cbg}\" style=\"width:100%;max-width:{w}px;background-color:{cbg};font-family:{font};color:{fg};\">\n",
        w = s.width,
        cbg = s.content_background,
        fg = s.text_color
    ));
    for block in &design.blocks {
        out.push_str(&block_html(block, s));
    }
    out.push_str("</table>\n");
    if !s.footer.trim().is_empty() {
        out.push_str(&format!(
            "<table role=\"presentation\" width=\"{w}\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\" style=\"width:100%;max-width:{w}px;\"><tr><td align=\"center\" style=\"padding:16px 24px;font-family:{font};font-size:12px;line-height:18px;color:#71717a;\">{}</td></tr></table>\n",
            html_text(&s.footer),
            w = s.width
        ));
    }
    out.push_str("</td></tr></table>\n</body>\n</html>\n");
    out
}

fn row(style: &str, inner: &str) -> String {
    format!("<tr><td style=\"{style}\">{inner}</td></tr>\n")
}

fn block_html(block: &MailBlock, s: &MailSettings) -> String {
    match block {
        MailBlock::Heading {
            text,
            level,
            align,
            color,
        } => {
            let size = match level {
                2 => 22,
                3 => 18,
                _ => 28,
            };
            let color = color.as_deref().unwrap_or(&s.text_color);
            row(
                &format!("padding:16px 24px 8px 24px;text-align:{};", align.css()),
                &format!(
                    "<h{l} style=\"margin:0;font-size:{size}px;line-height:{}px;font-weight:bold;color:{color};\">{}</h{l}>",
                    size + size / 4,
                    html_text(text),
                    l = (*level).clamp(1, 3)
                ),
            )
        }
        MailBlock::Paragraph {
            text,
            align,
            size,
            color,
        } => {
            let color = color.as_deref().unwrap_or(&s.text_color);
            row(
                &format!(
                    "padding:8px 24px;text-align:{};font-size:{size}px;line-height:{}px;color:{color};",
                    align.css(),
                    size + size / 2
                ),
                &html_text(text),
            )
        }
        MailBlock::Button {
            label,
            url,
            align,
            background,
            color,
            radius,
        } => row(
            &format!("padding:16px 24px;text-align:{};", align.css()),
            &format!(
                "<table role=\"presentation\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\" align=\"{a}\" style=\"display:inline-table;\"><tr><td bgcolor=\"{background}\" style=\"background-color:{background};border-radius:{radius}px;\"><a href=\"{}\" style=\"display:inline-block;padding:12px 28px;font-size:16px;font-weight:bold;color:{color};text-decoration:none;border-radius:{radius}px;\">{}</a></td></tr></table>",
                html_attr(url),
                html_text(label),
                a = align.css()
            ),
        ),
        MailBlock::Image {
            url,
            alt,
            width,
            align,
            link,
        } => {
            let size = match width {
                Some(w) => format!(
                    "width=\"{w}\" style=\"display:block;border:0;max-width:100%;height:auto;\""
                ),
                None => {
                    "width=\"100%\" style=\"display:block;border:0;width:100%;height:auto;\"".into()
                }
            };
            let img = format!(
                "<img src=\"{}\" alt=\"{}\" {size}>",
                html_attr(url),
                html_attr(alt)
            );
            let inner = match link {
                Some(l) if !l.trim().is_empty() => {
                    format!(
                        "<a href=\"{}\" style=\"text-decoration:none;\">{img}</a>",
                        html_attr(l)
                    )
                }
                _ => img,
            };
            // `display:block` images are aligned by the margin of a wrapper, which clients support better than `float`.
            let margin = match align {
                MailAlign::Left => "0 auto 0 0",
                MailAlign::Center => "0 auto",
                MailAlign::Right => "0 0 0 auto",
            };
            let wrapped = match width {
                Some(w) => format!(
                    "<div style=\"width:{w}px;max-width:100%;margin:{margin};\">{inner}</div>"
                ),
                None => inner,
            };
            row("padding:8px 24px;", &wrapped)
        }
        MailBlock::Divider { color, thickness } => row(
            "padding:12px 24px;",
            &format!(
                "<div style=\"height:0;border-top:{thickness}px solid {color};font-size:0;line-height:0;\">&nbsp;</div>"
            ),
        ),
        MailBlock::Spacer { height } => row(
            &format!("height:{height}px;line-height:{height}px;font-size:0;"),
            "&nbsp;",
        ),
        MailBlock::Columns { columns } => {
            let n = columns.len().max(1);
            let cells: String = columns
                .iter()
                .map(|c| {
                    let inner: String = c.blocks.iter().map(|b| block_html(b, s)).collect();
                    format!(
                        "<td class=\"col\" width=\"{}%\" valign=\"top\" style=\"width:{}%;\"><table role=\"presentation\" width=\"100%\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\">\n{inner}</table></td>",
                        100 / n,
                        100 / n
                    )
                })
                .collect();
            format!(
                "<tr><td style=\"padding:0;\"><table role=\"presentation\" width=\"100%\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\"><tr>{cells}</tr></table></td></tr>\n"
            )
        }
    }
}

// ---- plain text -------------------------------------------------------------------------

/// The plain-text version: the same content, read top to bottom, with the address of each button after its label.
pub fn render_text(design: &MailDesign) -> String {
    let mut parts: Vec<String> = Vec::new();
    for block in &design.blocks {
        block_text(block, &mut parts);
    }
    let mut out = parts.join("\n\n");
    if !design.settings.footer.trim().is_empty() {
        out.push_str("\n\n-- \n");
        out.push_str(&plain(design.settings.footer.trim()));
    }
    out.push('\n');
    out
}

fn block_text(block: &MailBlock, out: &mut Vec<String>) {
    match block {
        MailBlock::Heading { text, level, .. } => {
            let t = plain(text);
            // The first level is underlined: the plain text has no size.
            out.push(if *level <= 1 {
                format!("{t}\n{}", "=".repeat(t.chars().count().min(60)))
            } else {
                t
            });
        }
        MailBlock::Paragraph { text, .. } => {
            if !text.trim().is_empty() {
                out.push(plain(text));
            }
        }
        MailBlock::Button { label, url, .. } => {
            out.push(format!("{}: {}", plain(label), plain(url)))
        }
        MailBlock::Image { alt, link, .. } => {
            let mut line = plain(alt);
            if let Some(l) = link.as_deref().filter(|l| !l.trim().is_empty()) {
                if !line.is_empty() {
                    line.push_str(": ");
                }
                line.push_str(&plain(l));
            }
            if !line.is_empty() {
                out.push(line);
            }
        }
        MailBlock::Divider { .. } => out.push("----".into()),
        MailBlock::Spacer { .. } => {}
        MailBlock::Columns { columns } => {
            for c in columns {
                for b in &c.blocks {
                    block_text(b, out);
                }
            }
        }
    }
}

// ---- checks -----------------------------------------------------------------------------

/// One thing wrong (or doubtful) in a design, with the JSON pointer of where.
pub struct MailIssue {
    pub error: bool,
    pub code: &'static str,
    pub path: String,
    pub message: String,
}

/// Checks a design against the fields of the event it is the mail of. `base` is the pointer of the design.
pub fn check(design: &MailDesign, fields: &[&str], base: &str) -> Vec<MailIssue> {
    let mut c = Checker {
        fields,
        out: Vec::new(),
    };
    let s = &design.settings;
    if !(320..=800).contains(&s.width) {
        c.err(
            "E_MAIL_WIDTH",
            format!("{base}/settings/width"),
            "the width of a mail is 320 to 800 pixels".into(),
        );
    }
    for (name, v) in [
        ("background", &s.background),
        ("contentBackground", &s.content_background),
        ("textColor", &s.text_color),
    ] {
        c.color(v, format!("{base}/settings/{name}"));
    }
    c.text(&s.preheader, format!("{base}/settings/preheader"));
    c.text(&s.footer, format!("{base}/settings/footer"));
    if design.blocks.is_empty() {
        c.err(
            "E_MAIL_DESIGN_EMPTY",
            format!("{base}/blocks"),
            "a designed mail needs at least one block".into(),
        );
    }
    for (i, b) in design.blocks.iter().enumerate() {
        c.block(b, &format!("{base}/blocks/{i}"), s, false);
    }
    c.out
}

struct Checker<'a> {
    fields: &'a [&'a str],
    out: Vec<MailIssue>,
}

impl Checker<'_> {
    fn err(&mut self, code: &'static str, path: String, message: String) {
        self.out.push(MailIssue {
            error: true,
            code,
            path,
            message,
        });
    }
    fn warn(&mut self, code: &'static str, path: String, message: String) {
        self.out.push(MailIssue {
            error: false,
            code,
            path,
            message,
        });
    }

    fn color(&mut self, v: &str, path: String) {
        if parse_color(v).is_none() {
            self.err(
                "E_MAIL_COLOR",
                path,
                format!("`{v}` is not a colour: write it `#rrggbb`"),
            );
        }
    }

    fn size(&mut self, v: u32, range: std::ops::RangeInclusive<u32>, what: &str, path: String) {
        if !range.contains(&v) {
            self.err(
                "E_MAIL_SIZE",
                path,
                format!("{what} is {} to {} pixels", range.start(), range.end()),
            );
        }
    }

    /// The text parses, and its variables are fields of the event.
    fn text(&mut self, text: &str, path: String) {
        match segments(text) {
            Err(why) => self.err("E_MAIL_TEXT_SYNTAX", path, why),
            Ok(segs) => {
                for seg in segs {
                    if let Segment::Var(v) = seg
                        && !self.fields.contains(&v)
                    {
                        self.err(
                            "E_MAIL_VARIABLE_UNKNOWN",
                            path.clone(),
                            format!("`{v}` is not a field of the event: pick one from the list"),
                        );
                    }
                }
            }
        }
    }

    fn link(&mut self, url: &str, path: String) {
        self.text(url, path.clone());
        let u = url.trim();
        let ok = u.starts_with("{{")
            || ["https://", "http://", "mailto:"]
                .iter()
                .any(|p| u.to_lowercase().starts_with(p));
        if u.is_empty()
            || !ok
            || segments(u).is_ok_and(|segs| {
                segs.iter()
                    .any(|g| matches!(g, Segment::Text(t) if t.contains(char::is_whitespace)))
            })
        {
            self.err(
                "E_MAIL_LINK",
                path,
                format!("`{url}` is not a link: it starts with https://, http://, mailto: or is a variable, and has no space"),
            );
        }
    }

    fn block(&mut self, b: &MailBlock, p: &str, s: &MailSettings, nested: bool) {
        let page = parse_color(&s.content_background);
        let low = |this: &mut Self, fg: &str, bg: Option<[u8; 3]>, path: String| {
            if let (Some(fg), Some(bg)) = (parse_color(fg), bg)
                && contrast(fg, bg) < 3.0
            {
                this.warn(
                    "W_MAIL_CONTRAST",
                    path,
                    "this colour is hard to read on its background (contrast under 3:1)".into(),
                );
            }
        };
        match b {
            MailBlock::Heading {
                text, level, color, ..
            } => {
                self.text(text, format!("{p}/text"));
                if text.trim().is_empty() {
                    self.warn(
                        "W_MAIL_BLOCK_EMPTY",
                        p.into(),
                        "this heading is empty".into(),
                    );
                }
                if !(1..=3).contains(level) {
                    self.err(
                        "E_MAIL_SIZE",
                        format!("{p}/level"),
                        "a heading level is 1 to 3".into(),
                    );
                }
                if let Some(c) = color {
                    self.color(c, format!("{p}/color"));
                    low(self, c, page, format!("{p}/color"));
                }
            }
            MailBlock::Paragraph {
                text, size, color, ..
            } => {
                self.text(text, format!("{p}/text"));
                if text.trim().is_empty() {
                    self.warn(
                        "W_MAIL_BLOCK_EMPTY",
                        p.into(),
                        "this paragraph is empty".into(),
                    );
                }
                self.size(*size, 10..=32, "the size of a text", format!("{p}/size"));
                if let Some(c) = color {
                    self.color(c, format!("{p}/color"));
                    low(self, c, page, format!("{p}/color"));
                }
            }
            MailBlock::Button {
                label,
                url,
                background,
                color,
                radius,
                ..
            } => {
                self.text(label, format!("{p}/label"));
                if label.trim().is_empty() {
                    self.warn(
                        "W_MAIL_BLOCK_EMPTY",
                        p.into(),
                        "this button has no label".into(),
                    );
                }
                self.link(url, format!("{p}/url"));
                self.color(background, format!("{p}/background"));
                self.color(color, format!("{p}/color"));
                low(self, color, parse_color(background), format!("{p}/color"));
                self.size(
                    *radius,
                    0..=40,
                    "the rounding of a button",
                    format!("{p}/radius"),
                );
            }
            MailBlock::Image {
                url,
                alt,
                width,
                link,
                ..
            } => {
                self.link(url, format!("{p}/url"));
                self.text(alt, format!("{p}/alt"));
                if alt.trim().is_empty() {
                    self.warn(
                        "W_MAIL_IMAGE_ALT",
                        format!("{p}/alt"),
                        "an image without alt text is blank for the readers that block images"
                            .into(),
                    );
                }
                if let Some(w) = width {
                    self.size(*w, 16..=800, "the width of an image", format!("{p}/width"));
                }
                if let Some(l) = link.as_deref().filter(|l| !l.trim().is_empty()) {
                    self.link(l, format!("{p}/link"));
                }
            }
            MailBlock::Divider { color, thickness } => {
                self.color(color, format!("{p}/color"));
                self.size(
                    *thickness,
                    1..=10,
                    "the thickness of a line",
                    format!("{p}/thickness"),
                );
            }
            MailBlock::Spacer { height } => {
                self.size(*height, 1..=200, "a space", format!("{p}/height"));
            }
            MailBlock::Columns { columns } => {
                if nested {
                    self.err(
                        "E_MAIL_COLUMNS",
                        p.into(),
                        "columns cannot hold columns".into(),
                    );
                }
                if !(2..=3).contains(&columns.len()) {
                    self.err(
                        "E_MAIL_COLUMNS",
                        format!("{p}/columns"),
                        "a columns block has two or three columns".into(),
                    );
                }
                for (j, col) in columns.iter().enumerate() {
                    for (k, inner) in col.blocks.iter().enumerate() {
                        self.block(inner, &format!("{p}/columns/{j}/blocks/{k}"), s, true);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> MailDesign {
        MailDesign {
            settings: MailSettings {
                preheader: "Votre compte {{ name }}".into(),
                footer: "Envoyé à {{ email }}".into(),
                ..Default::default()
            },
            blocks: vec![
                MailBlock::heading("Bienvenue, {{ name }}"),
                MailBlock::paragraph("Votre compte <b>{{ email }}</b> est prêt.\nA bientôt."),
                MailBlock::button("Confirmer", "https://example.com/v?t={{ token }}&x=1"),
                MailBlock::divider(),
                MailBlock::spacer(),
                MailBlock::image("https://example.com/logo.png", "Logo"),
                MailBlock::columns(2),
            ],
        }
    }

    #[test]
    fn variables_are_split_from_the_text() {
        assert_eq!(
            segments("a {{ x }} b {{y}} {").unwrap(),
            vec![
                Segment::Text("a "),
                Segment::Var("x"),
                Segment::Text(" b "),
                Segment::Var("y"),
                Segment::Text(" {"),
            ]
        );
        assert!(segments("{{ a b }}").is_err());
        assert!(segments("{% if x %}").is_err());
        assert!(segments("{{ open").is_err());
        assert_eq!(variables("{{ a }} {{ b }} {{ a }}"), ["a", "b", "a"]);
        assert_eq!(fill("hi {{ n }}", |_| "Ann".into()), "hi Ann");
    }

    #[test]
    fn the_html_is_mail_safe_and_escapes_what_is_typed() {
        let html = render_html(&sample());
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(!html.contains("<script"));
        // What the author typed is escaped; the variables are left to Tera.
        assert!(
            html.contains("Votre compte &lt;b&gt;{{ email }}&lt;/b&gt; est prêt.<br>A bientôt.")
        );
        assert!(html.contains(">Bienvenue, {{ name }}</h1>"));
        assert!(html.contains("href=\"https://example.com/v?t={{ token }}&amp;x=1\""));
        assert!(html.contains("max-width:600px"));
        assert!(html.contains("Envoyé à {{ email }}"));
        assert!(html.contains("<img src=\"https://example.com/logo.png\" alt=\"Logo\""));
    }

    #[test]
    fn the_text_is_derived_too() {
        let text = render_text(&sample());
        assert_eq!(
            text,
            "Bienvenue, {{ name }}\n=====================\n\nVotre compte <b>{{ email }}</b> est prêt.\nA bientôt.\n\nConfirmer: https://example.com/v?t={{ token }}&x=1\n\n----\n\nLogo\n\n-- \nEnvoyé à {{ email }}\n"
        );
    }

    #[test]
    fn a_design_round_trips_through_json() {
        let d = sample();
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains("\"type\":\"heading\""));
        assert!(json.contains("\"contentBackground\""));
        assert_eq!(serde_json::from_str::<MailDesign>(&json).unwrap(), d);
    }

    #[test]
    fn what_is_wrong_is_pointed_at() {
        let mut d = sample();
        d.settings.width = 100;
        d.blocks[1] = MailBlock::paragraph("{{ nope }}");
        d.blocks[2] = MailBlock::button("Go", "javascript:alert(1)");
        d.blocks.push(MailBlock::Columns {
            columns: vec![MailColumn {
                blocks: vec![MailBlock::columns(2)],
            }],
        });
        let issues = check(&d, &["name", "email", "token"], "/d");
        let has = |code: &str, path: &str| issues.iter().any(|i| i.code == code && i.path == path);
        assert!(has("E_MAIL_WIDTH", "/d/settings/width"));
        assert!(has("E_MAIL_VARIABLE_UNKNOWN", "/d/blocks/1/text"));
        assert!(has("E_MAIL_LINK", "/d/blocks/2/url"));
        assert!(has("E_MAIL_COLUMNS", "/d/blocks/7/columns"));
        assert!(has("E_MAIL_COLUMNS", "/d/blocks/7/columns/0/blocks/0"));
        assert!(
            check(&sample(), &["name", "email", "token"], "/d")
                .iter()
                .all(|i| !i.error)
        );
    }

    #[test]
    fn colours_are_checked() {
        assert_eq!(parse_color("#ff0080"), Some([255, 0, 128]));
        assert_eq!(parse_color("red"), None);
        assert!(contrast([0, 0, 0], [255, 255, 255]) > 20.0);
    }
}
