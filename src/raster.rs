use crate::core::{Media, P2_HEAD_DOTS};
use base64::{engine::general_purpose::STANDARD, Engine};
use resvg::{
    tiny_skia::{Pixmap, Transform},
    usvg::{fontdb, Options, Tree},
};
use std::sync::{Arc, OnceLock};

pub(crate) fn fonts() -> Arc<fontdb::Database> {
    static FONTS: OnceLock<Arc<fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = fontdb::Database::new();
            db.load_system_fonts();
            // Prefer fonts with Chinese glyphs. USVG can resolve additional faces
            // from the same system font database when a glyph is missing.
            let family = [
                "Microsoft YaHei",
                "PingFang SC",
                "Noto Sans CJK SC",
                "WenQuanYi Zen Hei",
                "Arial",
                "DejaVu Sans",
            ]
            .into_iter()
            .find(|name| {
                db.faces()
                    .any(|f| f.families.iter().any(|(n, _)| n == name))
            })
            .or_else(|| {
                db.faces()
                    .next()
                    .and_then(|f| f.families.first())
                    .map(|(s, _)| s.as_str())
            })
            .map(str::to_owned);
            if let Some(family) = family {
                db.set_sans_serif_family(family);
            }
            Arc::new(db)
        })
        .clone()
}

pub(crate) fn ink_mask(svg: &str) -> Result<Vec<bool>, String> {
    let options = Options {
        fontdb: fonts(),
        ..Options::default()
    };
    let tree = Tree::from_str(svg, &options).map_err(|_| "无法解析元素图像。")?;
    let size = tree.size().to_int_size();
    let mut pix = Pixmap::new(size.width(), size.height()).ok_or("无法分配元素图像。")?;
    pix.fill(resvg::tiny_skia::Color::WHITE);
    resvg::render(&tree, Transform::identity(), &mut pix.as_mut());
    Ok(pix
        .data()
        .chunks_exact(4)
        .map(|p| 299 * u32::from(p[0]) + 587 * u32::from(p[1]) + 114 * u32::from(p[2]) < 128_000)
        .collect())
}

// The N1 needs a one-dot feed correction measured on this model.
pub(crate) const N1_FEED_SHIFT: u32 = 1;

pub fn packed_n1(svg: &str) -> Result<Vec<u8>, String> {
    pack_rotated(svg, 400, 96, 96, N1_FEED_SHIFT as usize)
}

pub fn packed_p2(svg: &str, media: &Media) -> Result<Vec<u8>, String> {
    if !media.supports_p2() {
        return Err("P2 纸宽最多 57 mm，有效打印宽度最多 384 点，请检查标签规格。".into());
    }
    let (along, across) = media.canvas_size();
    pack_rotated(svg, along, across, P2_HEAD_DOTS, 0)
}

/// Unsupported stock is still editable in its native printable-dot coordinates.
pub(crate) fn stock_preview_png(svg: &str, media: &Media) -> Result<String, String> {
    media.validate()?;
    let (along, across) = media.canvas_size();
    let packed = pack_rotated(svg, along, across, across, 0)?;
    preview_png(&packed, media, across)
}

fn pack_rotated(
    svg: &str,
    along: u32,
    across: u32,
    head_dots: u32,
    skip_feed_dots: usize,
) -> Result<Vec<u8>, String> {
    let opts = Options {
        fontdb: fonts(),
        ..Default::default()
    };
    let tree = Tree::from_str(svg, &opts).map_err(|_| "标签无法生成打印图像。")?;
    let size = tree.size();
    if size.width() != along as f32 || size.height() != across as f32 {
        return Err(format!(
            "打印图像须为 {along} × {across} 点，请检查标签规格。"
        ));
    }
    let mut pix = Pixmap::new(along, across).ok_or("无法分配打印缓冲区。")?;
    resvg::render(&tree, Transform::identity(), &mut pix.as_mut());
    let along = along as usize;
    let across = across as usize;
    let head_dots = head_dots as usize;
    let stride = head_dots.div_ceil(8);
    // The roll guides center the stock on the P2 head. Pad each row on both sides.
    let left_margin = (head_dots - across) / 2;
    let mut rotated = vec![0; along * stride];
    let px = pix.data();
    for source_y in 0..across {
        let x = left_margin + across - 1 - source_y;
        for source_x in skip_feed_dots..along {
            let i = 4 * (source_y * along + source_x);
            let dark =
                299 * u32::from(px[i]) + 587 * u32::from(px[i + 1]) + 114 * u32::from(px[i + 2]);
            if px[i + 3] > 32 && dark < 128_000 {
                rotated[(source_x - skip_feed_dots) * stride + x / 8] |= 0x80 >> (x % 8);
            }
        }
    }
    Ok(rotated)
}

/// Rotate the transmitted head rows back to canvas orientation. Hardware feed
/// corrections are already in the bytes and must stay visible in the preview.
pub fn preview_png(packed: &[u8], media: &Media, head_dots: u32) -> Result<String, String> {
    media.validate()?;
    let (along, across) = media.canvas_size();
    let head_stride = head_dots.div_ceil(8);
    if across > head_dots || packed.len() as u64 != u64::from(head_stride) * u64::from(along) {
        return Err("打印点阵与标签规格不匹配，无法预览。".into());
    }
    let left_margin = (head_dots - across) / 2;
    let png_stride = along.div_ceil(8) as usize;
    // In 1-bit grayscale PNG, a set bit is white; on the head, it is black.
    let mut pixels = vec![0xff; png_stride * across as usize];
    for (y, png_row) in pixels.chunks_exact_mut(png_stride).enumerate() {
        let head_x = (left_margin + across - 1) as usize - y;
        let head_bit = 0x80 >> (head_x % 8);
        for (x, head_row) in packed.chunks_exact(head_stride as usize).enumerate() {
            if head_row[head_x / 8] & head_bit != 0 {
                png_row[x / 8] &= !(0x80 >> (x % 8));
            }
        }
    }

    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, along, across);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::One);
        let mut writer = encoder.write_header().map_err(|_| "无法生成点阵预览。")?;
        writer
            .write_image_data(&pixels)
            .map_err(|_| "无法生成点阵预览。")?;
        writer.finish().map_err(|_| "无法生成点阵预览。")?;
    }
    Ok(format!("data:image/png;base64,{}", STANDARD.encode(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rotate_shift_and_pack_coordinates() {
        let svg = r#"<svg xmlns='http://www.w3.org/2000/svg' width='400' height='96'><rect width='400' height='96' fill='white'/><rect x='1' y='95' width='1' height='1'/><rect x='9' y='0' width='1' height='1'/><rect x='0' y='92' width='1' height='1'/></svg>"#;
        let out = packed_n1(svg).unwrap();
        assert_eq!(out.len(), 4800);
        assert_eq!(out[0], 0x80);
        assert_eq!(out[8 * 12 + 11], 1);
        assert_eq!(out.into_iter().map(u8::count_ones).sum::<u32>(), 2);
    }
    #[test]
    fn p2_packs_full_width_and_both_feed_edges_without_n1_shift() {
        let svg = r#"<svg xmlns='http://www.w3.org/2000/svg' width='400' height='384'>
            <rect x='0' y='383' width='1' height='1'/>
            <rect x='1' y='375' width='1' height='1'/>
            <rect x='399' y='0' width='1' height='1'/>
        </svg>"#;
        let page = packed_p2(svg, &Media::p2()).unwrap();
        assert_eq!(page.len(), 400 * 48);
        assert_eq!(page[0], 0x80);
        assert_eq!(page[48 + 1], 0x80);
        assert_eq!(page[399 * 48 + 47], 0x01);
        assert_eq!(page.into_iter().map(u8::count_ones).sum::<u32>(), 3);
        assert!(packed_n1(svg).is_err());
        assert!(packed_p2(
            "<svg xmlns='http://www.w3.org/2000/svg' width='400' height='96'/>",
            &Media::p2()
        )
        .is_err());
    }

    #[test]
    fn p2_centers_40x30_stock_and_preserves_feed_length() {
        let media = Media::p2_40x30();
        let svg = r#"<svg xmlns='http://www.w3.org/2000/svg' width='240' height='320'>
            <rect x='0' y='319' width='1' height='1'/>
            <rect x='239' y='0' width='1' height='1'/>
        </svg>"#;
        let page = packed_p2(svg, &media).unwrap();
        assert_eq!(page.len(), 240 * 48);
        assert_eq!(page[4], 0x80);
        assert_eq!(page[239 * 48 + 43], 1);
        assert_eq!(page.into_iter().map(u8::count_ones).sum::<u32>(), 2);
        assert!(packed_p2(svg, &Media::p2()).is_err());
        assert!(packed_p2(
            svg,
            &Media {
                width_mm: 39,
                ..media
            }
        )
        .is_err());
    }

    fn preview_dots(url: &str, size: (u32, u32)) -> Vec<(u32, u32)> {
        let bytes = STANDARD
            .decode(url.strip_prefix("data:image/png;base64,").unwrap())
            .unwrap();
        let mut reader = png::Decoder::new(bytes.as_slice()).read_info().unwrap();
        let mut data = vec![0; reader.output_buffer_size()];
        let frame = reader.next_frame(&mut data).unwrap();
        assert_eq!((frame.width, frame.height), size);
        assert_eq!(
            (frame.color_type, frame.bit_depth),
            (png::ColorType::Grayscale, png::BitDepth::One)
        );
        let mut ink = Vec::new();
        for y in 0..frame.height {
            for x in 0..frame.width {
                let byte = data[y as usize * frame.line_size + (x / 8) as usize];
                if byte & (0x80 >> (x % 8)) == 0 {
                    ink.push((x, y));
                }
            }
        }
        ink
    }

    #[test]
    fn n1_preview_includes_feed_correction_and_last_blank_column() {
        let media = Media::default();
        let svg = r#"<svg xmlns='http://www.w3.org/2000/svg' width='400' height='96'>
            <rect x='0' y='17' width='1' height='1'/>
            <rect x='1' y='95' width='1' height='1'/>
            <rect x='399' y='0' width='1' height='1'/>
        </svg>"#;
        let packed = packed_n1(svg).unwrap();
        let url = preview_png(&packed, &media, 96).unwrap();
        assert_eq!(preview_dots(&url, (400, 96)), [(398, 0), (0, 95)]);
    }

    #[test]
    fn p2_preview_crops_centered_stock_and_preserves_partial_bytes() {
        let mut media = Media::p2_40x30();
        media.printable_length_dots = 17;
        media.printable_width_dots = 11;
        // 11 centered dots occupy head positions 186 through 196 (inclusive).
        let mut packed = vec![0; 17 * 48];
        packed[186 / 8] |= 0x80 >> (186 % 8);
        packed[16 * 48 + 196 / 8] |= 0x80 >> (196 % 8);
        packed[8 * 48 + 185 / 8] |= 0x80 >> (185 % 8); // outside the crop
        let url = preview_png(&packed, &media, P2_HEAD_DOTS).unwrap();
        assert_eq!(preview_dots(&url, (17, 11)), [(16, 0), (0, 10)]);
        assert!(preview_png(&packed[1..], &media, P2_HEAD_DOTS).is_err());
        assert!(preview_png(&[], &media, 8).is_err());
    }

    #[cfg(feature = "desktop")]
    #[test]
    fn unsupported_but_valid_stock_displays_at_its_own_dot_coordinates() {
        use crate::device::Model;
        let mut media = Media::default();
        media.width_mm = 60;
        media.printable_length_dots = 15;
        media.printable_width_dots = 11;
        let svg = r#"<svg xmlns='http://www.w3.org/2000/svg' width='15' height='11'><rect x='0' y='0' width='1' height='1'/><rect x='14' y='10' width='1' height='1'/></svg>"#;
        for model in [Model::N1, Model::P2] {
            assert!(!model.supports(&media));
            assert!(model.rasterize(svg, &media).is_err());
            assert_eq!(model.preview_feed_shift(&media), 0);
            assert_eq!(
                preview_dots(&model.preview_png(svg, &media).unwrap(), (15, 11)),
                [(0, 0), (14, 10)]
            );
        }
        assert_eq!(Model::N1.preview_feed_shift(&Media::default()), 1);
        assert_eq!(Model::P2.preview_feed_shift(&Media::default()), 0);
    }

    #[test]
    fn different_media_and_invalid_xml_block_raster() {
        assert!(packed_n1("<svg>").is_err());
        assert!(
            packed_n1("<svg xmlns='http://www.w3.org/2000/svg' width='399' height='96'/>").is_err()
        );
    }
}
