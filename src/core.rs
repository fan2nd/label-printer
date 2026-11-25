use crate::symbol;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write,
};

use crate::stock::{dots_at, Model, DPI};
pub use crate::stock::{MAX_MEDIA_LENGTH_DOTS, P2_HEAD_DOTS};
pub const MAX_MEDIA_WIDTH_DOTS: u32 = P2_HEAD_DOTS;
pub const MAX_BATCH: usize = 500;
pub const MAX_FIELDS: usize = 24;
const MAX_VALUE: usize = 4096;
pub type Row = BTreeMap<String, String>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Text,
    Qr,
    Barcode,
    Image,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    #[serde(default = "uuid::Uuid::new_v4")]
    pub id: uuid::Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<crate::picture::Picture>,
    pub kind: Kind,
    pub text: String,
    pub x: i64,
    pub y: i64,
    pub w: u32,
    pub h: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module_size: Option<u32>,
}

impl Field {
    pub fn text_size(&self) -> u32 {
        self.font_size
            .unwrap_or_else(|| (self.h.saturating_mul(4) / 5).clamp(1, 32))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Media {
    #[serde(default = "default_dpi")]
    pub dpi: u32,
    #[serde(default)]
    pub cut: crate::stock::Cut,
    pub name: String,
    pub width_mm: u16,
    pub length_mm: u16,
    pub printable_width_dots: u32,
    pub printable_length_dots: u32,
}

fn default_dpi() -> u32 {
    DPI
}

pub fn builtin_media() -> &'static [(Model, Media, &'static [&'static str])] {
    static MEDIA: std::sync::OnceLock<Vec<(Model, Media, &'static [&'static str])>> =
        std::sync::OnceLock::new();
    MEDIA.get_or_init(|| include!(concat!(env!("OUT_DIR"), "/labels.rs")))
}

impl Default for Media {
    fn default() -> Self {
        Self::for_model(Model::N1)
    }
}

impl Media {
    pub fn for_model(model: Model) -> Self {
        builtin_media()
            .iter()
            .find(|(m, _, _)| *m == model)
            .expect("机型均有构建时验证的默认规格")
            .1
            .clone()
    }

    #[cfg(test)]
    pub fn p2() -> Self {
        Self {
            dpi: DPI,
            cut: Default::default(),
            name: "P2 test".into(),
            width_mm: 48,
            length_mm: 50,
            printable_width_dots: 384,
            printable_length_dots: 400,
        }
    }

    #[cfg(test)]
    pub fn p2_40x30() -> Self {
        Self {
            dpi: DPI,
            cut: Default::default(),
            name: "P2 small test".into(),
            width_mm: 40,
            length_mm: 30,
            printable_width_dots: 320,
            printable_length_dots: 240,
        }
    }

    pub fn supports_p2(&self) -> bool {
        self.dpi == DPI
            && self.width_mm <= 57
            && self.printable_width_dots <= P2_HEAD_DOTS
            && self.validate().is_ok()
    }

    pub fn canvas_size(&self) -> (u32, u32) {
        (self.printable_length_dots, self.printable_width_dots)
    }

    pub fn effective_cut(&self) -> &crate::stock::Cut {
        if self.cut == crate::stock::Cut::default() {
            if let Some((_, stock, _)) = builtin_media().iter().find(|(_, m, _)| {
                m.name == self.name
                    && m.width_mm == self.width_mm
                    && m.length_mm == self.length_mm
                    && m.canvas_size() == self.canvas_size()
            }) {
                return &stock.cut;
            }
        }
        &self.cut
    }

    pub fn is_known_n1(&self) -> bool {
        builtin_media().iter().any(|(model, stock, _)| {
            *model == Model::N1
                && self.dpi == stock.dpi
                && stock.width_mm == self.width_mm
                && stock.length_mm == self.length_mm
                && stock.canvas_size() == self.canvas_size()
        })
    }

    pub fn validate(&self) -> Result<(), String> {
        let (along, across) = self.canvas_size();
        if self.width_mm == 0 || self.length_mm == 0 {
            return Err("标签宽度和长度必须大于 0 mm。".into());
        }
        if !(1..=MAX_MEDIA_WIDTH_DOTS).contains(&across)
            || !(1..=MAX_MEDIA_LENGTH_DOTS).contains(&along)
        {
            return Err(format!("可印尺寸须为横向 1～{MAX_MEDIA_WIDTH_DOTS} 点、走纸方向 1～{MAX_MEDIA_LENGTH_DOTS} 点。"));
        }
        self.effective_cut().validate(along, across)?;
        if !matches!(self.dpi, 180 | 203) {
            return Err("标签规格需要 180 或 203 dpi。".into());
        }
        if across > dots_at(self.width_mm, self.dpi) || along > dots_at(self.length_mm, self.dpi) {
            return Err(format!("可印尺寸不能超过纸面尺寸（{} dpi）。", self.dpi));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Template {
    pub name: String,
    #[serde(default = "legacy_media")]
    pub media: Media,
    pub elements: Vec<Field>,
}

// Local templates from before media selection used a fixed 400 × 96 canvas.
fn legacy_media() -> Media {
    Media {
        name: "N1 · 14 × 50 mm".into(),
        width_mm: 14,
        length_mm: 50,
        printable_width_dots: 96,
        printable_length_dots: 400,
        dpi: DPI,
        cut: Default::default(),
    }
}

impl Default for Template {
    fn default() -> Self {
        Self {
            name: "物料标签".into(),
            media: Media::default(),
            elements: Vec::new(),
        }
    }
}

#[cfg(test)]
impl Template {
    pub(crate) fn example() -> Self {
        Self {
            name: "物料标签".into(),
            media: Media::default(),
            elements: vec![
                Field {
                    id: uuid::Uuid::new_v4(),
                    picture: None,
                    kind: Kind::Text,
                    font_size: Some(22),
                    module_size: None,
                    text: "物料 {seq}".into(),
                    x: 8,
                    y: 7,
                    w: 292,
                    h: 28,
                },
                Field {
                    id: uuid::Uuid::new_v4(),
                    picture: None,
                    kind: Kind::Barcode,
                    font_size: None,
                    module_size: Some(2),
                    text: "P-{seq}".into(),
                    x: 8,
                    y: 42,
                    w: 292,
                    h: 43,
                },
                Field {
                    id: uuid::Uuid::new_v4(),
                    picture: None,
                    kind: Kind::Qr,
                    font_size: None,
                    module_size: Some(4),
                    text: "P-{seq}".into(),
                    x: 309,
                    y: 5,
                    w: 86,
                    h: 86,
                },
            ],
        }
    }
}

#[cfg(test)]
impl Template {
    pub fn n1_example(media: Media) -> Self {
        let compact = media.is_known_n1() && media.length_mm == 25;
        let mut template = Self {
            media,
            ..Self::example()
        };
        if compact {
            for item in &mut template.elements {
                item.x = item.x.max(10);
                match item.kind {
                    Kind::Qr => {
                        item.x = 150;
                        item.module_size = Some(2);
                    }
                    Kind::Barcode => {
                        item.y = 55;
                        item.h = 34;
                        item.text = "{seq}".into();
                    }
                    _ => {}
                }
            }
        }
        template
    }

    pub fn p2_example(media: Media) -> Self {
        let compact = media.printable_length_dots < 400 && media.printable_width_dots >= 180;
        let mut draft = Self {
            media,
            ..Self::example()
        };
        if compact {
            for item in &mut draft.elements {
                match item.kind {
                    Kind::Text => item.w = 220,
                    Kind::Image => (),
                    Kind::Barcode => {
                        item.w = 220;
                        item.text = "{seq}".into();
                    }
                    Kind::Qr => {
                        item.x = 8;
                        item.y = 96;
                    }
                }
            }
        }
        draft
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sequence {
    pub start: i64,
    pub step: i64,
    pub pad: usize,
}

impl Default for Sequence {
    fn default() -> Self {
        Self {
            start: 1,
            step: 1,
            pad: 4,
        }
    }
}

impl Sequence {
    pub fn at(self, index: usize) -> Result<String, String> {
        if self.pad > 18 {
            return Err("补零位数应为 0～18。".into());
        }
        if self.step == 0 {
            return Err("递增步长不能为 0。".into());
        }
        let value = i128::from(self.start)
            + i128::from(self.step) * i128::try_from(index).map_err(|_| "序号位置超出范围。")?;
        let value = i64::try_from(value).map_err(|_| "序号超出范围。")?;
        let sign = if value < 0 { "-" } else { "" };
        Ok(format!(
            "{sign}{:0width$}",
            value.unsigned_abs(),
            width = self.pad
        ))
    }
}

pub fn parse_csv(input: &str) -> Result<Vec<Row>, String> {
    if input.len() > 2 * 1024 * 1024 {
        return Err("CSV 文件不得超过 2 MB。".into());
    }
    if input.trim().is_empty() {
        return Err("CSV 为空。".into());
    }
    let mut reader =
        csv::ReaderBuilder::new().from_reader(input.trim_start_matches('\u{feff}').as_bytes());
    let headers = reader
        .headers()
        .map_err(|e| format!("CSV 表头错误：{e}"))?
        .clone();
    let headers: Vec<_> = headers.iter().map(str::trim).collect();
    if headers.is_empty() || headers.len() > 64 {
        return Err("CSV 需要 1～64 列。".into());
    }
    let mut seen = BTreeSet::new();
    for h in &headers {
        if h.is_empty()
            || *h == "seq"
            || h.len() > 128
            || h.chars().any(|c| c.is_control() || c == '{' || c == '}')
        {
            return Err("CSV 列名不得为空、包含大括号或使用保留字段 seq。".into());
        }
        if !seen.insert(*h) {
            return Err(format!("CSV 列名重复：{h}"));
        }
    }
    let mut rows = Vec::new();
    for (i, record) in reader.records().enumerate() {
        if i == MAX_BATCH {
            return Err(format!("单批最多 {MAX_BATCH} 条。"));
        }
        let record = record.map_err(|e| format!("CSV 第 {} 条数据无法读取：{e}", i + 1))?;
        if record.iter().any(|s| s.len() > MAX_VALUE) {
            return Err(format!("CSV 第 {} 条数据存在过长字段。", i + 1));
        }
        rows.push(
            headers
                .iter()
                .copied()
                .zip(record.iter())
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect(),
        );
    }
    if rows.is_empty() {
        return Err("CSV 没有数据行。".into());
    }
    Ok(rows)
}

pub fn materialize(source: &str, row: &Row, seq: &str) -> Result<String, String> {
    if source.len() > MAX_VALUE {
        return Err("内容太长。".into());
    }
    let mut out = String::new();
    let mut chars = source.char_indices().peekable();
    while let Some((_, ch)) = chars.next() {
        match ch {
            '{' if chars.peek().is_some_and(|(_, next)| *next == '{') => {
                chars.next();
                out.push('{');
            }
            '}' if chars.peek().is_some_and(|(_, next)| *next == '}') => {
                chars.next();
                out.push('}');
            }
            '{' => {
                let mut field = String::new();
                loop {
                    match chars.next() {
                        Some((_, '}')) if !field.is_empty() => break,
                        Some((_, c)) if c != '{' && c != '}' => field.push(c),
                        _ => return Err("字段占位符格式错误；字面大括号用 {{ 或 }}。".into()),
                    }
                }
                if field == "seq" {
                    out.push_str(seq);
                } else {
                    out.push_str(
                        row.get(&field)
                            .ok_or_else(|| format!("缺少 CSV 字段：{field}"))?,
                    );
                }
            }
            '}' => return Err("字段占位符格式错误；字面大括号用 {{ 或 }}。".into()),
            c => out.push(c),
        }
        if out.len() > MAX_VALUE {
            return Err("替换后的内容太长。".into());
        }
    }
    Ok(out)
}

/// Validates editable templates without requiring a finished printable layout.
pub(crate) fn validate_template(template: &Template) -> Result<(), String> {
    template.media.validate()?;
    if template.elements.len() > MAX_FIELDS {
        return Err(format!("模板最多可包含 {MAX_FIELDS} 个元素。"));
    }
    for (idx, field) in template.elements.iter().enumerate() {
        if field.w == 0
            || field.h == 0
            || field.x.checked_add(i64::from(field.w)).is_none()
            || field.y.checked_add(i64::from(field.h)).is_none()
        {
            return Err(format!("元素 {} 的尺寸为 0 或坐标超出范围。", idx + 1));
        }
        if [field.font_size, field.module_size]
            .into_iter()
            .flatten()
            .any(|n| !(1..=MAX_MEDIA_WIDTH_DOTS).contains(&n))
        {
            return Err(format!(
                "元素 {} 的字号或模块尺寸须在 1～{MAX_MEDIA_WIDTH_DOTS} 点之间。",
                idx + 1
            ));
        }
        match (&field.kind, &field.picture) {
            (Kind::Image, Some(picture)) => picture.validate()?,
            (Kind::Image, None) => return Err(format!("元素 {} 缺少图片数据。", idx + 1)),
            (_, Some(_)) => return Err(format!("元素 {} 的类型与图片数据不匹配。", idx + 1)),
            _ => (),
        }
        if field.text.len() > MAX_VALUE {
            return Err(format!("元素 {} 的内容超过 {MAX_VALUE} 字节。", idx + 1));
        }
    }
    Ok(())
}

struct TextInk {
    tree: resvg::usvg::Tree,
    bounds: resvg::usvg::NonZeroRect,
    size: (u32, u32),
    escaped: String,
}

impl TextInk {
    fn shape(content: &str, font_size: u32) -> Result<Self, String> {
        use resvg::usvg::{Node, Options, Tree};
        if content
            .chars()
            .any(|c| c.is_control() || matches!(c, '\u{fffe}' | '\u{ffff}'))
        {
            return Err("文字只支持单行可打印字符。".into());
        }
        let escaped = xml_text(content);
        let source = format!("<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'><text font-family='sans-serif' font-size='{font_size}' xml:space='preserve'>{escaped}</text></svg>");
        let options = Options {
            fontdb: crate::raster::fonts(),
            ..Default::default()
        };
        let tree = Tree::from_str(&source, &options).map_err(|_| "文字排版失败。")?;
        let Some(Node::Text(text)) = tree.root().children().first() else {
            return Err("文字没有可显示字形，请检查内容和系统字体。".into());
        };
        let bounds = text
            .flattened()
            .bounding_box()
            .to_non_zero_rect()
            .ok_or("文字没有可显示字形，请检查内容和系统字体。")?;
        let width = f64::from(bounds.width()).ceil();
        let height = f64::from(bounds.height()).ceil();
        if width > f64::from(u32::MAX) || height > f64::from(u32::MAX) {
            return Err("文字尺寸超出范围。".into());
        }
        Ok(Self {
            tree,
            bounds,
            size: (width as u32, height as u32),
            escaped,
        })
    }
}

#[cfg(test)]
fn text_dimensions(content: &str, font_size: u32) -> Result<(u32, u32), String> {
    Ok(TextInk::shape(content, font_size)?.size)
}

fn xml_text(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

struct Artwork {
    bounds: Field,
    image: String,
}

impl Artwork {
    fn new(field: &Field, row: &Row, serial: &str, index: usize) -> Result<Self, String> {
        if field.kind == Kind::Image {
            let picture = field.picture.as_ref().ok_or("缺少图片数据。")?;
            return Ok(Self {
                bounds: field.clone(),
                image: picture.svg(field.w, field.h)?,
            });
        }
        let content = materialize(&field.text, row, serial)?;
        if content.trim().is_empty() {
            return Err("内容为空。".into());
        }
        let mut image = String::new();
        let bounds = match field.kind {
            Kind::Image => unreachable!(),
            Kind::Text => {
                let ink = TextInk::shape(&content, field.text_size())?;
                let mut bounds = field.clone();
                (bounds.w, bounds.h) = ink.size;
                bounds.font_size = Some(field.text_size());
                let outlined = ink.tree.to_string(&resvg::usvg::WriteOptions {
                    id_prefix: Some(format!("glyph{index}-")),
                    indent: resvg::usvg::Indent::None,
                    ..Default::default()
                });
                // Keep the shaped glyph paths so WebView, SVG and printer use identical ink.
                let (_, body) = outlined.split_once('>').ok_or("无法生成文字轮廓。")?;
                write!(image, "<svg xmlns='http://www.w3.org/2000/svg' xmlns:xlink='http://www.w3.org/1999/xlink' width='{w}' height='{h}' viewBox='{left} {top} {w} {h}'><title>{title}</title>{body}",
                    w=bounds.w,h=bounds.h,left=ink.bounds.x(),top=ink.bounds.y(),title=ink.escaped).unwrap();
                bounds
            }
            Kind::Qr | Kind::Barcode => {
                let symbol = symbol::encode(field.kind, &content)?;
                let bounds = symbol.layout(field)?;
                let local = Field {
                    x: 0,
                    y: 0,
                    ..bounds.clone()
                };
                write!(
                    image,
                    "<svg xmlns='http://www.w3.org/2000/svg' width='{}' height='{}'>",
                    local.w, local.h
                )
                .unwrap();
                symbol.draw(&mut image, &local);
                image.push_str("</svg>");
                bounds
            }
        };
        Ok(Self { bounds, image })
    }
}

pub fn editor_art(
    field: &Field,
    row: &Row,
    serial: &str,
    index: usize,
) -> Result<(Field, String), String> {
    let art = Artwork::new(field, row, serial, index)?;
    Ok((art.bounds, art.image))
}

#[cfg(test)]
pub fn render_editor_item(field: &Field, row: &Row, serial: &str) -> Result<String, String> {
    Ok(editor_art(field, row, serial, 0)?.1)
}

pub fn check_overlap(items: &[(Field, Option<String>)]) -> Result<(), String> {
    // Only rasterize when a symbol shares space with another element. White
    // space between glyphs or in a loose legacy box introduces no printed dots.
    let art: Vec<_> = items
        .iter()
        .enumerate()
        .filter_map(|(idx, (f, svg))| {
            if f.w > MAX_MEDIA_LENGTH_DOTS || f.h > MAX_MEDIA_WIDTH_DOTS {
                return None;
            }
            svg.as_deref().map(|s| (f, s, idx))
        })
        .collect();
    let mut masks = vec![None; art.len()];
    for (i, target) in art
        .iter()
        .enumerate()
        .filter(|(_, a)| matches!(a.0.kind, Kind::Qr | Kind::Barcode))
    {
        let a = target.0;
        for (j, other) in art.iter().enumerate() {
            if i == j {
                continue;
            }
            let b = other.0;
            let x0 = a.x.max(b.x);
            let y0 = a.y.max(b.y);
            let x1 =
                a.x.saturating_add(i64::from(a.w))
                    .min(b.x.saturating_add(i64::from(b.w)));
            let y1 =
                a.y.saturating_add(i64::from(a.h))
                    .min(b.y.saturating_add(i64::from(b.h)));
            if x0 >= x1 || y0 >= y1 {
                continue;
            }
            for k in [i, j] {
                if masks[k].is_none() {
                    masks[k] = Some(crate::raster::ink_mask(art[k].1)?);
                }
            }
            let target_mask = masks[i].as_ref().unwrap();
            let other_mask = masks[j].as_ref().unwrap();
            for y in y0..y1 {
                for x in x0..x1 {
                    if other_mask[((y - b.y) * i64::from(b.w) + (x - b.x)) as usize]
                        && !target_mask[((y - a.y) * i64::from(a.w) + (x - a.x)) as usize]
                    {
                        return Err(format!(
                            "元素 {} 的墨点遮挡了元素 {} 的码图，请调整位置。",
                            other.2 + 1,
                            target.2 + 1
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Check the resolved element for this record against the same geometry as the print head.
pub fn outside_media(media: &Media, field: &Field, artwork: Option<&str>, shift: u32) -> bool {
    let (w, h) = media.canvas_size();
    if field.x < 0 || field.y < 0 || field.x >= i64::from(w) || field.y >= i64::from(h) {
        return true;
    }
    let Some(artwork) = artwork else {
        return false;
    };
    if field
        .x
        .checked_add(i64::from(field.w))
        .is_none_or(|end| end > i64::from(w))
        || field
            .y
            .checked_add(i64::from(field.h))
            .is_none_or(|end| end > i64::from(h))
    {
        return true;
    }
    let (left, top) = (field.x as u32, field.y as u32);
    let cut = media.effective_cut();
    let margin = crate::stock::dots(cut.feed_margin_mm);
    if cut.radius_dots == 0
        && cut.regions_dots.is_empty()
        && left >= shift + margin
        && left + field.w <= w.saturating_sub(margin) + shift
    {
        return false;
    }
    let Ok(mask) = crate::raster::ink_mask(artwork) else {
        return false;
    };
    mask.iter().enumerate().any(|(i, &black)| {
        let x = left + (i as u32 % field.w);
        let y = top + (i as u32 / field.w);
        black
            && x.checked_sub(shift)
                .is_none_or(|px| !cut.contains(px, y, w, h))
    })
}

pub fn render_svg(template: &Template, row: &Row, seq: &str) -> Result<String, String> {
    validate_template(template)?;
    let mut art = Vec::with_capacity(template.elements.len());
    for (i, field) in template.elements.iter().enumerate() {
        let item = Artwork::new(field, row, seq, i).map_err(|e| format!("元素 {}：{e}", i + 1))?;
        art.push(item);
    }
    Ok(sheet_svg(
        &template.media,
        art.iter().map(|item| (&item.bounds, item.image.as_str())),
    ))
}

/// An unfinished label can render the valid items even when print checks fail.
pub fn draft_svg(media: &Media, items: &[(Field, Option<String>)]) -> String {
    sheet_svg(
        media,
        items
            .iter()
            .filter_map(|(field, art)| art.as_deref().map(|svg| (field, svg))),
    )
}

fn sheet_svg<'a>(media: &Media, items: impl Iterator<Item = (&'a Field, &'a str)>) -> String {
    let (width, height) = media.canvas_size();
    let mut svg = format!("<svg xmlns='http://www.w3.org/2000/svg' width='{width}' height='{height}' viewBox='0 0 {width} {height}'><rect width='{width}' height='{height}' fill='white'/>");
    for (field, artwork) in items {
        write!(
            svg,
            "<g transform='translate({} {})'>{artwork}</g>",
            field.x, field.y
        )
        .unwrap();
    }
    svg.push_str("</svg>");
    svg
}

pub fn preflight(
    template: &Template,
    rows: &[Row],
    quantity: usize,
    seq: Sequence,
) -> Result<Vec<String>, String> {
    let count = if rows.is_empty() {
        quantity
    } else {
        rows.len()
    };
    if count == 0 || count > MAX_BATCH {
        return Err(format!("单批须为 1～{MAX_BATCH} 条。"));
    }
    let empty = Row::new();
    (0..count)
        .map(|i| {
            seq.at(i)
                .and_then(|s| render_svg(template, rows.get(i).unwrap_or(&empty), &s))
                .map_err(|e| format!("第 {} 张：{e}", i + 1))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draft_uses_the_print_art_and_keeps_drawable_parts_of_unfinished_labels() {
        let mut template = Template::example();
        let row = Row::new();
        let items = |t: &Template| {
            t.elements
                .iter()
                .enumerate()
                .map(|(i, field)| match editor_art(field, &row, "0001", i) {
                    Ok((bounds, art)) => (bounds, Some(art)),
                    Err(_) => (field.clone(), None),
                })
                .collect::<Vec<_>>()
        };
        let original = render_svg(&template, &row, "0001").unwrap();
        assert_eq!(draft_svg(&template.media, &items(&template)), original);
        let mut missing = template.elements[0].clone();
        missing.text = "{missing}".into();
        template.elements.push(missing);
        assert!(render_svg(&template, &row, "0001").is_err());
        assert_eq!(draft_svg(&template.media, &items(&template)), original);

        template.elements.truncate(1);
        template.elements[0].x = 399;
        let clipped = draft_svg(&template.media, &items(&template));
        assert!(clipped.contains("translate(399 "));
        assert!(crate::raster::packed_n1(&clipped, &template.media).is_ok());
        assert_eq!(render_svg(&template, &row, "0001").unwrap(), clipped);
        template.elements[0].x = -4;
        let partial = render_svg(&template, &row, "0001").unwrap();
        assert!(crate::raster::packed_n1(&partial, &template.media)
            .unwrap()
            .iter()
            .any(|&b| b != 0));
        let blank = draft_svg(&template.media, &[]);
        assert!(crate::raster::packed_n1(&blank, &template.media)
            .unwrap()
            .iter()
            .all(|&byte| byte == 0));
    }

    #[test]
    fn both_p2_example_layouts_can_start_a_job() {
        for compact in [false, true] {
            let template = Template::p2_example(if compact {
                Media::p2_40x30()
            } else {
                Media::p2()
            });
            preflight(&template, &[], 1, Sequence::default()).unwrap();
        }
    }

    #[test]
    fn csv_sequence_and_mixed_arts_at_entry_point() {
        let rows = parse_csv("\u{feff}item,code\r\n\"A, B\",AB1\r\n物料,CD2\r\n").unwrap();
        let mut t = Template::example();
        t.elements[0].text = "{item} {seq}".into();
        t.elements[1].text = "{code}-{seq}".into();
        let job = preflight(
            &t,
            &rows,
            99,
            Sequence {
                start: 7,
                step: 2,
                pad: 3,
            },
        )
        .unwrap();
        assert_eq!(job.len(), 2);
        assert!(job[0].contains("A, B 007"));
        assert!(job[1].contains("物料 009"));
        assert!(job[1].contains("<path shape-rendering='crispEdges'"));
        assert_ne!(job[0], job[1]);
    }

    #[test]
    fn later_bad_row_aborts_entire_job() {
        let rows = parse_csv("code\nABC\n中文\n").unwrap();
        let mut t = Template::example();
        t.elements[1].text = "{code}".into();
        assert!(preflight(&t, &rows, 1, Sequence::default())
            .unwrap_err()
            .contains("第 2 张"));
    }

    #[test]
    fn placeholders_do_not_reexpand_csv_or_inject_xml() {
        let rows = parse_csv("a\n<g>{seq}</g>\n").unwrap();
        assert_eq!(
            materialize("{{{a}}}", &rows[0], "0001").unwrap(),
            "{<g>{seq}</g>}"
        );
        let t = Template {
            name: "test".into(),
            media: Media::default(),
            elements: vec![Field {
                id: uuid::Uuid::new_v4(),
                picture: None,
                kind: Kind::Text,
                font_size: Some(22),
                module_size: None,
                text: "{a}".into(),
                x: 0,
                y: 0,
                w: 400,
                h: 15,
            }],
        };
        let svg = render_svg(&t, &rows[0], "0001").unwrap();
        assert!(svg.contains("&lt;g&gt;{seq}&lt;/g&gt;"));
    }

    #[test]
    fn text_ink_dimensions_follow_glyphs_at_the_specified_font_size() {
        let size = 32;
        let (wide, tall) = text_dimensions("WWW", size).unwrap();
        assert!(wide > text_dimensions("iii", size).unwrap().0);
        assert!(wide < text_dimensions("WWWW", size).unwrap().0);
        assert!(tall > text_dimensions("...", size).unwrap().1);
        assert!(text_dimensions("WWW", 64).unwrap().1 > tall);

        let mut template = Template::example();
        template.elements.truncate(1);
        let item = &mut template.elements[0];
        item.text = "WWW".into();
        item.w = 1;
        let (width, height) = text_dimensions(&item.text, item.text_size()).unwrap();
        let viewport = format!("width='{width}' height='{height}'");
        let row = Row::new();
        assert!(render_editor_item(item, &row, "1")
            .unwrap()
            .contains(&viewport));
        assert!(render_svg(&template, &row, "1")
            .unwrap()
            .contains(&viewport));
        template.elements[0].w = 300;
        assert!(render_svg(&template, &row, "1")
            .unwrap()
            .contains(&viewport));
    }

    #[test]
    fn checks_overflow_and_geometry() {
        assert!(Sequence {
            start: i64::MAX,
            step: 1,
            pad: 3
        }
        .at(1)
        .is_err());
        assert_eq!(
            Sequence {
                start: -2,
                step: 1,
                pad: 3
            }
            .at(0)
            .unwrap(),
            "-002"
        );
        let mut t = Template::example();
        t.elements[0].x = i64::MAX;
        assert!(preflight(&t, &[], 1, Sequence::default()).is_err());
        let mut t = Template::example();
        t.elements[2].x = t.elements[1].x;
        t.elements[2].y = 12;
        assert!(preflight(&t, &[], 2, Sequence::default()).is_ok());
        let mut t = Template::example();
        t.elements[2].w = 40;
        // The module size, rather than a stale saved width, controls new QR fields.
        assert!(preflight(&t, &[], 1, Sequence::default()).is_ok());
    }

    #[test]
    fn code_in_space_between_letters_only_conflicts_when_ink_covers_it() {
        let mut template = Template::example();
        template.elements.remove(1);
        let text = &mut template.elements[0];
        text.x = 0;
        text.y = 0;
        text.text = "H                H".into();
        text.font_size = Some(28);
        template.elements[1].module_size = Some(1);
        template.elements[1].x = 26;
        template.elements[1].y = 0;
        let row = Row::new();
        let (text_bounds, _) = editor_art(&template.elements[0], &row, "1", 0).unwrap();
        assert!(text_bounds.w > 26 + 21);
        assert!(render_svg(&template, &row, "1").is_ok());
        let warning = |t: &Template| {
            let items = t
                .elements
                .iter()
                .enumerate()
                .map(|(i, field)| {
                    let (bounds, svg) = editor_art(field, &row, "1", i).unwrap();
                    (bounds, Some(svg))
                })
                .collect::<Vec<_>>();
            check_overlap(&items)
        };
        assert!(warning(&template).is_ok());
        template.elements[1].x = 0;
        assert!(warning(&template).unwrap_err().contains("遮挡"));
        assert!(render_svg(&template, &row, "1").is_ok());
    }

    #[test]
    fn text_boxes_can_overlap_in_output() {
        let mut t = Template::example();
        t.elements.truncate(1);
        let mut caption = t.elements[0].clone();
        caption.x += 10;
        caption.text = "A".into();
        t.elements.push(caption);
        let pages = preflight(&t, &[], 1, Sequence::default()).unwrap();
        assert!(pages[0].contains(">A</title>"));
    }

    #[test]
    fn oversized_or_ambiguous_csv_invalid() {
        assert!(parse_csv("a,a\n1,2\n").is_err());
        assert!(parse_csv("a,seq\nx,1\n").is_err());
        assert!(parse_csv("a,b\na\n").is_err());
        assert!(parse_csv(&format!("a\n{}", "x\n".repeat(MAX_BATCH + 1))).is_err());
    }

    #[test]
    fn p2_media_accepts_wide_elements_in_preflight_and_editor() {
        let mut template = Template {
            media: Media::p2(),
            ..Template::example()
        };
        template.elements.truncate(1);
        template.elements[0].kind = Kind::Qr;
        template.elements[0].h = 160;
        assert!(template.media.supports_p2());
        assert!(!template.media.is_known_n1());
        let pages = preflight(&template, &[], 1, Sequence::default()).unwrap();
        assert!(pages[0].contains("viewBox='0 0 400 384'"));
        assert!(render_editor_item(&template.elements[0], &Row::new(), "0001").is_ok());

        template.media.printable_width_dots -= 1;
        assert!(template.media.validate().is_ok());
        assert!(template.media.supports_p2());
        template.media.width_mm = 58;
        assert!(!template.media.supports_p2());
    }

    #[test]
    fn legacy_template_migrates_to_known_media() {
        let mut legacy = serde_json::to_value(Template::example()).unwrap();
        legacy.as_object_mut().unwrap().remove("media");
        let imported: Template = serde_json::from_value(legacy).unwrap();
        assert!(!imported.media.is_known_n1());
        let svg = &preflight(&imported, &[], 1, Sequence::default()).unwrap()[0];
        assert!(svg.contains("viewBox='0 0 400 96'"));
    }

    #[test]
    fn media_change_clips_existing_layout() {
        let mut template = Template::example();
        template.media = Media {
            dpi: DPI,
            cut: Default::default(),
            name: "窄标签，仅导出".into(),
            width_mm: 8,
            length_mm: 24,
            printable_width_dots: 64,
            printable_length_dots: 192,
        };
        assert!(!template.media.is_known_n1());
        assert!(preflight(&template, &[], 1, Sequence::default()).is_ok());
        template.elements.truncate(1);
        template.elements[0].w = 180;
        let svg = &preflight(&template, &[], 1, Sequence::default()).unwrap()[0];
        assert!(svg.contains("viewBox='0 0 192 64'"));
        template.media.printable_width_dots = 65;
        assert!(template.media.validate().unwrap_err().contains("超过纸面"));
        template.media.printable_width_dots = 385;
        assert!(template.media.validate().unwrap_err().contains("1～384"));
    }
}

#[cfg(test)]
#[test]
fn blank_default_and_cleanup_on_silver_stock() {
    assert!(Template::default().elements.is_empty());
    let (_, media, _) = builtin_media()
        .iter()
        .find(|(_, _, codes)| codes.contains(&"05262201"))
        .unwrap();
    let mut field = Template::example().elements.remove(0);
    field.x = 8;
    field.y = 10;
    field.w = 2;
    field.h = 1;
    let solid="<svg xmlns='http://www.w3.org/2000/svg' width='2' height='1'><rect width='2' height='1'/></svg>";
    let inset="<svg xmlns='http://www.w3.org/2000/svg' width='2' height='1'><rect x='1' width='1' height='1'/></svg>";
    assert!(outside_media(media, &field, Some(solid), 1));
    assert!(!outside_media(media, &field, Some(inset), 1));
    field.x = 120;
    assert!(!outside_media(media, &field, Some(solid), 1));
    field.x = 199;
    assert!(outside_media(media, &field, Some(solid), 1));
}
