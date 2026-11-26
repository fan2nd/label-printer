use crate::core::{Field, Kind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gesture {
    Move,
    Resize,
}

/// Layer order in a template is back to front, matching the print renderer.
pub fn move_layer(elements: &mut [Field], from: usize, to: usize, selected: usize) -> usize {
    let id = elements[selected].id;
    if from < to {
        elements[from..=to].rotate_left(1);
    } else {
        elements[to..=from].rotate_right(1);
    }
    elements.iter().position(|item| item.id == id).unwrap()
}

/// Align the resolved footprint to the displayed sheet, including its feed offset.
pub fn align(
    field: &mut Field,
    sheet: (u32, u32),
    turns: u8,
    feed_shift: u32,
    horizontal: bool,
    edge: i64,
) {
    let stock_x = horizontal != (turns % 2 == 1);
    let reversed = if horizontal {
        matches!(turns, 1 | 2)
    } else {
        matches!(turns, 2 | 3)
    };
    let edge = if reversed { 2 - edge } else { edge };
    let (w, h) = field.footprint();
    if stock_x {
        field.x = i64::from(feed_shift) + (i64::from(sheet.0) - i64::from(w)) * edge / 2;
    } else {
        field.y = (i64::from(sheet.1) - i64::from(h)) * edge / 2;
    }
}

/// Operates on this record's resolved geometry (including its symbol module size).
pub fn transform(field: &Field, mode: Gesture, dx: i32, dy: i32) -> Field {
    let width = crate::core::MAX_MEDIA_LENGTH_DOTS;
    let height = crate::core::MAX_MEDIA_WIDTH_DOTS;
    let (dx, dy) = if mode == Gesture::Resize && field.swaps_axes() {
        (dy, dx)
    } else {
        (dx, dy)
    };
    let mut next = field.clone();
    let adjust = |v: u32, d: i32, min: u32, max: u32| {
        (i64::from(v) + i64::from(d)).clamp(i64::from(min), i64::from(max.max(min))) as u32
    };
    match mode {
        Gesture::Move => {
            next.x = field.x.saturating_add(i64::from(dx));
            next.y = field.y.saturating_add(i64::from(dy));
        }
        Gesture::Resize if field.kind == Kind::Image || field.kind.is_shape() => {
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
            rotation: 0,
            stroke_dots: 1,
            filled: false,
            id: uuid::Uuid::new_v4(),
            picture: None,
            kind,
            text: "x".into(),
            x: 16,
            y: 8,
            w: 84,
            h: 84,
            module_size: Some(4),
            font_family: None,
            font_size: None,
        }
    }

    #[test]
    fn moving_layers_keeps_selection_and_saved_paint_order() {
        use crate::core::{self, Row, Template};

        let mut template = Template::default();
        for x in [11, 22, 33] {
            let mut item = field(Kind::Rectangle);
            item.x = x;
            template.elements.push(item);
        }
        let ids: Vec<_> = template.elements.iter().map(|item| item.id).collect();
        let selected = move_layer(&mut template.elements, 0, 2, 1);
        assert_eq!(template.elements[selected].id, ids[1]);
        let selected = move_layer(&mut template.elements, selected, 1, selected);
        assert_eq!(template.elements[selected].id, ids[1]);

        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("layers.json");
        crate::storage::export_template(&file, &template).unwrap();
        let saved = crate::storage::import_template(&file).unwrap();
        assert_eq!(
            saved.elements.iter().map(|f| f.id).collect::<Vec<_>>(),
            [ids[2], ids[1], ids[0]]
        );
        let row = Row::new();
        let svg = core::render_svg(&saved, &row, "1").unwrap();
        let positions = [33, 22, 11].map(|x| svg.find(&format!("translate({x} 8)")).unwrap());
        assert!(positions[0] < positions[1] && positions[1] < positions[2]);
        let items: Vec<_> = saved
            .elements
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let (bounds, art) = core::editor_art(f, &row, "1", i).unwrap();
                (bounds, Some(art))
            })
            .collect();
        assert_eq!(core::draft_svg(&saved.media, &items), svg);
    }

    #[test]
    fn alignment_follows_view_and_rotated_footprint() {
        let mut f = field(Kind::Image);
        (f.w, f.h, f.rotation) = (10, 20, 90);
        for (turns, expected) in [(0, (181, 0)), (1, (1, 0)), (2, (1, 86)), (3, (181, 86))] {
            align(&mut f, (200, 96), turns, 1, true, 2);
            align(&mut f, (200, 96), turns, 1, false, 0);
            assert_eq!((f.x, f.y), expected);
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
