use super::*;
use crate::{
    core::{Field, Kind},
    raster, symbol,
};
use std::fmt::Write;
pub const BAR_WIDTH: usize = 24;
#[derive(PartialEq)]
struct Tile {
    w: usize,
    h: usize,
    svg: String,
}
pub struct Sheet {
    pub page: Vec<u8>,
    pub preview: String,
}
#[derive(PartialEq)]
pub struct Layout {
    tiles: Vec<Vec<(usize, usize, Tile)>>,
    pub omitted: Vec<usize>,
}
impl Layout {
    pub fn count(&self) -> usize {
        self.tiles.len()
    }
    pub fn build(profile: &Profile) -> Result<Self, String> {
        let m = &profile.media;
        m.validate()?;
        if !m.supports_p2() {
            return Err("请选择 P2 规格。".into());
        }
        let (feed, across) = m.canvas_size();
        let width = across.saturating_sub(8) as usize;
        let height = feed.saturating_sub(24) as usize;
        let mut layout = Self {
            tiles: vec![],
            omitted: vec![],
        };
        let mut heights: Vec<Vec<usize>> = vec![];
        let font = profile
            .font
            .replace('&', "&amp;")
            .replace('\'', "&apos;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        for i in 0..ITEMS.len() {
            let tile = tile(i, m.dpi, &font, height)?;
            if tile.w > width || tile.h > height {
                layout.omitted.push(i);
                continue;
            }
            let mut slot = None;
            for (sheet, sky) in heights.iter().enumerate() {
                if let Some((x, y)) = sky
                    .windows(tile.w)
                    .enumerate()
                    .map(|(x, r)| (x, *r.iter().max().unwrap()))
                    .filter(|(_, y)| y + tile.h <= height)
                    .min_by_key(|(_, y)| *y)
                {
                    slot = Some((sheet, x, y));
                    break;
                }
            }
            let (sheet, x, y) = slot.unwrap_or_else(|| {
                heights.push(vec![0; width]);
                layout.tiles.push(vec![]);
                (layout.tiles.len() - 1, 0, 0)
            });
            heights[sheet][x..x + tile.w].fill(y + tile.h);
            layout.tiles[sheet].push((x + 4, y + 20, tile));
        }
        if layout.count() == 0 {
            return Err("此纸张放不下样张项目，请更换规格。".into());
        }
        Ok(layout)
    }
    pub fn render(&self, profile: &Profile, level: u8, index: usize) -> Result<Sheet, String> {
        let (feed, across) = profile.media.canvas_size();
        let tiles = self.tiles.get(index).ok_or("无此样张页")?;
        let mut raw=format!("<path d='M5 12L10 3L15 12Z'/><text x='20' y='15' font-family='sans-serif' font-size='13'>D{level} · {}/{}</text>",index+1,self.count());
        for (x, y, tile) in tiles {
            write!(raw, "<g transform='translate({x} {y})'>{}</g>", tile.svg).unwrap();
        }
        // Turn the physical portrait layout into the editor's along × across axes.
        let svg=format!("<svg xmlns='http://www.w3.org/2000/svg' width='{feed}' height='{across}'><g transform='translate(0 {across}) rotate(-90)'>{raw}</g></svg>");
        let page = raster::packed_p2(&svg, &profile.media)?;
        let stride = across.div_ceil(8) as usize;
        let offset = (384 - across as usize) / 2;
        let mut preview_bits = vec![0xff; stride * feed as usize];
        for (y, row) in page.chunks_exact(48).enumerate() {
            for x in 0..across as usize {
                let dot = offset + x;
                if row[dot / 8] & (0x80 >> (dot % 8)) != 0 {
                    preview_bits[y * stride + x / 8] &= !(0x80 >> (x % 8));
                }
            }
        }
        let preview = raster::grayscale_png(across, feed, png::BitDepth::One, &preview_bits)?;
        Ok(Sheet { page, preview })
    }
}
fn tile(i: usize, dpi: u32, font: &str, height: usize) -> Result<Tile, String> {
    if i < 5 {
        let mm = [1, 3, 6, 12, 20][i];
        let len = (f64::from(mm * dpi) / 25.4).round() as usize;
        // Keep a clean trailing zone beyond the common end of every bar.
        let h = height
            .min((20.0 * f64::from(dpi) / 25.4).round() as usize + 9)
            .max(len + 9);
        let end = h - 8;
        return Ok(Tile{w:38,h,svg:format!("<rect width='24' height='{len}' y='{}'/><text transform='translate(34 {}) rotate(-90)' font-family='sans-serif' font-size='10'>{mm}</text>",end-len,end)});
    }
    if i < 9 {
        let size = if i < 7 { 12 } else { 18 };
        let weight = if i % 2 == 0 { "bold" } else { "normal" };
        let s=format!("<text x='1' y='{size}' font-family='{font}, sans-serif' font-size='{size}' font-weight='{weight}'>阻值 A08Ω</text>");
        return Ok(Tile {
            w: 94,
            h: size + 5,
            svg: s,
        });
    }
    let kind = if i == 9 { Kind::Qr } else { Kind::Barcode };
    let code = symbol::encode(kind, if i == 9 { "P2" } else { "4030" })?;
    let margin = if i == 9 { 8 } else { 20 };
    let field = Field {
        id: Uuid::nil(),
        kind,
        text: String::new(),
        x: margin,
        y: if i == 9 { 8 } else { 3 },
        w: 1,
        h: 24,
        module_size: Some(2),
        rotation: 0,
        font_size: None,
        font_family: None,
        picture: None,
        stroke_dots: 1,
        filled: false,
    };
    let field = code.layout(&field)?;
    let mut svg = String::new();
    code.draw(&mut svg, &field);
    Ok(Tile {
        w: field.w as usize + margin as usize * 2,
        h: field.h as usize + field.y as usize * 2,
        svg,
    })
}
