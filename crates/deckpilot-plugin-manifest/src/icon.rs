//! Navigation icon files (`contributes.navigation[].icon`, plugin API 1.3):
//! what a package may ship and what the app may show.
//!
//! PNG and WebP icons are at most [`MAX_RASTER_ICON_BYTES`] and
//! [`MAX_ICON_SIDE`] pixels a side, read from the file's header (nothing is
//! decoded). SVG icons are at most [`MAX_SVG_ICON_BYTES`], parsed as XML
//! and rebuilt from an allow-list of elements and attributes
//! ([`sanitize_svg`]): scripts, `foreignObject`, event handlers, links
//! other than to a fragment of the same file (`#id`), `<style>` with
//! `url(` or `@import`, DOCTYPEs, entities and processing instructions are
//! refused. The package reader refuses such a file at build and install
//! time; the agent sanitizes again every time it serves one, and the app
//! renders the result only through `<img>` from a `data:` URL, which runs
//! no script and loads nothing anyway.

use std::borrow::Cow;

use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};

/// Largest PNG or WebP navigation icon.
pub const MAX_RASTER_ICON_BYTES: usize = 256 * 1024;
/// Largest SVG navigation icon.
pub const MAX_SVG_ICON_BYTES: usize = 64 * 1024;
/// Largest width and height of a PNG or WebP navigation icon, in pixels.
pub const MAX_ICON_SIDE: u32 = 512;
/// Deepest element nesting of an SVG icon.
const MAX_SVG_DEPTH: usize = 64;
/// Most elements in an SVG icon.
const MAX_SVG_ELEMENTS: usize = 4096;

/// The file formats of a navigation icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IconFormat {
    Png,
    Webp,
    Svg,
}

impl IconFormat {
    /// The format a path names by its (lowercase) extension.
    pub fn of_path(path: &str) -> Option<Self> {
        let (stem, extension) = path.rsplit_once('.')?;
        if stem.is_empty() || stem.ends_with('/') {
            return None;
        }
        match extension {
            "png" => Some(Self::Png),
            "webp" => Some(Self::Webp),
            "svg" => Some(Self::Svg),
            _ => None,
        }
    }

    /// The media type of a `data:` URL of the format.
    pub fn media_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Webp => "image/webp",
            Self::Svg => "image/svg+xml",
        }
    }

    /// The largest file of the format.
    pub fn max_bytes(self) -> usize {
        match self {
            Self::Png | Self::Webp => MAX_RASTER_ICON_BYTES,
            Self::Svg => MAX_SVG_ICON_BYTES,
        }
    }
}

/// Why an icon file cannot be shipped or shown. Completes a sentence that
/// starts with the file's path.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IconError {
    #[error("is not a .png, .webp or .svg file")]
    UnknownFormat,
    #[error("is {size} bytes; the limit for this format is {limit} bytes")]
    TooLarge { size: usize, limit: usize },
    #[error("is not a valid {0} file")]
    Malformed(&'static str),
    #[error("is {width}×{height} pixels; icons are 1 to 512 pixels a side")]
    Dimensions { width: u32, height: u32 },
    #[error("is not a safe SVG: {0}")]
    UnsafeSvg(String),
}

/// Checks an icon file for the package: its format (by extension) matches
/// its content, its size, its dimensions, and for an SVG that
/// [`sanitize_svg`] accepts it.
pub fn check_icon(path: &str, bytes: &[u8]) -> Result<IconFormat, IconError> {
    display_icon(path, bytes).map(|(format, _)| format)
}

/// What the app may show of an icon file: its format and the bytes to put
/// in the `data:` URL (an SVG rebuilt by [`sanitize_svg`], a PNG or WebP
/// as it is).
pub fn display_icon<'a>(
    path: &str,
    bytes: &'a [u8],
) -> Result<(IconFormat, Cow<'a, [u8]>), IconError> {
    let format = IconFormat::of_path(path).ok_or(IconError::UnknownFormat)?;
    if bytes.len() > format.max_bytes() {
        return Err(IconError::TooLarge {
            size: bytes.len(),
            limit: format.max_bytes(),
        });
    }
    let (width, height) = match format {
        IconFormat::Svg => {
            let clean = sanitize_svg(bytes)?;
            return Ok((format, Cow::Owned(clean.into_bytes())));
        }
        IconFormat::Png => png_size(bytes).ok_or(IconError::Malformed("PNG"))?,
        IconFormat::Webp => webp_size(bytes).ok_or(IconError::Malformed("WebP"))?,
    };
    if width == 0 || height == 0 || width > MAX_ICON_SIDE || height > MAX_ICON_SIDE {
        return Err(IconError::Dimensions { width, height });
    }
    Ok((format, Cow::Borrowed(bytes)))
}

fn u32_be(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn u32_le(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn u24_le(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at + 3)?;
    Some(u32::from(b[0]) | (u32::from(b[1]) << 8) | (u32::from(b[2]) << 16))
}

/// Width and height from a PNG's signature and IHDR chunk.
fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if !bytes.starts_with(SIGNATURE)
        || u32_be(bytes, 8)? != 13
        || bytes.get(12..16)? != b"IHDR"
        // IHDR (13 bytes) and its CRC must be there.
        || bytes.len() < 33
    {
        return None;
    }
    Some((u32_be(bytes, 16)?, u32_be(bytes, 20)?))
}

/// Width and height from a WebP's RIFF header and first chunk (`VP8 `,
/// `VP8L` or `VP8X`).
fn webp_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.get(0..4)? != b"RIFF" || bytes.get(8..12)? != b"WEBP" {
        return None;
    }
    let riff = usize::try_from(u32_le(bytes, 4)?).ok()?;
    if riff.checked_add(8)? > bytes.len() || riff < 4 {
        return None;
    }
    let chunk = bytes.get(12..16)?;
    let data = 20;
    match chunk {
        b"VP8 " => {
            // Frame tag (3 bytes), start code, then 14-bit width and height.
            if bytes.get(data + 3..data + 6)? != [0x9d, 0x01, 0x2a] {
                return None;
            }
            let width = u32::from(u16::from_le_bytes([bytes[data + 6], *bytes.get(data + 7)?]));
            let height = u32::from(u16::from_le_bytes([
                *bytes.get(data + 8)?,
                *bytes.get(data + 9)?,
            ]));
            Some((width & 0x3fff, height & 0x3fff))
        }
        b"VP8L" => {
            if *bytes.get(data)? != 0x2f {
                return None;
            }
            let bits = u32_le(bytes, data + 1)?;
            Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
        }
        b"VP8X" => Some((u24_le(bytes, data + 4)? + 1, u24_le(bytes, data + 7)? + 1)),
        _ => None,
    }
}

/// Elements an icon may use. Anything else is left out with its content
/// (editor metadata, `<image>`, `<a>`, animations…); `script` and
/// `foreignObject` refuse the file.
const ELEMENTS: &[&str] = &[
    "svg",
    "g",
    "defs",
    "symbol",
    "use",
    "path",
    "rect",
    "circle",
    "ellipse",
    "line",
    "polyline",
    "polygon",
    "text",
    "tspan",
    "title",
    "desc",
    "linearGradient",
    "radialGradient",
    "stop",
    "pattern",
    "clipPath",
    "mask",
    "marker",
    "style",
    "filter",
    "feBlend",
    "feColorMatrix",
    "feComposite",
    "feDropShadow",
    "feFlood",
    "feGaussianBlur",
    "feMerge",
    "feMergeNode",
    "feOffset",
];

/// Attributes an icon may use. Anything else is left out; an event handler
/// (`on…`) refuses the file.
const ATTRIBUTES: &[&str] = &[
    "id",
    "class",
    "style",
    "transform",
    "viewBox",
    "width",
    "height",
    "x",
    "y",
    "x1",
    "y1",
    "x2",
    "y2",
    "cx",
    "cy",
    "r",
    "rx",
    "ry",
    "fx",
    "fy",
    "fr",
    "d",
    "points",
    "pathLength",
    "fill",
    "fill-opacity",
    "fill-rule",
    "stroke",
    "stroke-width",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-miterlimit",
    "stroke-dasharray",
    "stroke-dashoffset",
    "stroke-opacity",
    "opacity",
    "color",
    "display",
    "visibility",
    "overflow",
    "clip-path",
    "clip-rule",
    "mask",
    "filter",
    "preserveAspectRatio",
    "version",
    "xmlns",
    "xmlns:xlink",
    "xml:space",
    "href",
    "xlink:href",
    "gradientUnits",
    "gradientTransform",
    "spreadMethod",
    "offset",
    "stop-color",
    "stop-opacity",
    "patternUnits",
    "patternContentUnits",
    "patternTransform",
    "clipPathUnits",
    "maskUnits",
    "maskContentUnits",
    "markerWidth",
    "markerHeight",
    "markerUnits",
    "refX",
    "refY",
    "orient",
    "marker-start",
    "marker-mid",
    "marker-end",
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "text-anchor",
    "dominant-baseline",
    "letter-spacing",
    "dx",
    "dy",
    "rotate",
    "textLength",
    "lengthAdjust",
    "vector-effect",
    "shape-rendering",
    "paint-order",
    "filterUnits",
    "primitiveUnits",
    "in",
    "in2",
    "result",
    "stdDeviation",
    "mode",
    "operator",
    "k1",
    "k2",
    "k3",
    "k4",
    "values",
    "type",
    "flood-color",
    "flood-opacity",
    "role",
    "aria-hidden",
    "aria-label",
];

fn unsafe_svg(message: impl Into<String>) -> IconError {
    IconError::UnsafeSvg(message.into())
}

/// Whether CSS (a `style` attribute, or a presentation attribute that
/// may hold `url(…)`) is safe: no `@import`, no escapes, no `expression(`
/// or `javascript:`, and every `url(` names a fragment of the same file.
fn css_ok(css: &str) -> bool {
    let lower = css.to_ascii_lowercase();
    if lower.contains("@import")
        || lower.contains('\\')
        || lower.contains("expression(")
        || lower.contains("javascript:")
    {
        return false;
    }
    let mut rest = lower.as_str();
    while let Some(at) = rest.find("url(") {
        rest = &rest[at + 4..];
        let target = rest.trim_start().trim_start_matches(['"', '\'']);
        if !target.starts_with('#') {
            return false;
        }
    }
    true
}

/// A link of `href` / `xlink:href`: only a fragment of the same file
/// (`#id`).
fn local_reference(value: &str) -> bool {
    value.trim().strip_prefix('#').is_some_and(|id| {
        !id.is_empty()
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
    })
}

/// Writes the allowed attributes of `element`, refusing unsafe ones.
fn attributes(element: &BytesStart<'_>, out: &mut String) -> Result<(), IconError> {
    for attribute in element.attributes() {
        let attribute =
            attribute.map_err(|e| unsafe_svg(format!("an attribute does not parse: {e}")))?;
        let name = attribute.key.into_inner();
        let local = name.rsplit(':').next().unwrap_or(name);
        if local.len() > 2 && local[..2].eq_ignore_ascii_case("on") {
            return Err(unsafe_svg(format!("it has an event handler `{name}`")));
        }
        let value = attribute
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(|_| unsafe_svg(format!("`{name}` uses an entity")))?;
        if local == "href" {
            if !local_reference(&value) {
                return Err(unsafe_svg(format!(
                    "`{name}` links to `{}`; only links within the file (#id) are allowed",
                    truncate(&value)
                )));
            }
        } else if !css_ok(&value) {
            return Err(unsafe_svg(format!(
                "`{name}` loads or runs something (url(), @import, javascript:)"
            )));
        }
        if !ATTRIBUTES.contains(&name) {
            continue;
        }
        out.push(' ');
        out.push_str(name);
        out.push_str("=\"");
        out.push_str(&quick_xml::escape::escape(value.as_ref()));
        out.push('"');
    }
    Ok(())
}

fn truncate(text: &str) -> String {
    text.chars().take(40).collect()
}

/// Parses an SVG icon and rebuilds it from the allow-lists: only the
/// elements and attributes an icon needs, no comments, declarations or
/// metadata. Refuses (rather than strips) what could run code or load
/// anything: `script`, `foreignObject`, event handlers, links that are not
/// `#id` (external, `data:`, `javascript:`, also on `<use>`), `<style>`
/// with `url(` or `@import`, CSS escapes, DOCTYPEs and entity
/// declarations, entity references other than the five predefined ones,
/// processing instructions; and a file that is not UTF-8, not well-formed
/// or not an `<svg>` document.
pub fn sanitize_svg(bytes: &[u8]) -> Result<String, IconError> {
    if bytes.len() > MAX_SVG_ICON_BYTES {
        return Err(IconError::TooLarge {
            size: bytes.len(),
            limit: MAX_SVG_ICON_BYTES,
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| unsafe_svg("it is not UTF-8"))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut reader = Reader::from_str(text);
    reader.config_mut().check_end_names = true;
    reader.config_mut().check_comments = true;

    let mut svg = Rebuilt {
        out: String::with_capacity(text.len()),
        ..Rebuilt::default()
    };
    loop {
        let event = reader
            .read_event()
            .map_err(|e| unsafe_svg(format!("it is not well-formed XML: {e}")))?;
        match event {
            Event::Eof => break,
            Event::DocType(_) => return Err(unsafe_svg("it has a DOCTYPE")),
            Event::PI(_) => return Err(unsafe_svg("it has a processing instruction")),
            Event::Decl(_) | Event::Comment(_) => {}
            Event::Start(element) => svg.element(&element, false)?,
            Event::Empty(element) => svg.element(&element, true)?,
            Event::End(_) => svg.end()?,
            Event::Text(text) => {
                let content = text.xml10_content();
                if svg.open.is_empty() && !content.trim().is_empty() {
                    return Err(unsafe_svg("it has text outside the <svg> element"));
                }
                svg.content(&content);
            }
            Event::CData(data) => svg.content(&data.into_inner()),
            Event::GeneralRef(reference) => {
                let resolved = if reference.is_char_ref() {
                    reference
                        .resolve_char_ref()
                        .ok()
                        .flatten()
                        .map(|c| c.to_string())
                } else {
                    quick_xml::escape::resolve_predefined_entity(&reference.into_inner())
                        .map(str::to_owned)
                };
                let Some(resolved) = resolved else {
                    return Err(unsafe_svg("it uses an entity"));
                };
                svg.content(&resolved);
            }
        }
    }
    if !svg.seen_root || !svg.open.is_empty() {
        return Err(unsafe_svg("it is not an <svg> document"));
    }
    Ok(svg.out)
}

/// The SVG [`sanitize_svg`] writes, as it reads.
#[derive(Default)]
struct Rebuilt {
    out: String,
    /// Open elements: (name, written). The content of a left-out element
    /// is left out too.
    open: Vec<(String, bool)>,
    elements: usize,
    seen_root: bool,
    /// The text of the open `<style>`.
    style: String,
}

impl Rebuilt {
    /// Whether what is read now is written (no open element left out).
    fn writing(&self) -> bool {
        self.open.iter().all(|(_, written)| *written)
    }

    fn element(&mut self, element: &BytesStart<'_>, empty: bool) -> Result<(), IconError> {
        let name = element.name().into_inner().to_owned();
        let local = name.rsplit(':').next().unwrap_or(&name);
        if local.eq_ignore_ascii_case("script") || local.eq_ignore_ascii_case("foreignObject") {
            return Err(unsafe_svg(format!("it has a <{name}> element")));
        }
        self.elements += 1;
        if self.elements > MAX_SVG_ELEMENTS {
            return Err(unsafe_svg("it has too many elements"));
        }
        if self.open.is_empty() {
            if self.seen_root || name != "svg" {
                return Err(unsafe_svg("it is not an <svg> document"));
            }
            self.seen_root = true;
        }
        if self.open.len() >= MAX_SVG_DEPTH {
            return Err(unsafe_svg("its elements are nested too deeply"));
        }
        let mut tag = String::from("<");
        tag.push_str(&name);
        // Checked (and refused when unsafe) even when left out.
        attributes(element, &mut tag)?;
        let keep = self.writing() && ELEMENTS.contains(&name.as_str());
        if keep {
            self.out.push_str(&tag);
            self.out.push_str(if empty { "/>" } else { ">" });
        }
        if !empty {
            if name == "style" {
                self.style.clear();
            }
            self.open.push((name, keep));
        }
        Ok(())
    }

    fn end(&mut self) -> Result<(), IconError> {
        let Some((name, written)) = self.open.pop() else {
            return Err(unsafe_svg("it is not well-formed XML"));
        };
        if name == "style" {
            if !css_ok(&self.style) {
                return Err(unsafe_svg("its <style> loads something (url(), @import)"));
            }
            if self.style.to_ascii_lowercase().contains("url(") {
                return Err(unsafe_svg("its <style> uses url()"));
            }
        }
        if written {
            self.out.push_str("</");
            self.out.push_str(&name);
            self.out.push('>');
        }
        Ok(())
    }

    /// Text, CDATA or a resolved reference inside the document.
    fn content(&mut self, content: &str) {
        if self.open.last().is_some_and(|(name, _)| name == "style") {
            self.style.push_str(content);
        }
        if self.writing() && !self.open.is_empty() {
            self.out.push_str(&quick_xml::escape::escape(content));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SVG: &str = r##"<?xml version="1.0"?>
<!-- an editor's comment -->
<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 24 24" inkscape:version="1">
  <metadata><rdf:RDF/></metadata>
  <defs><linearGradient id="g"><stop offset="0" stop-color="#000"/></linearGradient></defs>
  <style>.a { fill: #333; }</style>
  <path class="a" d="M0 0h24v24H0z" fill="url(#g)" data-x="1"/>
  <use href="#g" xlink:href="#g"/>
  <text>A &amp; B &#65;</text>
</svg>"##;

    #[test]
    fn a_clean_svg_is_rebuilt_from_the_allow_lists() {
        let clean = sanitize_svg(SVG.as_bytes()).unwrap();
        assert_eq!(
            clean,
            "<svg xmlns=\"http://www.w3.org/2000/svg\" \
             xmlns:xlink=\"http://www.w3.org/1999/xlink\" viewBox=\"0 0 24 24\">\n  \n  \
             <defs><linearGradient id=\"g\"><stop offset=\"0\" stop-color=\"#000\"/>\
             </linearGradient></defs>\n  <style>.a { fill: #333; }</style>\n  \
             <path class=\"a\" d=\"M0 0h24v24H0z\" fill=\"url(#g)\"/>\n  \
             <use href=\"#g\" xlink:href=\"#g\"/>\n  <text>A &amp; B A</text>\n</svg>"
        );
        // The result passes again, unchanged.
        assert_eq!(sanitize_svg(clean.as_bytes()).unwrap(), clean);
    }

    fn refused(svg: &str) -> String {
        match sanitize_svg(svg.as_bytes()) {
            Err(IconError::UnsafeSvg(why)) => why,
            other => panic!("{svg}: {other:?}"),
        }
    }

    #[test]
    fn malicious_svgs_are_refused() {
        let wrap =
            |inner: &str| format!(r#"<svg xmlns="http://www.w3.org/2000/svg">{inner}</svg>"#);
        for (svg, why) in [
            (wrap("<script>alert(1)</script>"), "<script>"),
            (wrap("<svg:script/>"), "<svg:script>"),
            (
                r#"<svg xmlns="http://www.w3.org/2000/svg" onload="alert(1)"/>"#.to_owned(),
                "event handler",
            ),
            (wrap(r#"<rect ONCLICK="x()"/>"#), "event handler"),
            (
                wrap("<foreignObject><div>x</div></foreignObject>"),
                "foreignObject",
            ),
            (wrap(r#"<use href="https://evil.example/a.svg#x"/>"#), "links to"),
            (wrap(r#"<use xlink:href="other.svg#x"/>"#), "links to"),
            (wrap(r#"<image href="data:image/png;base64,AAAA"/>"#), "links to"),
            (wrap(r#"<a href="javascript:alert(1)"><rect/></a>"#), "links to"),
            (wrap(r#"<a xlink:href=" JavaScript:alert(1)"/>"#), "links to"),
            (wrap("<style>@import url(https://e.example/x.css);</style>"), "<style>"),
            (wrap("<style>rect { fill: url(https://e.example/p) }</style>"), "<style>"),
            (wrap("<style>rect { fill: url(#local) }</style>"), "url()"),
            (wrap("<style><![CDATA[@import 'x.css';]]></style>"), "<style>"),
            (wrap(r#"<rect style="background:url(http://e.example/x)"/>"#), "loads"),
            (wrap(r#"<rect fill="url(data:x)"/>"#), "loads"),
            (wrap(r#"<rect style="fill:\75rl(x)"/>"#), "loads"),
            (
                r#"<!DOCTYPE svg [<!ENTITY x "boom">]><svg xmlns="http://www.w3.org/2000/svg">&x;</svg>"#
                    .to_owned(),
                "DOCTYPE",
            ),
            (wrap("<text>&x;</text>"), "entity"),
            (wrap(r#"<rect id="&x;"/>"#), "entity"),
            (
                r#"<?xml-stylesheet href="https://e.example/x.css"?><svg/>"#.to_owned(),
                "processing instruction",
            ),
            ("<html><svg/></html>".to_owned(), "not an <svg>"),
            ("<svg><g></svg>".to_owned(), "well-formed"),
            ("<svg/><svg/>".to_owned(), "not an <svg>"),
        ] {
            let message = refused(&svg);
            assert!(message.contains(why), "{svg}: {message}");
        }
        assert!(matches!(
            sanitize_svg(&[0xff, 0xfe]),
            Err(IconError::UnsafeSvg(_))
        ));
        let big = wrap(&"<g/>".repeat(MAX_SVG_ICON_BYTES / 4));
        assert!(matches!(
            sanitize_svg(big.as_bytes()),
            Err(IconError::TooLarge { .. })
        ));
    }

    pub(crate) fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        bytes.extend(width.to_be_bytes());
        bytes.extend(height.to_be_bytes());
        bytes.extend([8, 6, 0, 0, 0, 0, 0, 0, 0]);
        bytes
    }

    fn webp(chunk: [u8; 4], data: &[u8]) -> Vec<u8> {
        let mut bytes = b"RIFF".to_vec();
        bytes.extend(u32::try_from(4 + 8 + data.len()).unwrap().to_le_bytes());
        bytes.extend(b"WEBP");
        bytes.extend(chunk);
        bytes.extend(u32::try_from(data.len()).unwrap().to_le_bytes());
        bytes.extend(data);
        bytes
    }

    #[test]
    fn raster_dimensions_come_from_the_header() {
        assert_eq!(png_size(&png(64, 32)), Some((64, 32)));
        assert_eq!(
            check_icon("assets/a.png", &png(512, 512)),
            Ok(IconFormat::Png)
        );
        assert_eq!(
            check_icon("assets/a.png", &png(513, 16)),
            Err(IconError::Dimensions {
                width: 513,
                height: 16
            })
        );
        assert_eq!(
            check_icon("assets/a.png", &png(0, 16)),
            Err(IconError::Dimensions {
                width: 0,
                height: 16
            })
        );
        assert_eq!(
            check_icon("assets/a.png", b"GIF89a.........................."),
            Err(IconError::Malformed("PNG"))
        );
        // A WebP named .png is refused: the extension decides the format.
        let lossy = webp(*b"VP8 ", &[0, 0, 0, 0x9d, 0x01, 0x2a, 64, 0, 48, 0]);
        assert_eq!(webp_size(&lossy), Some((64, 48)));
        assert_eq!(check_icon("assets/a.webp", &lossy), Ok(IconFormat::Webp));
        assert!(check_icon("assets/a.png", &lossy).is_err());
        let width = 600u32 - 1;
        let height = 20u32 - 1;
        let bits = width | (height << 14);
        let mut lossless = vec![0x2f];
        lossless.extend(bits.to_le_bytes());
        assert_eq!(webp_size(&webp(*b"VP8L", &lossless)), Some((600, 20)));
        assert!(matches!(
            check_icon("assets/a.webp", &webp(*b"VP8L", &lossless)),
            Err(IconError::Dimensions { .. })
        ));
        let extended = webp(*b"VP8X", &[0, 0, 0, 0, 127, 0, 0, 255, 0, 0]);
        assert_eq!(webp_size(&extended), Some((128, 256)));
        // A RIFF size beyond the file.
        let mut short = lossy.clone();
        short[4] = 0xff;
        assert_eq!(webp_size(&short), None);
        let mut big = png(16, 16);
        big.resize(MAX_RASTER_ICON_BYTES + 1, 0);
        assert!(matches!(
            check_icon("assets/a.png", &big),
            Err(IconError::TooLarge { .. })
        ));
        assert_eq!(
            check_icon("assets/a.gif", &png(16, 16)),
            Err(IconError::UnknownFormat)
        );
    }

    #[test]
    fn formats_by_extension() {
        assert_eq!(IconFormat::of_path("assets/n.svg"), Some(IconFormat::Svg));
        assert_eq!(IconFormat::of_path("assets/n.webp"), Some(IconFormat::Webp));
        assert_eq!(IconFormat::of_path("assets/n.PNG"), None);
        assert_eq!(IconFormat::of_path("assets/.png"), None);
        assert_eq!(IconFormat::of_path("png"), None);
        assert_eq!(IconFormat::Svg.media_type(), "image/svg+xml");
    }
}
