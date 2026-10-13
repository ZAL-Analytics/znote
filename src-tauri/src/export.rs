use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt::Write as _;
use std::io::{Cursor, Write};
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use typst::introspection::{Location, Tag};
use typst::layout::{Abs, Frame, FrameItem, GroupItem, Point, Rect};
use typst::text::{Font, FontFlags, FontInstance, FontStyle, FontWeight, TextItem};
use typst::visualize::Paint;
use typst_kit::fonts;
use typst_layout::{Page, PagedDocument};
use typst_pdf::PdfOptions;
use typst_render::RenderOptions;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::compile;

const EMU_PER_POINT: f64 = 12700.0;
const SLIDE_EMU: std::ops::RangeInclusive<i64> = 914_400..=51_206_400;
const BACKGROUND_PIXELS_PER_POINT: f64 = 2.0;
const WRAP_SLACK: f64 = 1.03;

const XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#;
const NAMESPACES: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;
const RELATIONSHIPS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const OFFICE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PRESENTATION: &str = "application/vnd.openxmlformats-officedocument.presentationml";
const EMPTY_TREE: &str = r#"<p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>"#;
const COLOUR_MAP: &str = r#"<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>"#;

struct Run {
    x: f64,
    baseline: f64,
    width: f64,
    ascent: f64,
    descent: f64,
    size: f64,
    family: String,
    bold: bool,
    italic: bool,
    colour: [u8; 4],
    text: String,
    first_word: f64,
    trailing: f64,
    element: Option<Location>,
    font: FontInstance,
}

const NAME_FAMILY: u16 = 1;
const NAME_STYLE: u16 = 2;
const NAME_FULL: u16 = 4;
const NAME_VERSION: u16 = 5;
const SELECTION_ITALIC: u16 = 0x01;
const SELECTION_BOLD: u16 = 0x20;
const EMBEDDING_RESTRICTED: u16 = 0x02;

fn table<'a>(font: &'a Font, tag: &[u8; 4]) -> Option<&'a [u8]> {
    let data: &[u8] = font.data();
    let long = |at: usize| Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?) as usize);
    let start = if data.starts_with(b"ttcf") { long(12 + 4 * font.index() as usize)? } else { 0 };
    let count = usize::from(short(data, start + 4)?);
    let record = (0..count).map(|index| start + 12 + 16 * index).find(|&at| data.get(at..at + 4) == Some(tag))?;
    data.get(long(record + 8)?..)?.get(..long(record + 12)?)
}

fn short(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn name(font: &FontInstance, id: u16) -> Option<String> {
    font.ttf().names().into_iter().filter(|name| name.name_id == id).find_map(|name| name.to_string())
}

struct Face {
    family: String,
    bold: bool,
    italic: bool,
}

fn face(instance: &FontInstance) -> Face {
    let font = instance.font();
    let info = font.info();
    let selection = table(font, b"OS/2").and_then(|os2| short(os2, 62));
    Face {
        family: name(instance, NAME_FAMILY).unwrap_or_else(|| info.family.clone()),
        bold: selection.map_or(info.variant.weight >= FontWeight::SEMIBOLD, |bits| bits & SELECTION_BOLD != 0),
        italic: selection.map_or(info.variant.style != FontStyle::Normal, |bits| bits & SELECTION_ITALIC != 0),
    }
}

fn embeddable(instance: &FontInstance) -> Option<Vec<u8>> {
    let font = instance.font();
    let data: &[u8] = font.data();
    let true_type = data.starts_with(&[0, 1, 0, 0]) || data.starts_with(b"true");
    let os2 = table(font, b"OS/2").filter(|os2| os2.len() >= 78)?;
    let adjustment = table(font, b"head")?.get(8..12)?;
    let permissions = short(os2, 8)?;
    if !true_type || font.info().flags.contains(FontFlags::VARIABLE) || permissions & EMBEDDING_RESTRICTED != 0 {
        return None;
    }

    let swapped = |bytes: &[u8]| bytes.as_chunks::<4>().0.iter().flat_map(|long| long.iter().rev().copied()).collect::<Vec<u8>>();
    let mut eot = Vec::with_capacity(data.len() + 256);
    eot.extend([0; 4]);
    eot.extend((data.len() as u32).to_le_bytes());
    eot.extend(0x0002_0001_u32.to_le_bytes());
    eot.extend(0_u32.to_le_bytes());
    eot.extend(&os2[32..42]);
    eot.push(1);
    eot.push(u8::from(short(os2, 62)? & SELECTION_ITALIC != 0));
    eot.extend(u32::from(short(os2, 4)?).to_le_bytes());
    eot.extend(permissions.to_le_bytes());
    eot.extend(0x504C_u16.to_le_bytes());
    eot.extend(swapped(&os2[42..58]));
    eot.extend(swapped(os2.get(78..86).unwrap_or(&[0; 8])));
    eot.extend(swapped(adjustment));
    eot.extend([0; 16]);
    for id in [NAME_FAMILY, NAME_STYLE, NAME_VERSION, NAME_FULL] {
        let text: Vec<u8> = name(instance, id).unwrap_or_default().encode_utf16().flat_map(u16::to_le_bytes).collect();
        eot.extend([0; 2]);
        eot.extend((text.len() as u16).to_le_bytes());
        eot.extend(text);
    }
    eot.extend([0; 4]);
    eot.extend(data);
    let size = (eot.len() as u32).to_le_bytes();
    eot[..4].copy_from_slice(&size);
    Some(eot)
}

fn run(text: &TextItem, at: Point, element: Option<Location>) -> Option<Run> {
    let font = text.font.font();
    let info = font.info();
    let face = face(&text.font);
    let Paint::Solid(colour) = &text.fill else { return None };
    if text.stroke.is_some() || info.flags.contains(FontFlags::MATH) {
        return None;
    }
    let metrics = text.font.metrics();
    let advance = |bytes: std::ops::Range<usize>| -> f64 {
        let inside = text.glyphs.iter().filter(|glyph| bytes.contains(&glyph.range().start));
        inside.map(|glyph| glyph.x_advance.at(text.size).to_pt()).sum()
    };
    let content = text.text.as_str();
    let lead = content.len() - content.trim_start().len();
    let word_end = content[lead..].find(char::is_whitespace).map_or(content.len(), |index| lead + index);
    Some(Run {
        x: at.x.to_pt(),
        baseline: at.y.to_pt(),
        width: text.width().to_pt(),
        ascent: metrics.ascender.at(text.size).to_pt(),
        descent: -metrics.descender.at(text.size).to_pt(),
        size: text.size.to_pt(),
        family: face.family,
        bold: face.bold,
        italic: face.italic,
        colour: colour.to_vec4_u8(),
        text: text.text.to_string(),
        first_word: advance(0..word_end),
        trailing: advance(content.trim_end().len()..content.len()),
        element,
        font: text.font.clone(),
    })
}

fn inside(run: &Run, clip: Option<Rect>) -> bool {
    let slack = Abs::pt(1.0);
    clip.is_none_or(|clip| {
        Abs::pt(run.x) >= clip.min.x - slack
            && Abs::pt(run.x + run.width) <= clip.max.x + slack
            && Abs::pt(run.baseline - run.ascent) >= clip.min.y - slack
            && Abs::pt(run.baseline + run.descent) <= clip.max.y + slack
    })
}

fn narrowed(clip: Option<Rect>, group: &GroupItem, origin: Point) -> Option<Rect> {
    let Some(curve) = &group.clip else { return clip };
    let own = curve.bbox(None);
    let (min, max) = (own.min + origin, own.max + origin);
    Some(match clip {
        Some(outer) => Rect::new(min.max(outer.min), max.min(outer.max)),
        None => Rect::new(min, max),
    })
}

fn split_text(
    frame: &Frame,
    origin: Point,
    clip: Option<Rect>,
    open: &mut Vec<Location>,
    runs: &mut Vec<Run>,
) -> Frame {
    let mut rest = Frame::new(frame.size(), frame.kind());
    for (position, item) in frame.items() {
        let at = origin + *position;
        match item {
            FrameItem::Group(group) if group.transform.is_only_translate() => {
                let inner = at + Point::new(group.transform.tx, group.transform.ty);
                let mut group = group.clone();
                group.frame = split_text(&group.frame, inner, narrowed(clip, &group, inner), open, runs);
                rest.push(*position, FrameItem::Group(group));
            }
            FrameItem::Text(text) => match run(text, at, open.last().copied()).filter(|run| inside(run, clip)) {
                Some(run) => runs.push(run),
                None => rest.push(*position, item.clone()),
            },
            FrameItem::Tag(tag) => {
                match tag {
                    Tag::Start(content, _) => open.extend(content.location()),
                    Tag::End(location, ..) => {
                        if let Some(index) = open.iter().rposition(|opened| opened == location) {
                            open.remove(index);
                        }
                    }
                }
                rest.push(*position, item.clone());
            }
            _ => rest.push(*position, item.clone()),
        }
    }
    rest
}

fn row(run: &Run) -> i64 {
    (run.baseline * 2.0).round() as i64
}

fn continues(line: &[Run], run: &Run) -> bool {
    line.last().is_some_and(|last| {
        let gap = run.x - (last.x + last.width);
        row(last) == row(run) && gap > -1.0 && gap < 0.3 * run.size
    })
}

fn lines(mut runs: Vec<Run>) -> Vec<Vec<Run>> {
    runs.sort_by(|a, b| row(a).cmp(&row(b)).then(a.x.total_cmp(&b.x)));
    let mut lines: Vec<Vec<Run>> = Vec::new();
    for run in runs {
        match lines.last_mut() {
            Some(line) if continues(line, &run) => line.push(run),
            _ => lines.push(vec![run]),
        }
    }
    lines.retain(|line| line.iter().any(|run| !run.text.trim().is_empty()));
    lines
}

#[derive(Clone, Copy, PartialEq)]
enum Align {
    Left,
    Centre,
    Right,
}

struct Paragraph {
    lines: Vec<Vec<Run>>,
    align: Align,
    pitch: f64,
}

fn span(line: &[Run]) -> (f64, f64) {
    let right = line.iter().map(|run| run.x + run.width - run.trailing).fold(line[0].x, f64::max);
    (line[0].x, right)
}

fn size(line: &[Run]) -> f64 {
    line.iter().map(|run| run.size).fold(0.0, f64::max)
}

fn alignment(above: &[Run], below: &[Run]) -> Option<Align> {
    let ((left, right), (next_left, next_right)) = (span(above), span(below));
    if (left - next_left).abs() <= 1.0 {
        Some(Align::Left)
    } else if (left + right - next_left - next_right).abs() <= 2.0 {
        Some(Align::Centre)
    } else if (right - next_right).abs() <= 1.0 {
        Some(Align::Right)
    } else {
        None
    }
}

fn joins(paragraph: &Paragraph, line: &[Run]) -> Option<(Align, f64)> {
    let last = paragraph.lines.last()?;
    let pitch = line[0].baseline - last[0].baseline;
    let align = alignment(last, line)?;
    let same_text = last[0].element == line[0].element
        && last[0].family == line[0].family
        && (size(last) - size(line)).abs() < 0.01;
    let follows = if paragraph.lines.len() == 1 {
        pitch > 0.0 && pitch <= 2.5 * size(line)
    } else {
        align == paragraph.align && (pitch - paragraph.pitch).abs() <= 0.5
    };
    (same_text && follows).then_some((align, pitch))
}

fn paragraphs(lines: Vec<Vec<Run>>) -> Vec<Paragraph> {
    let mut paragraphs: Vec<Paragraph> = Vec::new();
    for line in lines {
        let joined = paragraphs.iter_mut().rev().find_map(|paragraph| Some((joins(paragraph, &line)?, paragraph)));
        match joined {
            Some(((align, pitch), paragraph)) => {
                paragraph.align = align;
                paragraph.pitch = pitch;
                paragraph.lines.push(line);
            }
            None => paragraphs.push(Paragraph { lines: vec![line], align: Align::Left, pitch: 0.0 }),
        }
    }
    paragraphs
}

fn emu(points: f64) -> i64 {
    (points * EMU_PER_POINT).round() as i64
}

fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars().filter(|character| !character.is_control()) {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            other => escaped.push(other),
        }
    }
    escaped
}

fn text_run(run: &Run, text: &str) -> String {
    let [red, green, blue, alpha] = run.colour;
    let family = escape(&run.family);
    format!(
        r#"<a:r><a:rPr sz="{}" b="{}" i="{}" dirty="0"><a:solidFill><a:srgbClr val="{red:02X}{green:02X}{blue:02X}"><a:alpha val="{}"/></a:srgbClr></a:solidFill><a:latin typeface="{family}"/><a:ea typeface="{family}"/><a:cs typeface="{family}"/></a:rPr><a:t>{}</a:t></a:r>"#,
        (run.size * 100.0).round() as i64,
        u8::from(run.bold),
        u8::from(run.italic),
        (f64::from(alpha) / 255.0 * 100_000.0).round() as i64,
        escape(text),
    )
}

fn text_box(id: usize, paragraph: &Paragraph) -> String {
    let lines = &paragraph.lines;
    let first = &lines[0];
    let wrapped = lines.len() > 1;
    let ascent = first.iter().map(|run| run.ascent).fold(0.0, f64::max);
    let descent = first.iter().map(|run| run.descent).fold(0.0, f64::max);
    let left = lines.iter().map(|line| span(line).0).fold(f64::INFINITY, f64::min);
    let right = lines.iter().map(|line| span(line).1).fold(left, f64::max);
    let width = if wrapped { (right - left) * WRAP_SLACK } else { right - left };
    let x = match paragraph.align {
        Align::Left => left,
        Align::Centre => (left + right - width) / 2.0,
        Align::Right => right - width,
    };
    let (top, height, wrap, spacing) = if wrapped {
        let spacing = format!(r#"<a:spcPts val="{}"/>"#, (paragraph.pitch * 100.0).round() as i64);
        (first[0].baseline + descent - paragraph.pitch, paragraph.pitch * lines.len() as f64, "square", spacing)
    } else {
        (first[0].baseline - ascent, ascent + descent, "none", r#"<a:spcPct val="100000"/>"#.to_string())
    };
    let align = match paragraph.align {
        Align::Left => "l",
        Align::Centre => "ctr",
        Align::Right => "r",
    };
    let mut xml = format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="{id}" name="Text {id}"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="{}" y="{}"/><a:ext cx="{}" cy="{}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:noFill/></p:spPr><p:txBody><a:bodyPr wrap="{wrap}" lIns="0" tIns="0" rIns="0" bIns="0" rtlCol="0" anchor="t"><a:noAutofit/></a:bodyPr><a:lstStyle/><a:p><a:pPr marL="0" indent="0" algn="{align}"><a:lnSpc>{spacing}</a:lnSpc><a:spcBef><a:spcPts val="0"/></a:spcBef><a:buNone/></a:pPr>"#,
        emu(x),
        emu(top),
        emu(width),
        emu(height),
    );
    for (index, line) in lines.iter().enumerate() {
        let mut glue = "";
        if let Some(above) = index.checked_sub(1).map(|above| &lines[above]) {
            let (above_left, above_right) = span(above);
            let word_space = 0.25 * line[0].size;
            let open_ended = above.last().is_some_and(|run| !run.text.ends_with(|last: char| last.is_whitespace() || last == '-'));
            if above_right - above_left + word_space + line[0].first_word <= width {
                xml.push_str("<a:br/>");
            } else if open_ended {
                glue = " ";
            }
        }
        for (position, run) in line.iter().enumerate() {
            let text = if position == 0 { format!("{glue}{}", run.text) } else { run.text.clone() };
            xml.push_str(&text_run(run, &text));
        }
    }
    xml.push_str("</a:p></p:txBody></p:sp>");
    xml
}

fn layers(page: &Page) -> (Page, Vec<Run>) {
    let mut runs = Vec::new();
    let mut background = page.clone();
    background.frame = split_text(&page.frame, Point::zero(), None, &mut Vec::new(), &mut runs);
    (background, runs)
}

struct Slide {
    xml: String,
    picture: Vec<u8>,
    fonts: BTreeMap<(String, bool, bool), FontInstance>,
}

fn slide(page: &Page) -> Result<Slide, String> {
    let (background, runs) = layers(page);
    let options = RenderOptions { pixel_per_pt: BACKGROUND_PIXELS_PER_POINT.into(), ..Default::default() };
    let picture = typst_render::render(&background, &options)
        .encode_png()
        .map_err(|error| format!("A slide could not be drawn: {error}"))?;
    let paragraphs = paragraphs(lines(runs));
    let fonts = paragraphs
        .iter()
        .flat_map(|paragraph| paragraph.lines.iter().flatten())
        .map(|run| ((run.family.clone(), run.bold, run.italic), run.font.clone()))
        .collect();

    let mut xml = format!(
        r#"{XML}<p:sld {NAMESPACES}><p:cSld><p:spTree>{EMPTY_TREE}<p:pic><p:nvPicPr><p:cNvPr id="2" name="Background"/><p:cNvPicPr><a:picLocks noChangeAspect="1"/></p:cNvPicPr><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="rId2"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{}" cy="{}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr></p:pic>"#,
        emu(page.frame.width().to_pt()),
        emu(page.frame.height().to_pt()),
    );
    for (index, paragraph) in paragraphs.iter().enumerate() {
        xml.push_str(&text_box(index + 3, paragraph));
    }
    let _ = write!(xml, "</p:spTree></p:cSld>{COLOUR_MAP}</p:sld>");
    Ok(Slide { xml, picture, fonts })
}

fn relationships(targets: &[(&str, String)]) -> String {
    let mut xml = format!(r#"{XML}<Relationships xmlns="{RELATIONSHIPS}">"#);
    for (index, (kind, target)) in targets.iter().enumerate() {
        let _ = write!(xml, r#"<Relationship Id="rId{}" Type="{OFFICE}/{kind}" Target="{target}"/>"#, index + 1);
    }
    xml.push_str("</Relationships>");
    xml
}

fn content_types(slides: usize) -> String {
    let mut xml = format!(
        r#"{XML}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Default Extension="fntdata" ContentType="application/x-fontdata"/><Override PartName="/ppt/presentation.xml" ContentType="{PRESENTATION}.presentation.main+xml"/><Override PartName="/ppt/slideMasters/slideMaster1.xml" ContentType="{PRESENTATION}.slideMaster+xml"/><Override PartName="/ppt/slideLayouts/slideLayout1.xml" ContentType="{PRESENTATION}.slideLayout+xml"/><Override PartName="/ppt/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/>"#
    );
    for number in 1..=slides {
        let _ = write!(xml, r#"<Override PartName="/ppt/slides/slide{number}.xml" ContentType="{PRESENTATION}.slide+xml"/>"#);
    }
    xml.push_str("</Types>");
    xml
}

type Embedded = BTreeMap<String, [Option<usize>; 4]>;

fn presentation(slides: usize, width: i64, height: i64, embedded: &Embedded) -> String {
    let mut xml = format!(
        r#"{XML}<p:presentation {NAMESPACES} embedTrueTypeFonts="1"><p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst><p:sldIdLst>"#
    );
    for index in 0..slides {
        let _ = write!(xml, r#"<p:sldId id="{}" r:id="rId{}"/>"#, 256 + index, 3 + index);
    }
    let _ = write!(xml, r#"</p:sldIdLst><p:sldSz cx="{width}" cy="{height}"/><p:notesSz cx="6858000" cy="9144000"/>"#);
    if !embedded.is_empty() {
        xml.push_str("<p:embeddedFontLst>");
        for (family, styles) in embedded {
            let _ = write!(xml, r#"<p:embeddedFont><p:font typeface="{}"/>"#, escape(family));
            for (style, link) in ["regular", "bold", "italic", "boldItalic"].iter().zip(styles) {
                if let Some(link) = link {
                    let _ = write!(xml, r#"<p:{style} r:id="rId{link}"/>"#);
                }
            }
            xml.push_str("</p:embeddedFont>");
        }
        xml.push_str("</p:embeddedFontLst>");
    }
    xml.push_str("</p:presentation>");
    xml
}

fn master() -> String {
    format!(
        r#"{XML}<p:sldMaster {NAMESPACES}><p:cSld><p:bg><p:bgRef idx="1001"><a:schemeClr val="bg1"/></p:bgRef></p:bg><p:spTree>{EMPTY_TREE}</p:spTree></p:cSld><p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/><p:sldLayoutIdLst><p:sldLayoutId id="2147483649" r:id="rId1"/></p:sldLayoutIdLst></p:sldMaster>"#
    )
}

fn layout() -> String {
    format!(
        r#"{XML}<p:sldLayout {NAMESPACES} type="blank" preserve="1"><p:cSld name="Blank"><p:spTree>{EMPTY_TREE}</p:spTree></p:cSld>{COLOUR_MAP}</p:sldLayout>"#
    )
}

fn office_theme() -> String {
    let colours: String = [
        ("dk1", "000000"),
        ("lt1", "FFFFFF"),
        ("dk2", "404040"),
        ("lt2", "E6E6E6"),
        ("accent1", "595959"),
        ("accent2", "737373"),
        ("accent3", "8C8C8C"),
        ("accent4", "A6A6A6"),
        ("accent5", "BFBFBF"),
        ("accent6", "D9D9D9"),
        ("hlink", "0563C1"),
        ("folHlink", "954F72"),
    ]
    .iter()
    .map(|(name, value)| format!(r#"<a:{name}><a:srgbClr val="{value}"/></a:{name}>"#))
    .collect();
    let font = r#"<a:latin typeface="Arial"/><a:ea typeface=""/><a:cs typeface=""/>"#;
    let fill = r#"<a:solidFill><a:schemeClr val="phClr"/></a:solidFill>"#.repeat(3);
    let line = r#"<a:ln w="9525"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln>"#.repeat(3);
    let effect = "<a:effectStyle><a:effectLst/></a:effectStyle>".repeat(3);
    format!(
        r#"{XML}<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="znote"><a:themeElements><a:clrScheme name="znote">{colours}</a:clrScheme><a:fontScheme name="znote"><a:majorFont>{font}</a:majorFont><a:minorFont>{font}</a:minorFont></a:fontScheme><a:fmtScheme name="znote"><a:fillStyleLst>{fill}</a:fillStyleLst><a:lnStyleLst>{line}</a:lnStyleLst><a:effectStyleLst>{effect}</a:effectStyleLst><a:bgFillStyleLst>{fill}</a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>"#
    )
}

pub fn pdf(document: &PagedDocument) -> Result<Vec<u8>, String> {
    typst_pdf::pdf(document, &PdfOptions::default()).map_err(|errors| {
        let reason = errors.first().map(|error| error.message.to_string()).unwrap_or_default();
        format!("The PDF could not be made: {reason}")
    })
}

pub fn pptx(document: &PagedDocument) -> Result<(Vec<u8>, BTreeSet<String>), String> {
    let failed = |error: zip::result::ZipError| format!("The presentation could not be made: {error}");
    let pages = document.pages();
    let first = pages.first().ok_or("This page has nothing to export.")?;
    let side = |points: f64| emu(points).clamp(*SLIDE_EMU.start(), *SLIDE_EMU.end());

    let mut parts = vec![
        ("[Content_Types].xml".to_string(), content_types(pages.len())),
        ("_rels/.rels".into(), relationships(&[("officeDocument", "ppt/presentation.xml".into())])),
        ("ppt/slideMasters/slideMaster1.xml".into(), master()),
        (
            "ppt/slideMasters/_rels/slideMaster1.xml.rels".into(),
            relationships(&[
                ("slideLayout", "../slideLayouts/slideLayout1.xml".into()),
                ("theme", "../theme/theme1.xml".into()),
            ]),
        ),
        ("ppt/slideLayouts/slideLayout1.xml".into(), layout()),
        (
            "ppt/slideLayouts/_rels/slideLayout1.xml.rels".into(),
            relationships(&[("slideMaster", "../slideMasters/slideMaster1.xml".into())]),
        ),
        ("ppt/theme/theme1.xml".into(), office_theme()),
    ];
    let mut links = vec![("slideMaster", "slideMasters/slideMaster1.xml".to_string()), ("theme", "theme/theme1.xml".into())];
    let mut fonts = BTreeMap::new();
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);

    for (index, page) in pages.iter().enumerate() {
        let number = index + 1;
        let slide = slide(page)?;
        fonts.extend(slide.fonts);
        links.push(("slide", format!("slides/slide{number}.xml")));
        parts.push((format!("ppt/slides/slide{number}.xml"), slide.xml));
        parts.push((
            format!("ppt/slides/_rels/slide{number}.xml.rels"),
            relationships(&[
                ("slideLayout", "../slideLayouts/slideLayout1.xml".into()),
                ("image", format!("../media/image{number}.png")),
            ]),
        ));
        writer.start_file(format!("ppt/media/image{number}.png"), stored).map_err(failed)?;
        writer.write_all(&slide.picture).map_err(|error| failed(error.into()))?;
    }

    let mut embedded = Embedded::new();
    let mut referenced = BTreeSet::new();
    for ((family, bold, italic), font) in fonts {
        let Some(data) = embeddable(&font) else {
            referenced.insert(font.font().info().family.clone());
            continue;
        };
        let number = links.len() + 1;
        links.push(("font", format!("fonts/font{number}.fntdata")));
        embedded.entry(family).or_default()[usize::from(bold) + 2 * usize::from(italic)] = Some(number);
        writer.start_file(format!("ppt/fonts/font{number}.fntdata"), SimpleFileOptions::default()).map_err(failed)?;
        writer.write_all(&data).map_err(|error| failed(error.into()))?;
    }
    let (width, height) = (side(first.frame.width().to_pt()), side(first.frame.height().to_pt()));
    parts.push(("ppt/presentation.xml".into(), presentation(pages.len(), width, height, &embedded)));
    parts.push(("ppt/_rels/presentation.xml.rels".into(), relationships(&links)));

    for (name, xml) in parts {
        writer.start_file(name, SimpleFileOptions::default()).map_err(failed)?;
        writer.write_all(xml.as_bytes()).map_err(|error| failed(error.into()))?;
    }
    Ok((writer.finish().map_err(failed)?.into_inner(), referenced))
}

fn not_installed(used: BTreeSet<String>, installed: &HashSet<String>) -> Vec<String> {
    used.into_iter().filter(|family| !installed.contains(&family.to_lowercase())).collect()
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Pdf,
    Pptx,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Exported {
    path: String,
    missing_fonts: Vec<String>,
}

#[derive(Default)]
pub struct Exports {
    last: Mutex<Option<PathBuf>>,
}

impl Exports {
    fn last(&self) -> Result<PathBuf, String> {
        let last = self.last.lock().map_err(|error| error.to_string())?;
        last.clone().ok_or_else(|| "Nothing has been exported yet.".to_string())
    }
}

#[tauri::command(async)]
pub fn export_page(
    app: AppHandle,
    exports: State<Exports>,
    page: String,
    text: String,
    format: Format,
) -> Result<Option<Exported>, String> {
    let document = compile::document(&app, &page, text)?;
    let (label, extension) = match format {
        Format::Pdf => ("PDF", "pdf"),
        Format::Pptx => ("PowerPoint", "pptx"),
    };
    let name = page.rsplit('/').next().unwrap_or(&page);
    let dialog = app.dialog().file().set_file_name(format!("{name}.{extension}")).add_filter(label, &[extension]);
    let Some(target) = dialog.blocking_save_file() else {
        return Ok(None);
    };
    let mut path = target.into_path().map_err(|error| error.to_string())?;
    if path.extension().is_none_or(|given| !given.eq_ignore_ascii_case(extension)) {
        path.as_mut_os_string().push(format!(".{extension}"));
    }

    let (bytes, missing_fonts) = match format {
        Format::Pdf => (pdf(&document)?, Vec::new()),
        Format::Pptx => {
            let (bytes, used) = pptx(&document)?;
            let installed = fonts::system().map(|(_, info)| info.family.to_lowercase()).collect();
            (bytes, not_installed(used, &installed))
        }
    };
    std::fs::write(&path, bytes).map_err(|error| format!("The file could not be saved: {error}"))?;
    *exports.last.lock().map_err(|error| error.to_string())? = Some(path.clone());
    Ok(Some(Exported { path: path.to_string_lossy().into_owned(), missing_fonts }))
}

#[tauri::command]
pub fn open_export(app: AppHandle, exports: State<Exports>) -> Result<(), String> {
    let path = exports.last()?.to_string_lossy().into_owned();
    app.opener().open_path(path, None::<&str>).map_err(|error| format!("The file could not be opened: {error}"))
}

#[tauri::command]
pub fn show_export(app: AppHandle, exports: State<Exports>) -> Result<(), String> {
    app.opener()
        .reveal_item_in_dir(exports.last()?)
        .map_err(|error| format!("The folder could not be opened: {error}"))
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::sync::{Arc, LazyLock};

    use typst_kit::fonts::FontStore;
    use zip::ZipArchive;

    use super::*;
    use crate::compile::{PageWorld, font_store};

    static FONTS: LazyLock<Arc<FontStore>> =
        LazyLock::new(|| Arc::new(font_store(&std::env::temp_dir().join("no-themes"))));

    fn world() -> PageWorld {
        let folder = std::env::temp_dir();
        PageWorld::new(&folder, "main.typ", &folder.join("no-themes"), FONTS.clone(), |_| {}).unwrap()
    }

    fn document(text: &str) -> PagedDocument {
        world().document(text.into()).unwrap()
    }

    fn part(archive: &mut ZipArchive<Cursor<Vec<u8>>>, name: &str) -> String {
        let mut text = String::new();
        archive.by_name(name).unwrap().read_to_string(&mut text).unwrap();
        text
    }

    fn blank(page: &Page) -> bool {
        let picture = typst_render::render(page, &RenderOptions::default());
        picture.pixels().iter().all(|pixel| *pixel == picture.pixels()[0])
    }

    #[test]
    fn a_pdf_is_a_pdf() {
        assert!(pdf(&document("Hello")).unwrap().starts_with(b"%PDF"));
    }

    #[test]
    fn a_page_with_problems_is_not_exported() {
        let mut world = world();
        assert!(world.document("#nope".into()).unwrap_err().contains("Fix the problems"));
    }

    #[test]
    fn each_page_becomes_a_slide_with_editable_text_over_a_picture() {
        let source = "#set page(width: 400pt, height: 225pt)\n= Known heading\n#\"A & B < C\" *bold* _slanted_\n#pagebreak()\nTwo";
        let document = document(source);
        let (bytes, fonts) = pptx(&document).unwrap();
        let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
        let names: Vec<String> = (0..archive.len()).map(|index| archive.by_index(index).unwrap().name().unwrap().to_string()).collect();

        let count = |folder: &str| names.iter().filter(|name| name.starts_with(folder) && !name.contains("_rels")).count();
        assert_eq!(count("ppt/slides/"), 2);
        assert_eq!(count("ppt/media/"), 2);
        assert!(part(&mut archive, "ppt/presentation.xml").contains(r#"<p:sldSz cx="5080000" cy="2857500"/>"#));
        assert!(part(&mut archive, "ppt/_rels/presentation.xml.rels").contains("slides/slide2.xml"));
        assert!(part(&mut archive, "[Content_Types].xml").contains("/ppt/slides/slide2.xml"));

        let slide = part(&mut archive, "ppt/slides/slide1.xml");
        assert!(slide.contains("<a:t>Known heading</a:t>"), "{slide}");
        assert!(slide.contains("A &amp; B &lt; C"), "{slide}");
        assert!(slide.contains(r#"b="1" i="0""#) && slide.contains(r#"b="0" i="1""#), "{slide}");
        assert!(slide.contains(r#"<a:latin typeface="Libertinus Serif"/>"#), "{slide}");
        assert_eq!(slide.matches("<p:sp>").count(), 2);
        assert!(part(&mut archive, "ppt/slides/slide2.xml").contains("<a:t>Two</a:t>"));
        assert!(fonts.contains("Libertinus Serif"));

        assert!(!blank(&document.pages()[0]));
        assert!(blank(&layers(&document.pages()[0]).0));
    }

    #[test]
    fn math_and_transformed_text_stay_in_the_picture() {
        let document = document("$ x^2 + y $\n#rotate(20deg)[Tilted]");
        let page = &document.pages()[0];
        let slide = slide(page).unwrap();
        assert!(!slide.xml.contains("<p:sp>"), "{}", slide.xml);
        assert!(slide.fonts.is_empty());
        assert!(!blank(&layers(page).0));
    }

    #[test]
    fn text_in_a_clipped_box_is_editable_unless_the_clip_cuts_it() {
        let document = document(
            "#block(width: 200pt, inset: 10pt, radius: 8pt, fill: aqua, clip: true)[Inside the panel]\n#block(width: 30pt, height: 8pt, clip: true)[Cut off by the clip]",
        );
        let slide = slide(&document.pages()[0]).unwrap();
        assert!(slide.xml.contains("<a:t>Inside the panel</a:t>"), "{}", slide.xml);
        assert!(!slide.xml.contains("Cut off"), "{}", slide.xml);
    }

    #[test]
    fn lines_of_one_paragraph_share_a_box_that_wraps() {
        let narrow = "#set page(width: 200pt, height: 400pt)\n";
        let boxes = |source: &str| slide(&document(&format!("{narrow}{source}")).pages()[0]).unwrap().xml;

        let heading = boxes("= A heading that is long enough to wrap");
        assert_eq!(heading.matches("<p:sp>").count(), 1, "{heading}");
        assert!(heading.contains(r#"wrap="square""#) && heading.contains("<a:spcPts"), "{heading}");
        assert!(heading.contains("long enough to wrap</a:t>") && !heading.contains("<a:br/>"), "{heading}");

        let untagged = boxes("#block(width: 100pt, text(20pt)[Block title wrapping here])");
        assert_eq!(untagged.matches("<p:sp>").count(), 1, "{untagged}");

        let centred = boxes("#align(center)[A centred paragraph that is long enough to wrap over lines]");
        assert_eq!(centred.matches("<p:sp>").count(), 1, "{centred}");
        assert!(centred.contains(r#"algn="ctr""#), "{centred}");

        let forced = boxes("One \\ Two is longer");
        assert_eq!(forced.matches("<p:sp>").count(), 1, "{forced}");
        assert!(forced.contains("<a:br/>"), "{forced}");

        let glued = boxes("#block(width: 150pt)[Building Excellence, \\ Timeless Quality.]");
        assert!(glued.contains("<a:br/>") || glued.contains("<a:t> Timeless Quality.</a:t>"), "{glued}");

        let single = boxes("Short");
        assert!(single.contains(r#"wrap="none""#), "{single}");

        let separate = boxes("First paragraph.\n\nSecond paragraph.\n- Item one\n- Item two");
        assert_eq!(separate.matches("<p:sp>").count(), 6, "{separate}");
    }

    #[test]
    fn true_type_fonts_travel_inside_the_presentation() {
        let document = document("Serif text and `mono text`");
        let (bytes, referenced) = pptx(&document).unwrap();
        let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();

        let links = part(&mut archive, "ppt/_rels/presentation.xml.rels");
        assert!(links.contains(r#"relationships/font" Target="fonts/font4.fntdata""#), "{links}");
        let presentation = part(&mut archive, "ppt/presentation.xml");
        let listed = r#"<p:embeddedFontLst><p:embeddedFont><p:font typeface="DejaVu Sans Mono"/><p:regular r:id="rId4"/></p:embeddedFont></p:embeddedFontLst>"#;
        assert!(presentation.contains(listed), "{presentation}");
        assert!(part(&mut archive, "[Content_Types].xml").contains("application/x-fontdata"));

        let mut font = Vec::new();
        archive.by_name("ppt/fonts/font4.fntdata").unwrap().read_to_end(&mut font).unwrap();
        let size = |at: usize| u32::from_le_bytes(font[at..at + 4].try_into().unwrap()) as usize;
        assert_eq!(size(0), font.len());
        assert_eq!(&font[34..36], &[0x4C, 0x50]);
        assert!(font[font.len() - size(4)..].starts_with(&[0, 1, 0, 0]));

        assert_eq!(referenced, BTreeSet::from(["Libertinus Serif".to_string()]));
    }

    #[test]
    fn fonts_the_machine_does_not_have_are_listed() {
        let used = BTreeSet::from(["Arial".to_string(), "Libertinus Serif".to_string()]);
        let installed = HashSet::from(["arial".to_string()]);
        assert_eq!(not_installed(used, &installed), ["Libertinus Serif"]);
    }
}
