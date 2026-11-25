use crate::core::{Field, Kind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gesture {
    Move,
    Resize,
}

/// Operates on this record's resolved geometry (including its symbol module size).
pub fn transform(field: &Field, mode: Gesture, dx: i32, dy: i32) -> Field {
    let width = crate::core::MAX_MEDIA_LENGTH_DOTS;
    let height = crate::core::MAX_MEDIA_WIDTH_DOTS;
    let mut next = field.clone();
    let adjust = |v: u32, d: i32, min: u32, max: u32| {
        (i64::from(v) + i64::from(d)).clamp(i64::from(min), i64::from(max.max(min))) as u32
    };
    match mode {
        Gesture::Move => {
            next.x = field.x.saturating_add(i64::from(dx));
            next.y = field.y.saturating_add(i64::from(dy));
        }
        Gesture::Resize if field.kind == Kind::Image => {
            next.w = adjust(field.w, dx, 1, width);
            next.h = adjust(field.h, dy, 1, height);
        }
        Gesture::Resize if field.kind != Kind::Text => {
            if let Some(unit) = field.module_size.filter(|unit| *unit > 0) {
                let columns = (field.w / unit).max(1);
                let square = field.kind == Kind::Qr;
                let delta = if square && dy.unsigned_abs() > dx.unsigned_abs() {
                    dy
                } else {
                    dx
                };
                if delta != 0 {
                    let mut limit = width;
                    if square {
                        limit = limit.min(height);
                    }
                    let dots = adjust(field.w, delta, 1, limit);
                    let scaled =
                        ((u64::from(dots) + u64::from(columns / 2)) / u64::from(columns)) as u32;
                    let scaled = scaled.clamp(1, (limit / columns).max(1));
                    next.module_size = Some(scaled);
                    next.w = scaled * columns;
                    if square {
                        next.h = next.w;
                    }
                }
            }
            if field.kind == Kind::Barcode && dy != 0 {
                next.h = adjust(field.h, dy, 1, height);
            }
        }
        Gesture::Resize => {}
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;
    fn field(kind: Kind) -> Field {
        Field {
            id: uuid::Uuid::new_v4(),
            picture: None,
            kind,
            text: "x".into(),
            x: 16,
            y: 8,
            w: 84,
            h: 84,
            module_size: Some(4),
            font_size: None,
        }
    }

    #[test]
    fn move_past_edges_preserves_dimensions() {
        let f = transform(&field(Kind::Text), Gesture::Move, 1000, -50);
        assert_eq!((f.x, f.y, f.w, f.h), (1016, -42, 84, 84));
    }

    #[test]
    fn corner_resize_snaps_codes_to_whole_modules() {
        let qr = transform(&field(Kind::Qr), Gesture::Resize, 20, 38);
        assert_eq!(
            (qr.x, qr.y, qr.w, qr.h, qr.module_size),
            (16, 8, 126, 126, Some(6))
        );
        let bars = transform(&field(Kind::Barcode), Gesture::Resize, -20, -10);
        assert_eq!((bars.w, bars.h, bars.module_size), (63, 74, Some(3)));
        let text = field(Kind::Text);
        assert_eq!(transform(&text, Gesture::Resize, 40, 40), text);
    }
}
