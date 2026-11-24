use crate::core::{Field, Kind, Media, MAX_MEDIA_LENGTH_DOTS, MAX_MEDIA_WIDTH_DOTS};
use base64::{engine::general_purpose::STANDARD, Engine};
use image::{
    imageops::{self, FilterType},
    GrayImage, ImageReader, Luma,
};
use serde::{Deserialize, Serialize};
use std::{io::Cursor, path::Path, sync::Arc};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Picture {
    width: u32,
    height: u32,
    gray: Arc<str>,
    pub threshold: u8,
    pub dither: bool,
    pub invert: bool,
}

fn fit(w: u32, h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    let scale = (max_w as f64 / w as f64)
        .min(max_h as f64 / h as f64)
        .min(1.);
    (
        (w as f64 * scale).round().max(1.) as u32,
        (h as f64 * scale).round().max(1.) as u32,
    )
}

pub fn import(path: &Path, media: &Media) -> Result<Field, String> {
    let reader = ImageReader::open(path)
        .and_then(|r| r.with_guessed_format())
        .map_err(|_| "无法读取图片文件。")?;
    let decoder = reader
        .into_decoder()
        .map_err(|_| "无法解码图片，请选择 PNG、JPEG 或 BMP。")?;
    use image::ImageDecoder;
    let (sw, sh) = decoder.dimensions();
    if sw == 0 || sh == 0 || u64::from(sw) * u64::from(sh) > 24_000_000 {
        return Err("图片尺寸无效或超过 2400 万像素。".into());
    }
    let mut decoder = decoder;
    let orientation = decoder.orientation().map_err(|_| "无法读取图片方向。")?;
    let mut source = image::DynamicImage::from_decoder(decoder).map_err(|_| "图片数据损坏。")?;
    source.apply_orientation(orientation);
    let rgba = source.into_rgba8();
    let gray = GrayImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let [r, g, b, a] = rgba.get_pixel(x, y).0;
        let luma = (299 * u32::from(r) + 587 * u32::from(g) + 114 * u32::from(b) + 500) / 1000;
        Luma([((luma * u32::from(a) + 255 * (255 - u32::from(a)) + 127) / 255) as u8])
    });
    let (width, height) = fit(
        gray.width(),
        gray.height(),
        MAX_MEDIA_LENGTH_DOTS,
        MAX_MEDIA_WIDTH_DOTS,
    );
    let gray = imageops::resize(&gray, width, height, FilterType::Triangle);
    let (mw, mh) = media.canvas_size();
    let (w, h) = fit(width, height, mw, mh);
    Ok(Field {
        id: uuid::Uuid::new_v4(),
        kind: Kind::Image,
        text: String::new(),
        x: 0,
        y: 0,
        w,
        h,
        font_size: None,
        module_size: None,
        picture: Some(Picture {
            width,
            height,
            gray: STANDARD.encode(gray.as_raw()).into(),
            threshold: 128,
            dither: false,
            invert: false,
        }),
    })
}

impl Picture {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=MAX_MEDIA_LENGTH_DOTS).contains(&self.width)
            || !(1..=MAX_MEDIA_WIDTH_DOTS).contains(&self.height)
            || self.gray.len() != (self.width * self.height).div_ceil(3) as usize * 4
        {
            return Err("模板图片尺寸无效。".into());
        }
        Ok(())
    }
    fn pixels(&self) -> Result<GrayImage, String> {
        self.validate()?;
        let bytes = STANDARD
            .decode(self.gray.as_bytes())
            .map_err(|_| "模板图片数据无效。")?;
        if bytes.len() != (self.width * self.height) as usize {
            return Err("模板图片数据长度无效。".into());
        }
        GrayImage::from_raw(self.width, self.height, bytes)
            .ok_or_else(|| "模板图片数据长度无效。".into())
    }
    pub fn svg(&self, w: u32, h: u32) -> Result<String, String> {
        if !(1..=MAX_MEDIA_LENGTH_DOTS).contains(&w) || !(1..=MAX_MEDIA_WIDTH_DOTS).contains(&h) {
            return Err("图片尺寸超过打印范围。".into());
        }
        let gray = imageops::resize(&self.pixels()?, w, h, FilterType::Triangle);
        let stride = w.div_ceil(8) as usize;
        let mut bits = vec![0; stride * h as usize];
        const BAYER: [[i16; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
        for (x, y, p) in gray.enumerate_pixels() {
            let level = i16::from(p[0]);
            let offset = if self.dither {
                BAYER[y as usize % 4][x as usize % 4] * 16 - 120
            } else {
                0
            };
            let black = level < (i16::from(self.threshold) + offset).clamp(0, 255);
            if black ^ self.invert {
                bits[y as usize * stride + x as usize / 8] |= 0x80 >> (x % 8);
            }
        }
        let mut out = Cursor::new(Vec::new());
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(png::ColorType::Indexed);
            enc.set_depth(png::BitDepth::One);
            enc.set_palette(vec![255, 255, 255, 0, 0, 0]);
            enc.set_trns(vec![0, 255]);
            let mut writer = enc.write_header().map_err(|_| "无法生成图片。")?;
            writer
                .write_image_data(&bits)
                .map_err(|_| "无法生成图片。")?;
            writer.finish().map_err(|_| "无法生成图片。")?;
        }
        let data = STANDARD.encode(out.into_inner());
        Ok(format!("<svg xmlns='http://www.w3.org/2000/svg' xmlns:xlink='http://www.w3.org/1999/xlink' width='{w}' height='{h}'><image width='{w}' height='{h}' xlink:href='data:image/png;base64,{data}'/></svg>"))
    }
}

#[cfg(all(test, feature = "desktop"))]
mod tests {
    use super::*;
    use crate::{
        core::{self, Row, Template},
        device::Model,
        editor, storage,
    };

    #[test]
    fn imported_image_scales_dithers_inverts_and_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("gray.png");
        let src = image::RgbaImage::from_fn(8, 8, |x, _| match x {
            0 => image::Rgba([0, 0, 0, 0]),
            1 => image::Rgba([0, 0, 0, 255]),
            _ => image::Rgba([128, 128, 128, 255]),
        });
        src.save(&path).unwrap();
        let media = Media::default();
        let mut field = import(&path, &media).unwrap();
        let pic = field.picture.as_mut().unwrap();
        let mask = |p: &Picture| crate::raster::ink_mask(&p.svg(8, 8).unwrap()).unwrap();
        let before = mask(pic);
        assert!(!before[0] && before[1] && !before[2]);
        pic.threshold = 160;
        assert!(mask(pic)[2]);
        pic.dither = true;
        let dither = mask(pic);
        assert_ne!(dither, before);
        assert!(dither[2..8].contains(&true) && dither[2..8].contains(&false));
        pic.invert = true;
        assert!(mask(pic).iter().zip(&dither).all(|(a, b)| a != b));
        let scaled = editor::transform(&field, &media, editor::Gesture::Resize, 8, 4);
        assert_eq!((scaled.w, scaled.h), (16, 12));
        let template = Template {
            name: "测试".into(),
            media,
            elements: vec![scaled],
        };
        let path = tmp.path().join("image.json");
        storage::export_template(&path, &template).unwrap();
        assert_eq!(template, storage::import_template(&path).unwrap());
        let svg = core::render_svg(&template, &Row::new(), "1").unwrap();
        assert!(Model::N1
            .rasterize(&svg, &template.media)
            .unwrap()
            .iter()
            .any(|x| *x != 0));
    }
}
