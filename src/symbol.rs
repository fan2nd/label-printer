use crate::core::{Field, Kind};
use barcoders::sym::code128::Code128;
use qrcode::{Color, EcLevel, QrCode};
use std::fmt::Write;

pub enum Symbol {
    Qr(QrCode),
    Barcode(Vec<u8>),
}

pub fn encode(kind: Kind, content: &str) -> Result<Symbol, String> {
    if content.trim().is_empty() {
        return Err("内容为空。".into());
    }
    match kind {
        Kind::Qr => QrCode::with_error_correction_level(content.as_bytes(), EcLevel::M)
            .map(Symbol::Qr)
            .map_err(|_| "二维码内容过长。".into()),
        Kind::Barcode => {
            if !content.bytes().all(|c| (32..=126).contains(&c)) {
                return Err("Code 128 只接受可打印 ASCII，中文请用二维码。".into());
            }
            let numeric = content.len() % 2 == 0 && content.bytes().all(|b| b.is_ascii_digit());
            let charset = if numeric { '\u{0106}' } else { '\u{0181}' };
            let code = Code128::new(format!("{charset}{content}"))
                .map_err(|_| "Code 128 无法编码该内容。")?;
            Ok(Symbol::Barcode(code.encode()))
        }
        Kind::Image | Kind::Text => Err("文字元素无法编码为条码。".into()),
    }
}

impl Symbol {
    /// Resolve a legacy allocation or explicit module size into the symbol's ink bounds.
    pub fn layout(&self, field: &Field) -> Result<Field, String> {
        const OVERFLOW: &str = "条码尺寸或坐标超出范围。";
        let (modules, legacy_unit) = match self {
            Self::Qr(qr) => {
                let side = qr.width() as u32;
                (side, (field.w.min(field.h) / (side + 8)).max(2))
            }
            Self::Barcode(bits) => {
                let length = u32::try_from(bits.len()).map_err(|_| OVERFLOW)?;
                let with_margins = length.checked_add(20).ok_or(OVERFLOW)?;
                (length, (field.w / with_margins).clamp(2, 4))
            }
        };
        let unit = field.module_size.unwrap_or(legacy_unit);
        if unit == 0 || (matches!(self, Self::Barcode(_)) && field.h == 0) {
            return Err("条码模块尺寸和高度必须大于零。".into());
        }
        let mut ink = field.clone();
        ink.module_size = Some(unit);
        ink.w = modules.checked_mul(unit).ok_or(OVERFLOW)?;
        if matches!(self, Self::Qr(_)) {
            ink.h = ink.w;
        }
        if field.module_size.is_none() {
            // Center the ink within the old outer box, which included the quiet zone.
            // An undersized old box expands from its existing origin.
            ink.x = ink
                .x
                .checked_add(i64::from(field.w.saturating_sub(ink.w) / 2))
                .ok_or(OVERFLOW)?;
            ink.y = ink
                .y
                .checked_add(i64::from(field.h.saturating_sub(ink.h) / 2))
                .ok_or(OVERFLOW)?;
        }
        ink.x.checked_add(i64::from(ink.w)).ok_or(OVERFLOW)?;
        ink.y.checked_add(i64::from(ink.h)).ok_or(OVERFLOW)?;
        Ok(ink)
    }

    pub fn draw(&self, svg: &mut String, resolved: &Field) {
        let unit = resolved
            .module_size
            .expect("symbol layout resolves the module size");
        match self {
            Self::Qr(qr) => {
                svg.push_str("<path shape-rendering='crispEdges' d='");
                for row in 0..qr.width() {
                    for col in 0..qr.width() {
                        if qr[(col, row)] == Color::Dark {
                            let x = resolved.x + i64::from(col as u32 * unit);
                            let y = resolved.y + i64::from(row as u32 * unit);
                            write!(svg, "M{x} {y}h{unit}v{unit}h-{unit}z").unwrap();
                        }
                    }
                }
                svg.push_str("'/>");
            }
            Self::Barcode(bits) => {
                for (col, &bit) in bits.iter().enumerate() {
                    if bit == 1 {
                        let x = resolved.x + i64::from(col as u32 * unit);
                        write!(svg, "<rect x='{x}' y='{}' width='{unit}' height='{}' shape-rendering='crispEdges'/>", resolved.y, resolved.h).unwrap();
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Template;

    #[test]
    fn legacy_qr_preserves_centered_ink_and_fixed_modules_preserve_origin() {
        let code = encode(Kind::Qr, "A").unwrap();
        let mut field = Template::example().elements[2].clone();
        field.x = 3;
        field.y = 5;
        field.w = 62;
        field.h = 59;
        field.module_size = None;
        let legacy = code.layout(&field).unwrap();
        assert_eq!((legacy.x, legacy.y, legacy.w, legacy.h), (13, 13, 42, 42));
        assert_eq!(legacy.module_size, Some(2));
        field.module_size = Some(1);
        let fixed = code.layout(&field).unwrap();
        assert_eq!((fixed.x, fixed.y, fixed.w, fixed.h), (3, 5, 21, 21));
        field.module_size = Some(u32::MAX);
        assert!(code.layout(&field).is_err());
    }

    #[test]
    fn barcode_size_accounts_for_numeric_encoding_and_legacy_quiet_zones() {
        let code = encode(Kind::Barcode, "1234").unwrap();
        let mut field = Template::example().elements[1].clone();
        field.w = 300;
        field.module_size = None;
        let legacy = code.layout(&field).unwrap();
        // Start C, two data symbols, checksum, and stop = 4 * 11 + 13 modules.
        assert_eq!(legacy.module_size, Some(3));
        assert_eq!((legacy.x, legacy.w, legacy.h), (field.x + 64, 171, field.h));
        field.w = 1000;
        assert_eq!(code.layout(&field).unwrap().module_size, Some(4));
        field.w = 3;
        assert_eq!(code.layout(&field).unwrap().module_size, Some(2));
        field.module_size = Some(1);
        let fixed = code.layout(&field).unwrap();
        assert_eq!((fixed.x, fixed.w), (field.x, 57));
        field.x = i64::MAX;
        assert!(code.layout(&field).is_err());
    }
}
