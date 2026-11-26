use crate::core::{self, Field, Kind, Media, Sequence, Template};
use crate::{
    device::{Device, Model},
    editor::{self, Gesture},
    print_job::{PrintJob, PrintScope},
    storage,
    table::Table,
    workbook::{open_workbook, Workbook},
};
use dioxus::prelude::*;
use std::sync::Arc;
use tokio::sync::Mutex;

struct DataFile {
    name: String,
    sheets: Vec<String>,
    sheet: String,
    book: Option<Arc<std::sync::Mutex<Workbook>>>,
    table: Result<Arc<Table>, String>,
}

impl DataFile {
    fn open(path: &std::path::Path) -> Result<Self, String> {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("csv"))
        {
            if path.metadata().map_err(|_| "无法读取 CSV 文件。")?.len() > 2 * 1024 * 1024 {
                return Err("CSV 文件不得超过 2 MB。".into());
            }
            let text =
                std::fs::read_to_string(path).map_err(|_| "无法读取 CSV，请使用 UTF-8 编码。")?;
            return Ok(Self {
                name,
                sheets: vec![],
                sheet: String::new(),
                book: None,
                table: Ok(Arc::new(Table::from_csv(&text)?)),
            });
        }
        let mut book = open_workbook(path)?;
        let sheets = book.sheet_names().to_vec();
        let sheet = sheets[0].clone();
        let table = book.table(&sheet).map(Arc::new);
        if sheets.len() == 1 {
            table.as_ref().map_err(Clone::clone)?;
        }
        Ok(Self {
            name,
            sheets,
            sheet,
            book: Some(Arc::new(std::sync::Mutex::new(book))),
            table,
        })
    }
}

#[derive(Clone)]
enum FieldBinding {
    Column(usize),
    Fixed(String),
}

type Connection = Arc<Mutex<Device>>;
const UI_CSS: &str = include_str!("../assets/ui.css");
#[derive(Clone, Copy, PartialEq)]
struct CanvasDrag {
    pointer: i32,
    index: usize,
    id: uuid::Uuid,
    mode: Gesture,
    start: (f64, f64),
    dots_per_pixel: (f64, f64),
    turns: u8,
    delta: (i32, i32),
}

// Convert a screen-space motion back to the stock's X/Y axes.
fn stock_delta(turns: u8, dx: f64, dy: f64) -> (f64, f64) {
    match turns {
        1 => (dy, -dx),
        2 => (-dx, -dy),
        3 => (-dy, dx),
        _ => (dx, dy),
    }
}

impl CanvasDrag {
    fn delta_at(self, event: &PointerEvent) -> (i32, i32) {
        let point = event.client_coordinates();
        let (dx, dy) = stock_delta(self.turns, point.x - self.start.0, point.y - self.start.1);
        (
            (dx * self.dots_per_pixel.0).round() as i32,
            (dy * self.dots_per_pixel.1).round() as i32,
        )
    }
}

type CanvasItems = Memo<Vec<(Field, Option<String>)>>;

// Start from the visible geometry so an edit to a legacy template keeps its anchor.
fn edit_item(
    mut template: Signal<Template>,
    items: CanvasItems,
    index: usize,
    edit: impl FnOnce(&mut Field),
) {
    let Some(mut item) = items.read().get(index).map(|(item, _)| item.clone()) else {
        return;
    };
    edit(&mut item);
    if let Some(stored) = template.write().elements.get_mut(index) {
        *stored = item;
    }
}

fn edit_position(
    template: Signal<Template>,
    items: CanvasItems,
    index: usize,
    mode: Gesture,
    dx: i32,
    dy: i32,
) {
    if dx == 0 && dy == 0 {
        return;
    }
    edit_item(template, items, index, |item| {
        *item = editor::transform(item, mode, dx, dy);
    });
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Text => "文字",
        Kind::Image => "图片",
        Kind::Qr => "二维码",
        Kind::Barcode => "Code 128",
        Kind::Line => "直线",
        Kind::Rectangle => "矩形",
        Kind::Ellipse => "椭圆",
    }
}

#[component]
pub fn App() -> Element {
    let mut template = use_signal(Template::default);
    let mut library = use_signal(|| storage::templates().unwrap_or_default());
    let mut values = use_signal(core::Row::new);
    let variables = use_memo(move || core::variable_names(&template.read()));
    let mut recent = use_signal(|| storage::recent_templates().unwrap_or_default());
    let recent_tiles = use_memo(move || {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let library = library.read();
        recent
            .read()
            .iter()
            .filter_map(|name| {
                let i = library.iter().position(|t| &t.name == name)?;
                let t = &library[i];
                let sample = core::variable_names(t)
                    .into_iter()
                    .map(|k| (k, "1".into()))
                    .collect();
                let thumb = core::render_svg(t, &sample, "0001")
                    .ok()
                    .map(|svg| format!("data:image/svg+xml;base64,{}", STANDARD.encode(svg)));
                Some((i, name.clone(), t.media.width_mm, t.media.length_mm, thumb))
            })
            .collect::<Vec<_>>()
    });
    let media_library = use_signal(|| storage::media().unwrap_or_default());
    let mut csv_text = use_signal(String::new);
    let mut data_file = use_signal(|| None::<DataFile>);
    let mut bindings = use_signal(std::collections::BTreeMap::<String, FieldBinding>::new);
    let data_table = use_memo(move || {
        if let Some(file) = data_file.read().as_ref() {
            return file.table.clone().map(Some);
        }
        let text = csv_text.read();
        if text.trim().is_empty() {
            Ok(None)
        } else {
            Table::from_csv(&text).map(|table| Some(Arc::new(table)))
        }
    });
    let automatic = use_memo(move || match &*data_table.read() {
        Ok(Some(table)) => table.match_columns(&variables.read()),
        _ => Vec::new(),
    });
    let mut count = use_signal(|| 1usize);
    let mut start = use_signal(|| 1i64);
    let mut step = use_signal(|| 1i64);
    let mut pad = use_signal(|| 4usize);
    let mut view_turns = use_signal(|| 0_u8);
    let mut page = use_signal(|| 0usize);
    let mut field = use_signal(|| 0usize);
    let mut canvas_pixels = use_signal(|| (0.0, 0.0));
    let mut drag = use_signal(|| None::<CanvasDrag>);
    let mut notice = use_signal(|| {
        match storage::templates()
            .and_then(|_| storage::media())
            .map(|_| ())
        {
            Ok(()) => (String::new(), false),
            Err(e) => (e, true),
        }
    });
    let mut busy = use_signal(|| false);
    let mut device_panel = use_signal(|| false);
    let mut inspector_tab = use_signal(|| "规格");
    let mut usb_mode = use_signal(|| false);
    let mut usb_ports = use_signal(Vec::<(String, String)>::new);
    let mut usb_port = use_signal(String::new);
    let mut refresh_ports = move || match crate::printer::usb::ports() {
        Ok(ports) => {
            if !ports.iter().any(|(path, _)| path == &*usb_port.peek()) {
                usb_port.set(String::new());
            }
            usb_ports.set(ports);
        }
        Err(e) => {
            usb_ports.set(Vec::new());
            usb_port.set(String::new());
            notice.set((e, true));
        }
    };
    let mut selected_model = use_signal(Model::default);
    let mut select_item = move |i: usize| {
        field.set(i);
        inspector_tab.set("属性");
    };
    let mut dragged_layer = use_signal(|| None::<(i32, uuid::Uuid)>);
    let mut move_layer = move |from: usize, to: usize| {
        if !busy() && from != to {
            let mut draft = template.write();
            field.set(editor::move_layer(&mut draft.elements, from, to, field()));
        }
    };
    let mut remove_element = move |i: usize| {
        if busy() || i >= template.read().elements.len() {
            return;
        }
        template.write().elements.remove(i);
        if template.read().elements.is_empty() && inspector_tab() == "属性" {
            inspector_tab.set("规格");
        }
        let selected = field();
        if i <= selected {
            field.set(selected.saturating_sub(1));
        }
    };
    let mut connection = use_signal(|| None::<Connection>);
    let mut copies = use_signal(|| "1".to_owned());
    let mut confirm_print = use_signal(|| None::<PrintScope>);
    let mut range_label = use_signal(|| None::<crate::raster::RangeLabel>);
    let mut printing = use_signal(|| false);
    let mut stop_requested = use_signal(|| false);

    let mut bluetooth_loss = use_signal(|| None::<String>);
    use_resource(move || {
        let session = connection.read().clone();
        async move {
            let Some(session) = session else { return };
            let monitor = session.lock().await.take_disconnect_monitor();
            let Some(monitor) = monitor else { return };
            monitor.await;
            if !connection
                .peek()
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &session))
            {
                return;
            }

            connection.set(None);
            confirm_print.set(None);
            device_panel.set(true);
            if *printing.peek() {
                // The print task reports the loss together with its confirmed page count.
                stop_requested.set(true);
            } else {
                let model = selected_model.peek().name();
                bluetooth_loss.set(Some(format!(
                    "{model} 蓝牙连接已断开。请检查打印机电源和距离，然后重新连接。"
                )));
                if !notice.peek().1 {
                    notice.set((String::new(), false));
                }
            }
        }
    });

    let batch_rows = use_memo(move || {
        let data = data_table.read();
        if let Some(table) = data.as_ref().map_err(Clone::clone)? {
            let fields = variables.read();
            let mut columns = automatic();
            let choices = bindings.read();
            let mut fixed = core::Row::new();
            for (field, column) in fields.iter().zip(&mut columns) {
                if column.is_none() {
                    match choices.get(field) {
                        Some(FieldBinding::Column(index)) => *column = Some(*index),
                        Some(FieldBinding::Fixed(value)) => {
                            fixed.insert(field.clone(), value.clone());
                        }
                        None => {}
                    }
                }
            }
            return table.mapped_rows(&fields, &columns, &fixed);
        }
        let n = count();
        if !(1..=core::MAX_BATCH).contains(&n) {
            return Err(format!("记录数须为 1～{} 条。", core::MAX_BATCH));
        }
        let row = values.read();
        if row.is_empty() {
            Ok(vec![])
        } else {
            Ok(vec![row.clone(); n])
        }
    });
    let records = use_memo(move || {
        let rows = batch_rows.read();
        let rows = rows.as_ref().map_err(Clone::clone)?;
        let n = if rows.is_empty() { count() } else { rows.len() };
        if (1..=core::MAX_BATCH).contains(&n) {
            Ok(n)
        } else {
            Err(format!("记录数须为 1～{} 条。", core::MAX_BATCH))
        }
    });
    let using_table = data_file.read().is_some() || !csv_text.read().trim().is_empty();
    let table_record_count = data_table
        .read()
        .as_ref()
        .ok()
        .and_then(Option::as_ref)
        .map_or(0, |t| t.rows.len());
    let label_count = records().unwrap_or(0);
    let current_page = page().min(label_count.saturating_sub(1));
    let number = current_page + 1;
    let preview = use_memo(move || {
        let n = records()?;
        let index = page().min(n - 1);
        let rows = batch_rows.read();
        let empty = core::Row::new();
        let row = rows
            .as_ref()
            .map_err(Clone::clone)?
            .get(index)
            .unwrap_or(&empty);
        let seq = Sequence {
            start: start(),
            step: step(),
            pad: pad(),
        };
        seq.at(index)
            .and_then(|serial| core::render_svg(&template.read(), row, &serial))
            .map_err(|e| format!("第 {} 张：{e}", index + 1))
    });
    let validation = preview.read().as_ref().err().cloned();
    let invalid_page = validation.is_some();
    let is_busy = busy();
    let linked = connection.read().is_some();
    let model = selected_model();
    let model_name = model.name();
    let supported_stock = model.supported_stock();
    let delivery = if model != Model::P2 {
        "打印机已确认"
    } else {
        "已发送"
    };
    let (msg, error) = notice();
    let lost_link = bluetooth_loss();
    let show_link_error = lost_link.is_some();
    let error_message = lost_link.or_else(|| error.then(|| msg.clone()));

    let save_name = template.read().name.clone();
    let media = template.read().media.clone();
    let Media {
        width_mm,
        length_mm,
        printable_width_dots,
        printable_length_dots,
        ..
    } = media.clone();
    let w = printable_length_dots.clamp(1, core::MAX_MEDIA_LENGTH_DOTS);
    let h = printable_width_dots.clamp(1, core::MAX_MEDIA_WIDTH_DOTS);
    let turns = view_turns();
    let angle = u16::from(turns) * 90;
    let (view_w, view_h) = if turns % 2 == 1 { (h, w) } else { (w, h) };
    let view_ratio = f64::from(view_w) / f64::from(view_h);
    let canvas_items = use_memo(move || {
        let idx = page().min(records().unwrap_or(0).saturating_sub(1));
        let rows = batch_rows.read();
        let empty = core::Row::new();
        let row = rows
            .as_ref()
            .ok()
            .and_then(|rs| rs.get(idx))
            .unwrap_or(&empty);
        let serial = Sequence {
            start: start(),
            step: step(),
            pad: pad(),
        }
        .at(idx)
        .unwrap_or_else(|_| "{seq}".into());
        template
            .read()
            .elements
            .iter()
            .enumerate()
            .map(|(i, f)| match core::editor_art(f, row, &serial, i) {
                Ok((mut effective, svg)) => {
                    effective.text.clone_from(&f.text);
                    (effective, Some(svg))
                }
                Err(_) => (f.clone(), None),
            })
            .collect::<Vec<_>>()
    });
    let dot_preview = use_memo(move || {
        let draft = template.read();
        let model = selected_model();
        match &*preview.read() {
            Ok(svg) => model.preview_png(svg, &draft.media),
            Err(_) => {
                let svg = core::draft_svg(&draft.media, &canvas_items.read());
                model.preview_png(&svg, &draft.media)
            }
        }
    });
    let feed_shift = model.preview_feed_shift(&media);
    let outside = use_memo(move || {
        let draft = template.read();
        let media = &draft.media;
        let shift = selected_model().preview_feed_shift(media);
        canvas_items
            .read()
            .iter()
            .filter(|(item, art)| core::outside_media(media, item, art.as_deref(), shift))
            .map(|(item, _)| item.id)
            .collect::<std::collections::HashSet<_>>()
    });
    let overlap = use_memo(move || core::check_overlap(&canvas_items.read()).err());
    let chosen = canvas_items
        .read()
        .get(field())
        .map(|(item, _)| item.clone());
    let remove_corner = chosen.as_ref().map(|item| {
        let left = (item.x as f64 - f64::from(feed_shift)) / f64::from(w);
        let (width, height) = item.footprint();
        let right = left + f64::from(width) / f64::from(w);
        let top = item.y as f64 / f64::from(h);
        let bottom = top + f64::from(height) / f64::from(h);
        let (x, y) = match turns {
            1 => (1.0 - top, left),
            2 => (1.0 - left, 1.0 - bottom),
            3 => (bottom, 1.0 - right),
            _ => (right, top),
        };
        (100.0 * x, 100.0 * y)
    });
    let physical_caption = format!(
        "纸宽 {width_mm} × 走纸长 {length_mm} mm · {} dpi",
        media.dpi
    );
    let cut = media.effective_cut();
    // A schematic 1.5 mm corner is used only for the preview of unmeasured stock.
    let radius = match (cut.rounded, cut.regions_dots.is_empty(), cut.radius_dots) {
        (Some(true), true, 0) => 12,
        (Some(true), true, r) => r,
        _ => 0,
    };
    let paper_outline = format!(
        "border-radius:{}% / {}%",
        100.0 * f64::from(radius) / f64::from(w),
        100.0 * f64::from(radius) / f64::from(h)
    );
    let transport = if usb_mode() { "USB" } else { "蓝牙" };
    let layout_name = match cut.combined {
        Some(true) => "组合标签 · ",
        Some(false) => "单张标签 · ",
        None => "",
    };
    let corner_name = match cut.rounded {
        Some(true) => "圆角 · ",
        Some(false) => "直角 · ",
        None => "",
    };
    let supported_media = model.supports(&media);
    let media_choices = use_memo(move || {
        let model = selected_model();
        core::builtin_media()
            .iter()
            .filter(|(m, _, _)| *m == model)
            .map(|(_, m, _)| m.clone())
            .chain(media_library.read().iter().cloned())
            .collect::<Vec<_>>()
    });
    let media_selector = media_choices
        .read()
        .iter()
        .position(|m| *m == media)
        .map(|i| i.to_string())
        .unwrap_or_default();
    let template_selector = library
        .read()
        .iter()
        .position(|t| t.id == template.read().id)
        .map(|i| i.to_string())
        .unwrap_or_default();

    let mut load_template = move |item: Template, defaults: core::Row| {
        if !core::variable_names(&item).is_empty() {
            inspector_tab.set("数据");
        }
        bindings.write().clear();
        values.set(defaults);
        template.set(item);
        field.set(0);
        page.set(0);
    };
    let mut open_template = move |index: usize| {
        let Some(saved) = library.read().get(index).cloned() else {
            return;
        };
        match storage::record_template(&saved.name) {
            Ok(names) => recent.set(names),
            Err(e) => notice.set((e, true)),
        }
        load_template(saved, core::Row::new());
    };
    let copy_count = copies.read().parse::<usize>().unwrap_or(0);
    let total =
        PrintJob::new(PrintScope::Batch, label_count, copy_count).map_or(0, |job| job.len());
    let batch_valid = total > 0;
    let can_print = !is_busy && linked && label_count > 0 && supported_media;
    let mut start_drag = move |event: PointerEvent, index: usize, mode: Gesture| {
        use dioxus::html::input_data::MouseButton;
        event.stop_propagation();
        if busy()
            || drag.peek().is_some()
            || !event.is_primary()
            || event.trigger_button() != Some(MouseButton::Primary)
        {
            return;
        }
        let (width, height) = *canvas_pixels.peek();
        if width <= 0.0 || height <= 0.0 {
            return;
        }
        let point = event.client_coordinates();
        select_item(index);
        drag.set(Some(CanvasDrag {
            pointer: event.pointer_id(),
            index,
            id: template.read().elements[index].id,
            mode,
            start: (point.x, point.y),
            turns,
            dots_per_pixel: (f64::from(w) / width, f64::from(h) / height),
            delta: (0, 0),
        }));
    };
    let project_name = env!("CARGO_PKG_NAME");
    rsx! {
        style { "{UI_CSS}" }
        div { class: "app",
            onpointermove:move |event| {
                let active = *drag.peek();
                if let Some(mut active) = active.filter(|d| d.pointer == event.pointer_id()) {
                    let delta = active.delta_at(&event);
                    if active.delta != delta { active.delta = delta; drag.set(Some(active)); }
                }
            },
            onpointerup:move |event| {
                let active = *drag.peek();
                if let Some(active) = active.filter(|d| d.pointer == event.pointer_id()) {
                    drag.set(None);
                    if !busy() && template.read().elements.get(active.index).is_some_and(|f|f.id == active.id) {
                        let (dx,dy) = active.delta_at(&event);
                        edit_position(template,canvas_items,active.index,active.mode,dx,dy);
                    }
                }
            },
            onpointercancel:move |event| { if drag.peek().is_some_and(|d| d.pointer == event.pointer_id()) { drag.set(None); } },
            if let Some(active) = drag() {
                div { class:"drag-surface", "aria-hidden":"true",
                    style:if active.mode == Gesture::Resize { "cursor:nwse-resize" } else { "cursor:grabbing" },
                    onpointerleave:move |event| { if event.pointer_id() == active.pointer { drag.set(None); } }
                }
            }
            header { class: "bar",
                div { class: "brand", h1 { "{project_name}" } span { class: "muted", "{physical_caption}" } }
                div { class:"actions job-actions",
                    button { class:"device-switch", r#type:"button", "aria-expanded":device_panel(), "aria-controls":"device-settings",
                        onclick:move |_| device_panel.toggle(),
                        span {class:if linked {"device-dot connected"} else {"device-dot"}}
                        if linked { "{model_name} · {transport}" } else { "连接打印机" }
                    }
                    if printing() {
                        button { r#type:"button", disabled:stop_requested(), onclick:move |_| stop_requested.set(true),
                            if stop_requested() { "正在停止…" } else { "停止后续打印" }
                        }
                    } else {
                        button { r#type:"button", class:"primary", disabled:!can_print || invalid_page || !batch_valid, onclick:move |_| confirm_print.set(Some(PrintScope::Batch)),
                            if batch_valid { "打印 {total} 张" } else { "批量打印" }
                        }
                    }
                }
                if device_panel() {
                    section { id:"device-settings",class:"device-settings","aria-label":"设备连接",
                    if !linked {
                    div { class:"device-choice", select { class:"printer-model", "aria-label":"打印机型号", disabled:is_busy || linked, value:"{model_name}", onchange: move |e| {
                        if busy() || connection.read().is_some() { return; }
                        let next = match e.value().as_str() {
                            "P2" => Model::P2, "N1" => Model::N1, _ => return,
                        };
                        let builtin = core::builtin_media().iter().any(|(_, m, _)| *m == template.read().media);
                        if builtin { template.write().media = Media::for_model(next); }
                        selected_model.set(next);
                        if next == Model::N1 && usb_mode() { refresh_ports(); }
                        page.set(0);
                    },
                        option { value:"N1", "精臣 N1" }
                        option { value:"P2", "德佟 P2" }
                    } }
                        div { class:"transport-picker", role:"group","aria-label":"连接方式",
                            for (usb,label) in [(false,"蓝牙"),(true,"USB")] {
                                button {r#type:"button",disabled:is_busy,"aria-pressed":usb_mode()==usb,onclick:move |_| {
                                    usb_mode.set(usb);
                                    if usb && model==Model::N1 { refresh_ports(); }
                                },"{label}"}
                            }
                        }
                        if usb_mode() && model == Model::N1 {
                            div { class:"device-choice port-choice", select { class:"usb-port", title:"{usb_port}", "aria-label":"USB 串口", disabled:is_busy || linked, value:"{usb_port}", onchange:move |e| usb_port.set(e.value()),
                                option { value:"", if usb_ports.read().is_empty() {"未发现 USB 串口"} else {"选择打印机端口"} }
                                for (path,label) in usb_ports.read().iter() {
                                    option { key:"{path}", value:"{path}", "{label}" }
                                }
                            }
                            }
                            button { r#type:"button", disabled:is_busy, onclick:move |_| refresh_ports(), "刷新端口" }
                        }
                    }
                    if linked { span { class:"status good", "{model_name} · {transport} 已连接" } }
                    button { id:"connect-printer", r#type: "button", disabled: is_busy || (!linked && model == Model::N1 && usb_mode() && usb_port.read().is_empty()), onclick: move |_| {
                        if busy() { return; }
                        busy.set(true);
                        let previous = connection.write().take();
                        if let Some(printer) = previous {
                            notice.set(("正在断开打印机…".into(), false));
                            spawn(async move {
                                let result = printer.lock().await.disconnect().await;
                                busy.set(false);
                                notice.set(match result { Ok(())=>(format!("{model_name} · {transport} 已断开。"),false), Err(e)=>(e,true) });
                            });
                        } else {
                            notice.set((format!("正在搜索并识别 {model_name}…"), false));
                            spawn(async move {
                                let port = (model == Model::N1 && usb_mode()).then(|| usb_port());
                                match model.connect(usb_mode(),port).await {
                                    Ok(mut printer) => {
                                        let mut message=format!("已连接：{}",printer.name());
                                        if let Device::N1(n1) = &mut printer {
                                            match n1.read_stock_barcode().await {
                                                Some(code) => {
                                                    if let Some((_,media,_))=core::builtin_media().iter().find(|(m,_,codes)| *m==Model::N1 && codes.contains(&code.as_str())) {
                                                        template.write().media=media.clone();
                                                        message=format!("已识别耗材：{}（{code}）",media.name);
                                                    } else {message=format!("耗材编码 {code} 未匹配规格，请核对标签规格。");}
                                                }
                                                None=>message.push_str("。未读到耗材编码，请核对标签规格。"),
                                            }
                                        }
                                        notice.set((message,false));
                                        connection.set(Some(Arc::new(Mutex::new(printer))));
                                        device_panel.set(false);
                                    }
                                    Err(e)=>notice.set((e,true)),
                                }
                                busy.set(false);
                            });
                        }
                    }, if linked { "断开连接" } else { "连接" } }
                    }
                }
                if !error && !msg.is_empty() {
                    span { class:"toolbar-notice muted", role:"status", "{msg}" }
                }
            }
            main { class: "workspace",
                aside { class:"palette",
                    h2 { "插入组件" }
                    div {class:"palette-grid",
                                for (kind,text,w,h,label,icon) in [(Kind::Text,"新文字",136,24,"文字","M3 5h13M9.5 5v14M14 11h7m-3.5 0v8"),(Kind::Qr,"N1",86,86,"二维码","M3 3h7v7H3z M14 3h7v7h-7z M3 14h7v7H3z M14 14h3v3h4v4h-7z"),(Kind::Barcode,"0001",190,40,"条形码","M3 4v16m3-16v16m4-16v16m2-16v16m4-16v16m4-16v16"),(Kind::Text,"{seq}",48,24,"序号","M7 3L5 21M16 3l-2 18M3 9h18M2 15h18"),(Kind::Line,"",80,2,"直线","M4 20L20 4"),(Kind::Rectangle,"",80,40,"矩形","M3 5h18v14H3z"),(Kind::Ellipse,"",48,48,"椭圆","M21 12a9 7 0 1 1-18 0 9 7 0 1 1 18 0z")] {
                                    button { r#type: "button", disabled: is_busy, onclick: move |_| {
                                        let m = template.read().media.clone();
                                        let (across, down) = m.canvas_size();
                                        let i = template.read().elements.len();
                                        if i >= core::MAX_FIELDS { notice.set((format!("每个模板最多放置 {} 个元素。", core::MAX_FIELDS), true)); return; }
                                        let w = w.min(across).max(1);
                                        let h = h.min(down).max(1);
                                        let offset = 4 + 8 * (i as u32 % 8);
                                        let x = offset.min(across.saturating_sub(w));
                                        let y = offset.min(down.saturating_sub(h));
                                        template.write().elements.push(Field {rotation:0,stroke_dots:1,filled:false,id:uuid::Uuid::new_v4(), picture:None, kind, text: text.into(), x:x.into(), y:y.into(), w, h, font_family:None,font_size:(kind == Kind::Text).then_some(22), module_size:matches!(kind, Kind::Barcode | Kind::Qr).then_some(2)});
                                        select_item(i);
                                    }, svg {view_box:"0 0 24 24","aria-hidden":"true",path { d:icon }} span {"{label}"} }
                                }
                                button { r#type:"button", disabled:is_busy, onclick:move |_| {
                                    if busy() {return;}
                                    if template.read().elements.len() >= core::MAX_FIELDS {
                                        notice.set((format!("每个模板最多放置 {} 个元素。", core::MAX_FIELDS),true));return;
                                    }
                                    busy.set(true);
                                    let stock=template.read().media.clone();
                                    spawn(async move {
                                        if let Some(file)=rfd::AsyncFileDialog::new().add_filter("图片", &["png","jpg","jpeg","bmp"]).pick_file().await {
                                            let path=file.path().to_owned();
                                            match tokio::task::spawn_blocking(move ||crate::picture::import(&path,&stock)).await {
                                                Ok(Ok(item))=>{let index=template.read().elements.len(); template.write().elements.push(item); select_item(index);}
                                                Ok(Err(reason))=>notice.set((reason,true)),
                                                Err(_)=>notice.set(("读取图片失败。".into(),true)),
                                            }
                                        }
                                        busy.set(false);
                                    });
                                }, svg {view_box:"0 0 24 24","aria-hidden":"true",path {d:"M3 4h18v16H3z M3 16l6-6 5 5 3-3 4 4 M15 8h1"}} span {"图片"} }
                    }
                    if !template.read().elements.is_empty() {
                        div {class:"section-row layer-heading",
                            h2 { "画布元素" }
                            if template.read().elements.len() > 1 {
                                div {class:"actions", role:"group", "aria-label":"元素层级",
                                    button {r#type:"button", title:"上移一层", "aria-label":"上移一层",
                                        disabled:is_busy || field() + 1 >= template.read().elements.len(),
                                        onclick:move |_| move_layer(field(),field()+1), "↑"}
                                    button {r#type:"button", title:"下移一层", "aria-label":"下移一层",
                                        disabled:is_busy || field()==0,
                                        onclick:move |_| move_layer(field(),field()-1), "↓"}
                                }
                            }
                        }
                        ol {class:"elements", "aria-label":"画布元素，最前层在上",
                            "data-reordering":dragged_layer.read().is_some(),
                            onpointerleave:move |_| dragged_layer.set(None),
                            onpointercancel:move |_| dragged_layer.set(None),
                            onpointerup:move |_| dragged_layer.set(None),
                            for (i, object) in template.read().elements.iter().enumerate().rev() {
                                {
                                    let id = object.id;
                                    let kind = kind_name(object.kind);
                                    let content = object.text.trim();
                                    let name = if content.is_empty() { kind } else { content };
                                    rsx! {
                                        li { key:"{id}", class:"element-entry",
                                            onpointerup:move |event| {
                                                let source = *dragged_layer.peek();
                                                if let Some((_,id))=source.filter(|(pointer,_)| *pointer == event.pointer_id()) {
                                                    let from=template.peek().elements.iter().position(|f| f.id==id);
                                                    if let Some(from)=from { move_layer(from,i); }
                                                }
                                            },
                                            button { class:"element-select", r#type:"button", disabled:is_busy, "aria-pressed":field() == i,
                                                title:"{name}", onclick:move |_| select_item(i),
                                                onpointerdown:move |event| {
                                                    if event.is_primary() && event.trigger_button() == Some(dioxus::html::input_data::MouseButton::Primary) {
                                                        dragged_layer.set(Some((event.pointer_id(),id)));
                                                    }
                                                },
                                                if !content.is_empty() { span {class:"layer-kind","{kind}"} }
                                                span {class:"layer-name","{name}"}
                                            }
                                            button { class:"element-remove danger", r#type:"button", disabled:is_busy,
                                                "aria-label":"删除{name}", title:"删除元素",
                                                onclick:move |_| remove_element(i), "×"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if !outside.read().is_empty() {
                                button { r#type:"button", class:"danger", disabled:is_busy || outside.read().is_empty(),
                                    onclick:move |_| {
                                        if busy() {return;}
                                        let ids=outside();
                                        let mut draft=template.write();
                                        let selected=draft.elements.get(field()).map(|item|item.id);
                                        draft.elements.retain(|item|!ids.contains(&item.id));
                                        let next=selected.and_then(|id| draft.elements.iter().position(|item|item.id == id))
                                            .unwrap_or_else(||field().min(draft.elements.len().saturating_sub(1)));
                                        field.set(next);
                                        if draft.elements.is_empty() && inspector_tab()=="属性" {inspector_tab.set("规格");}
                                    }, "删除越界元素"
                                }
                    }
                }
                section { class: "editor", "aria-label": "标签设计",
                    div { class: "section-row",
                        h2 { class:"document-title", "{save_name}" }
                        div { class:"actions",
                            button { r#type:"button", title:"顺时针旋转编辑视图 90°", onclick:move |_| view_turns.set((turns + 1) % 4), "旋转视图 · {angle}°" }
                        button { r#type:"button", disabled:!can_print || invalid_page, onclick:move |_| confirm_print.set(Some(PrintScope::Current(current_page))), "试打当前页" }
                        }
                    }
                    div { class: "preview",
                        div { class:"dot-viewport",
                            div { class:"label-sheet",
                                style:"--sheet-ratio:{view_ratio};aspect-ratio:{view_w}/{view_h}",
                                div { id:"label-canvas", class:"label-canvas", style:"{paper_outline};position:absolute;left:50%;top:50%;width:{100.0*f64::from(w)/f64::from(view_w)}%;height:{100.0*f64::from(h)/f64::from(view_h)}%;transform:translate(-50%,-50%) rotate({angle}deg)", "aria-label":"标签画布",
                                    "data-w":"{w}", "data-h":"{h}", "data-disabled":"{is_busy}",
                                    onresize:move |event| {
                                        if let Ok(size) = event.get_content_box_size() {
                                            canvas_pixels.set((size.width,size.height));
                                        }
                                    },
                                    if let Ok(src) = &*dot_preview.read() {
                                        img { class:"dot-print", src:"{src}", draggable:false, alt:"标签打印效果" }
                                    }
                                    for (i,(item,valid)) in canvas_items.read().iter().map(|(item, art)| {
                                        let bounds = match drag().filter(|d| d.id == item.id) {
                                            Some(d) => editor::transform(item, d.mode, d.delta.0, d.delta.1),
                                            None => item.clone(),
                                        };
                                        (bounds, art.is_some())
                                    }).enumerate() {
                                        button {
                                            key:"{item.id}", r#type:"button", class:"canvas-element",
                                            "aria-label":"画布元素 {kind_name(item.kind)} {item.text}",
                                            "aria-pressed":field() == i, "data-invalid":!valid,
                                            disabled:is_busy,
                                            "data-kind": match item.kind { Kind::Qr=>"qr", Kind::Image=>"image", _=>"other" }, "data-module":item.module_size.unwrap_or(0),
                                            "data-index":"{i}", "data-x":"{item.x}", "data-y":"{item.y}", "data-w":"{item.w}", "data-h":"{item.h}",
                                            style:"left:{100.0 * (item.x as f64 - f64::from(feed_shift)) / f64::from(w)}%;top:{100.0 * item.y as f64 / f64::from(h)}%;width:{100.0 * f64::from(item.footprint().0.max(1)) / f64::from(w)}%;height:{100.0 * f64::from(item.footprint().1.max(1)) / f64::from(h)}%",
                                            onpointerdown:move |event| start_drag(event,i,Gesture::Move),
                                            onclick:move |_| select_item(i),
                                            onkeydown:move |event| {
                                                let unit: i32=if event.modifiers().shift() {10} else {1};
                                                let (sx,sy)=match event.key() {Key::ArrowLeft=>(-unit,0),Key::ArrowRight=>(unit,0),Key::ArrowUp=>(0,-unit),Key::ArrowDown=>(0,unit),_=>return};
                                                let (dx,dy)=stock_delta(turns,f64::from(sx),f64::from(sy));
                                                let (mut dx,mut dy)=(dx as i32,dy as i32);
                                                let mode = if event.modifiers().alt() {
                                                    if item.kind == Kind::Text { return; }
                                                    let columns = i32::try_from((item.w / item.module_size.unwrap_or(1).max(1)).max(1)).unwrap_or(i32::MAX);
                                                    if item.kind == Kind::Qr || (item.kind == Kind::Barcode && !item.swaps_axes()) { dx = dx.saturating_mul(columns); }
                                                    if item.kind == Kind::Qr || (item.kind == Kind::Barcode && item.swaps_axes()) { dy = dy.saturating_mul(columns); }
                                                    Gesture::Resize
                                                } else { Gesture::Move };
                                                event.prevent_default();
                                                field.set(i);
                                                edit_position(template,canvas_items,i,mode,dx,dy);
                                            },
                                            if field() == i && item.kind != Kind::Text {
                                                span { class:"resize-handle", onpointerdown:move |event| start_drag(event,i,Gesture::Resize), title:"拖动缩放", "aria-hidden":"true" }
                                            }
                                        }
                                    }
                                }
                                if let (Some(item), Some((x,y))) = (&chosen, remove_corner) {
                                    button { class:"element-remove canvas-remove danger", hidden:drag().is_some(), r#type:"button", disabled:is_busy,
                                        style:"--corner-x:{x}%;--corner-y:{y}%",
                                        "aria-label":"删除选中的{kind_name(item.kind)}", title:"删除元素",
                                        onclick:move |_| remove_element(field()), "×"
                                    }
                                }
                            }
                        }
                    }
                    div { class:"preview-footer",
                        div { class: "pager",
                            button { r#type: "button", disabled: current_page == 0 || is_busy, onclick: move |_| page.set(current_page.saturating_sub(1)), "上一张" }
                            if label_count > 0 { span { "{number} / {label_count}" } } else { span { "0 / 0" } }
                            button { r#type: "button", disabled: current_page + 1 >= label_count || is_busy, onclick: move |_| page.set(current_page + 1), "下一张" }
                        }
                        div { class:"print-summary",
                            label { "每条份数", input { r#type:"number", min:1, max:core::MAX_BATCH, disabled:is_busy, value:"{copies}", oninput:move |e| copies.set(e.value()) } }
                            if label_count > 0 {
                                if batch_valid {
                                    p { class:"muted", "{label_count} 条 × {copy_count} 份，共 {total} 张" }
                                } else {
                                    p { class:"error inline", "请调整每条份数，单批总量须为 1～500 张。" }
                                }
                            }
                        }
                    }
                    if !supported_media {
                        p { class: "muted", "当前规格仅可预览。{model_name} 支持 {supported_stock}。" }
                    }
                    if template.read().elements.is_empty() {
                        p { class:"muted", "从组件栏选择要插入的内容。" }
                    } else if let Some(reason) = validation.or_else(|| dot_preview.read().as_ref().err().cloned()) {
                        p { class: "error inline", "{reason}" }
                    }
                    if !outside.read().is_empty() { p {class:"muted", "有 {outside.read().len()} 个元素超出可打印区域，实物可能印不全。"} }
                    if let Some(reason)=overlap() {p {class:"muted", "{reason}"}}
                }
                aside { class: "inspector",
                    nav { class:"inspector-nav","aria-label":"工作台面板",
                        for tab in ["规格","模板","数据","属性"] {
                            button { r#type:"button",disabled:tab=="属性" && chosen.is_none(),"aria-pressed":inspector_tab()==tab,onclick:move |_| inspector_tab.set(tab),"{tab}" }
                        }
                    }
                    if inspector_tab()=="属性" {
                        if let Some(object) = chosen {
                            h2 { "{kind_name(object.kind)}属性" }
                            if let Some(picture) = &object.picture {
                                div { class:"image-settings",
                                    label { "黑白阈值 {picture.threshold}", input { r#type:"range", min:0,max:255,value:"{picture.threshold}",disabled:is_busy,
                                        oninput:move |e| if let Ok(value)=e.value().parse::<u8>() {
                                            edit_item(template,canvas_items,field(),|f| if let Some(p)=&mut f.picture {p.threshold=value;});
                                        }
                                    } }
                                    label { input { r#type:"checkbox", checked:picture.dither,disabled:is_busy,onchange:move |e| {
                                        edit_item(template,canvas_items,field(),|f| if let Some(p)=&mut f.picture {p.dither=e.checked();});
                                    } } "抖动" }
                                    label { input { r#type:"checkbox", checked:picture.invert,disabled:is_busy,onchange:move |e| {
                                        edit_item(template,canvas_items,field(),|f| if let Some(p)=&mut f.picture {p.invert=e.checked();});
                                    } } "反色" }
                                }
                            } else if matches!(object.kind,Kind::Rectangle | Kind::Ellipse) {
                                div { class:"image-settings",
                                    label { input {r#type:"checkbox",checked:object.filled,disabled:is_busy,onchange:move |e|
                                        edit_item(template,canvas_items,field(),|item| item.filled=e.checked()) } "实心" }
                                    if !object.filled {
                                        label { "边线 / 点",input {r#type:"number",min:1,value:"{object.stroke_dots}",disabled:is_busy,oninput:move |e| {
                                            if let Ok(n @ 1..) = e.value().parse() {edit_item(template,canvas_items,field(),|item|item.stroke_dots=n);}
                                        } } }
                                    }
                                }
                            } else if object.kind != Kind::Line {
                                label { "内容", input { disabled: is_busy, value: "{object.text}", oninput: move |e| {
                                    edit_item(template,canvas_items,field(),|item| item.text = e.value());
                                } } }
                            }
                            div { class:"coords numeric",
                                for (name, value, horizontal) in [("X", object.x, true), ("Y", object.y, false)] {
                                    label { "{name} / 点", input { r#type:"number", disabled:is_busy, value:"{value}", oninput:move |e| {
                                        if let Ok(v) = e.value().parse() {
                                            edit_item(template,canvas_items,field(),|item| if horizontal { item.x = v; } else { item.y = v; });
                                        }
                                    } } }
                                }
                                label { "元素旋转", select { disabled:is_busy, value:"{object.rotation}", onchange:move |e| {
                                    if let Ok(angle @ (0 | 90 | 180 | 270)) = e.value().parse::<u16>() {
                                        edit_item(template,canvas_items,field(),|item| item.rotation = angle);
                                    }
                                },
                                    for angle in [0,90,180,270] { option { value:"{angle}", "{angle}°" } }
                                } }
                                if object.kind == Kind::Text {
                                    label { "字体", select { disabled:is_busy, value:object.font_family.as_deref().unwrap_or(""),
                                        onchange:move |e| {
                                            let name=e.value();
                                            edit_item(template,canvas_items,field(),|item| item.font_family=(!name.is_empty()).then_some(name));
                                        },
                                        option { value:"","默认字体" }
                                        if let Some(missing)=object.font_family.as_ref().filter(|name| crate::raster::font_families().binary_search(name).is_err()) {
                                            option {value:"{missing}","{missing}（本机缺失）"}
                                        }
                                        for name in crate::raster::font_families() {option {key:"{name}",value:"{name}","{name}"}}
                                    } }
                                    label { "字号 / 点", input { r#type:"number", min:"1", max:core::MAX_MEDIA_WIDTH_DOTS, disabled:is_busy,
                                        value:object.text_size(), oninput:move |e| {
                                            if let Ok(size @ 1..=core::MAX_MEDIA_WIDTH_DOTS) = e.value().parse::<u32>() {
                                                edit_item(template,canvas_items,field(),|item| item.font_size = Some(size));
                                            }
                                        }
                                    } }
                                } else if object.kind == Kind::Image || object.kind.is_shape() {
                                    for (name,value,is_width) in [(if object.kind==Kind::Line {"长度 / 点"} else {"宽 / 点"},object.w,true),(if object.kind==Kind::Line {"线宽 / 点"} else {"高 / 点"},object.h,false)] {
                                        label { "{name}", input { r#type:"number", min:1, max:if is_width {core::MAX_MEDIA_LENGTH_DOTS} else {core::MAX_MEDIA_WIDTH_DOTS},
                                            disabled:is_busy,value:"{value}", oninput:move |e| {
                                                let max=if is_width {core::MAX_MEDIA_LENGTH_DOTS} else {core::MAX_MEDIA_WIDTH_DOTS};
                                                if let Ok(n)=e.value().parse::<u32>() {if (1..=max).contains(&n) {
                                                    edit_item(template,canvas_items,field(),|f| if is_width {f.w=n;} else {f.h=n;});
                                                }}
                                            }
                                        } }
                                    }
                                } else {
                                    label { "模块 / 点", input { r#type:"number", min:"1", max:core::MAX_MEDIA_WIDTH_DOTS, disabled:is_busy,
                                        value:object.module_size.unwrap_or(2), oninput:move |e| {
                                            if let Ok(size @ 1..=core::MAX_MEDIA_WIDTH_DOTS) = e.value().parse::<u32>() {
                                                edit_item(template,canvas_items,field(),|item| item.module_size = Some(size));
                                            }
                                        }
                                    } }
                                }
                                if object.kind == Kind::Barcode {
                                    label { "条高 / 点", input { r#type:"number", min:"1", disabled:is_busy,
                                        value:object.h, oninput:move |e| {
                                            if let Ok(v @ 1..) = e.value().parse::<u32>() {
                                                edit_item(template,canvas_items,field(),|item| item.h = v);
                                            }
                                        }
                                    } }
                                }
                            }
                            div { class:"actions", role:"group", "aria-label":"相对标签对齐",
                                for (label,horizontal,edge) in [("靠左",true,0),("水平居中",true,1),("靠右",true,2),("靠上",false,0),("垂直居中",false,1),("靠下",false,2)] {
                                    button { r#type:"button",disabled:is_busy,onclick:move |_| {
                                        edit_item(template,canvas_items,field(),|item|editor::align(item,(w,h),turns,feed_shift,horizontal,edge));
                                    },"{label}" }
                                }
                            }
                            button { r#type:"button",disabled:is_busy || template.read().elements.len()>=core::MAX_FIELDS,onclick:move |_| {
                                let mut draft=template.write();
                                let Some(mut duplicate)=draft.elements.get(field()).cloned() else {return;};
                                duplicate.id=uuid::Uuid::new_v4();
                                duplicate.x=duplicate.x.saturating_add(4);
                                duplicate.y=duplicate.y.saturating_add(4);
                                field.set(draft.elements.len());
                                draft.elements.push(duplicate);
                            },"复制所选元素" }
                            span { class:"muted", "占用 {object.footprint().0} × {object.footprint().1} 点" }
                        }

                    }
                    if inspector_tab()=="规格" {
                    section { class: "region",
                        label { "规格（纸宽 × 走纸长）", select { disabled: is_busy, value: "{media_selector}", onchange: move |e| {
                            if let Ok(i) = e.value().parse::<usize>() {
                                if let Some(stock) = media_choices.read().get(i).cloned() {
                                    template.write().media = stock;
                                    page.set(0);
                                }
                            }
                        },
                            if media_selector.is_empty() {
                                option { value:"", "{media.name}（模板规格）" }
                            }
                            for (i, entry) in media_choices.read().iter().enumerate() {
                                option { key:"{i}", value:"{i}", "{entry.name}" }
                            }
                        } }
                        span {class:"muted","{layout_name}{corner_name}点阵 {printable_length_dots} × {printable_width_dots} 点"}
                            if supported_media && (model == Model::N1 || (model == Model::P2 && (width_mm,length_mm,w,h)==(40,30,240,320))) {
                                button { r#type:"button", disabled:is_busy, onclick:move |_| {
                                    match crate::raster::range_label(model, &template.read().media, usb_mode()) {
                                        Ok(label) => { range_label.set(Some(label)); confirm_print.set(Some(PrintScope::Range)); }
                                        Err(reason) => notice.set((reason, true))
                                    }
                                }, "标定四边范围" }
                            }


                    }
                    }
                    if inspector_tab()=="模板" {
                    section { class: "region", "aria-label":"模板库",
                        if model == Model::N1 {
                            span {class:"muted","12 × 25 mm 亚银纸"}
                            div {class:"actions",for preset in storage::presets() {
                                button {key:"{preset.template.name}",r#type:"button",disabled:is_busy,onclick:move |_| {
                                    load_template(preset.template.duplicate(),preset.values.clone());
                                },"{preset.template.name}"}
                            } }
                        }
                        if !recent_tiles.read().is_empty() {
                            span { class:"muted","最近使用" }
                            div { class:"recent-templates",
                                for (index,name,width,length,thumb) in recent_tiles() {
                                    button { key:"{name}", r#type:"button", disabled:is_busy, onclick:move |_| open_template(index),
                                        if let Some(src) = thumb { img {src,alt:""} }
                                        span { "{name}" } small { "{width} × {length} mm" }
                                    }
                                }
                            }
                        }
                        if !library.read().is_empty() {
                            label { "本机模板", select { disabled: is_busy, value: "{template_selector}", onchange: move |e| {
                                if let Ok(i) = e.value().parse::<usize>() {
                                    open_template(i);
                                }
                            },
                                option { value: "", selected:template_selector.is_empty(), "选择模板…" }
                                for (i, entry) in library.read().iter().enumerate() {
                                    option { key:"{entry.id}", value:"{i}", selected:entry.id==template.read().id, "{entry.name}" }
                                }
                            } }
                        }
                        label { "模板名称", input { id:"template-name", disabled: is_busy, value: "{save_name}", oninput: move |e| template.write().name=e.value() } }
                        div { class: "actions",
                            button { r#type: "button", disabled: is_busy, onclick: move |_| {
                                let mut item = template();
                                item.name = item.name.trim().to_owned();
                                if item.name.is_empty() { notice.set(("请输入模板名称。".to_string(), true)); return; }
                                if let Err(e)=core::validate_template(&item) { notice.set((e,true)); return; }
                                let mut entries = library();
                                if entries.iter().any(|t| t.id!=item.id && t.name==item.name) {notice.set(("模板名称已被使用，请更换名称。".into(),true));return;}
                                if let Some(old) = entries.iter_mut().find(|existing| existing.id == item.id) { *old = item.clone(); }
                                else { entries.push(item.clone()); }
                                match storage::save_templates(&entries) {
                                    Ok(()) => { library.set(entries); template.set(item); notice.set(("模板已保存在本机。".to_string(), false)); }
                                    Err(e) => notice.set((e, true)),
                                }
                            }, "保存模板" }
                            button { r#type: "button", disabled: is_busy, onclick: move |_| {
                                let mut draft = template.read().duplicate();
                                draft.name = storage::available_name(&format!("{} 副本", draft.name.trim()), &library.read());
                                template.set(draft);
                                notice.set(("正在编辑副本，保存后加入本机模板。".into(),false));
                            }, "复制为新模板" }
                            button { r#type: "button", disabled: is_busy, onclick: move |_| {
                                let media = template.read().media.clone();
                                let draft = Template {id:uuid::Uuid::new_v4(),name:storage::available_name("未命名模板", &library.read()), media, elements:Vec::new()};
                                values.set(core::Row::new()); template.set(draft); field.set(0); page.set(0);
                                notice.set(("已新建空白模板。".to_string(), false));
                            }, "新建空白" }
                            button { r#type:"button", disabled:is_busy, onclick:move |_| {
                                if busy() {return;}
                                let item=template();
                                if let Err(e)=core::validate_template(&item) { notice.set((e,true)); return; }
                                busy.set(true);
                                spawn(async move {
                                    if let Some(path)=rfd::AsyncFileDialog::new().set_file_name("N1-template.json").add_filter("N1 标签模板", &["json"]).save_file().await {
                                        notice.set(match storage::export_template(path.path(),&item) {
                                            Ok(())=>("已导出模板。".into(),false),Err(e)=>(e,true)
                                        });
                                    }
                                    busy.set(false);
                                });
                            }, "导出" }
                            button { r#type:"button", disabled:is_busy, onclick:move |_| {
                                if busy() {return;}
                                busy.set(true);
                                spawn(async move {
                                    if let Some(path)=rfd::AsyncFileDialog::new().add_filter("N1 标签模板", &["json"]).pick_file().await {
                                        match storage::import_template(path.path()) {
                                            Ok(item)=>{load_template(item,core::Row::new());notice.set(("已导入模板，保存后才会加入本机模板列表。".into(),false));}
                                            Err(e)=>notice.set((e,true)),
                                        }
                                    }
                                    busy.set(false);
                                });
                            }, "导入模板" }

                        }

                    }
                    }
                    if inspector_tab()=="数据" {
                    if !variables.read().is_empty() && !using_table {
                        section { class:"region",
                            h2 { "填写模板" }
                            for key in variables() {
                                label { key:"{key}", "{key}", input {disabled:is_busy, value:values.read().get(&key).cloned().unwrap_or_default(),
                                    oninput:move |e| {values.write().insert(key.clone(),e.value());}
                                } }
                            }
                        }
                    }
                    section { class: "region",
                        h2 { "批量与递增" }
                        div { class:"actions",
                            button { r#type:"button", disabled:is_busy, onclick:move |_| {
                                if busy() { return; }
                                busy.set(true);
                                spawn(async move {
                                    if let Some(file) = rfd::AsyncFileDialog::new()
                                        .add_filter("Excel / CSV", &["xlsx", "xls", "xlsm", "csv"]).pick_file().await {
                                        let path = file.path().to_owned();
                                        match tokio::task::spawn_blocking(move || DataFile::open(&path)).await {
                                            Ok(Ok(file)) => {
                                                data_file.set(Some(file));
                                                csv_text.set(String::new());
                                                bindings.write().clear();
                                                page.set(0);
                                                notice.set((String::new(), false));
                                            }
                                            Ok(Err(reason)) => notice.set((reason, true)),
                                            Err(_) => notice.set(("无法读取表格文件。".into(), true)),
                                        }
                                    }
                                    busy.set(false);
                                });
                            }, "导入表格" }
                            if using_table {
                                button { r#type:"button", disabled:is_busy, onclick:move |_| {
                                    data_file.set(None); csv_text.set(String::new()); bindings.write().clear(); page.set(0);
                                }, "移除数据" }
                            }
                        }
                        if let Some(file) = data_file.read().as_ref() {
                            span { class:"muted toolbar-notice", "{file.name}" }
                            if file.sheets.len() > 1 {
                                label { "工作表", select { value:"{file.sheet}", disabled:is_busy, onchange:move |event| {
                                    let sheet = event.value();
                                    let Some(book) = data_file.read().as_ref().and_then(|f| f.book.clone()) else {return;};
                                    busy.set(true);
                                    spawn(async move {
                                        let name = sheet.clone();
                                        let result = tokio::task::spawn_blocking(move || {
                                            book.lock().map_err(|_| "请重新导入工作簿。".to_owned())?.table(&name).map(Arc::new)
                                        }).await.unwrap_or_else(|_| Err("无法读取工作表。".into()));
                                        if let Some(file) = data_file.write().as_mut() {
                                            file.sheet = sheet;
                                            file.table = result;
                                        }
                                        bindings.write().clear();
                                        page.set(0);
                                        busy.set(false);
                                    });
                                },
                                    for sheet in &file.sheets { option { key:"{sheet}", value:"{sheet}", "{sheet}" } }
                                } }
                            }
                        } else {
                            details {
                                summary { "粘贴 CSV" }
                                label { "CSV 数据", textarea { rows:3, disabled:is_busy, spellcheck:false, value:"{csv_text}", placeholder:"名称,编号",
                                    oninput:move |e| { csv_text.set(e.value()); bindings.write().clear(); page.set(0); }
                                } }
                            }
                        }
                        if let Ok(Some(table)) = &*data_table.read() {
                            for (key, col) in variables().into_iter().zip(automatic()) {
                                if col.is_none() {
                                    {
                                        let selected = match bindings.read().get(&key) {
                                            Some(FieldBinding::Column(index)) => index.to_string(),
                                            Some(FieldBinding::Fixed(_)) => "fixed".into(),
                                            None => String::new(),
                                        };
                                        let pick_key = key.clone();
                                        let edit_key = key.clone();
                                        rsx! {
                                            div { key:"{key}", class:"region",
                                                label { "{key}", select { disabled:is_busy, value:"{selected}", onchange:move |e| {
                                                    let value = e.value();
                                                    let mut choices = bindings.write();
                                                    if value == "fixed" {
                                                        choices.insert(pick_key.clone(), FieldBinding::Fixed(values.read().get(&pick_key).cloned().unwrap_or_default()));
                                                    } else if let Ok(index) = value.parse::<usize>() {
                                                        choices.insert(pick_key.clone(), FieldBinding::Column(index));
                                                    } else {
                                                        choices.remove(&pick_key);
                                                    }
                                                    page.set(0);
                                                },
                                                    option { value:"", "请选择对应列" }
                                                    for (i, header) in table.headers.iter().enumerate() {
                                                        {
                                                            let sample: String = table.rows.iter().map(|row| row[i].trim())
                                                                .find(|cell| !cell.is_empty()).unwrap_or_default().chars().take(20).collect();
                                                            let caption = if sample.is_empty() { header.clone() } else { format!("{header}（{sample}）") };
                                                            rsx! { option { value:"{i}", "第 {i+1} 列 · {caption}" } }
                                                        }
                                                    }
                                                    option { value:"fixed", "填写固定值" }
                                                } }
                                                if let Some(FieldBinding::Fixed(value)) = bindings.read().get(&key) {
                                                    input { "aria-label":"{key}固定值", value:"{value}", disabled:is_busy,
                                                        oninput:move |e| {bindings.write().insert(edit_key.clone(), FieldBinding::Fixed(e.value()));}
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        } else if let Err(reason) = &*data_table.read() {
                            p { class:"error inline", "{reason}" }
                        }
                        div { class: "coords numeric",
                            label { "记录数", input { r#type: "number", min: 1, max: "500", disabled: is_busy || using_table, value: if using_table { table_record_count } else { count() }, oninput: move |e| if let Ok(v) = e.value().parse() {count.set(v); page.set(0);} } }
                            label { "起号", input { r#type: "number", disabled: is_busy, value: "{start}", oninput: move |e| if let Ok(v) = e.value().parse() {start.set(v);} } }
                            label { "步长", input { r#type: "number", disabled: is_busy, value: "{step}", oninput: move |e| if let Ok(v) = e.value().parse() {step.set(v);} } }
                            label { "位数", input { r#type: "number", min: 0, max: "18", disabled: is_busy, value: "{pad}", oninput: move |e| if let Ok(v) = e.value().parse() {pad.set(v);} } }
                        }
                        p { class: "muted", "用 {{seq}} 代表递增号，用 {{列名}} 引用模板字段或表格列。同一记录的各份使用相同序号。" }
                    }
                    }
                }
            }
            if let Some(reason) = error_message {
                dialog { key:"{show_link_error}", id:"n1-error-dialog", "data-link-loss":show_link_error, class:"dialog native-dialog", "aria-labelledby":"n1-error-title", "aria-describedby":"n1-error-message",
                    onmounted: move |_| {
                        let _=document::eval("const dialog = document.getElementById('n1-error-dialog'); if (dialog && !dialog.open) { dialog.addEventListener('cancel', e=>e.preventDefault()); dialog.addEventListener('click', e=>{ if(e.target.closest('button')) { dialog.close(); if(dialog.dataset.linkLoss==='true') document.getElementById('connect-printer')?.focus(); const name=document.getElementById('template-name'); if(dialog.textContent.includes('请输入模板名称') && name) name.focus(); } }); dialog.showModal(); dialog.querySelector('button').focus(); }");
                    },
                    h2 { id:"n1-error-title", if show_link_error { "打印机已断开" } else { "操作未完成" } }
                    p { id:"n1-error-message", "{reason}" }
                    div { class:"actions",
                        button { r#type:"button", class:"primary", onclick: move |_| {
                            if show_link_error { bluetooth_loss.set(None); } else { notice.set((String::new(), false)); }
                        }, "确认" }
                    }
                }
            }
            if let Some(scope) = confirm_print() {
                    dialog { id:"print-dialog", class:"dialog native-dialog", "aria-labelledby":"print-title",
                        onmounted:move |_| { let _=document::eval("const d=document.getElementById('print-dialog'); d.addEventListener('cancel', e=>{e.preventDefault();d.querySelector('button').click()}); d.addEventListener('click', e=>{if(e.target.closest('button')) d.close()}); d.showModal(); d.querySelector('button').focus()"); },
                        h2 { id:"print-title", if scope == PrintScope::Range { "{model_name} 四边范围标定" } else if matches!(scope, PrintScope::Current(_)) { "试打第 {number} 条？" } else { "打印 {total} 张标签？" } }
                        if scope == PrintScope::Range {
                            if let Some(label) = range_label.read().as_ref() {
                                div { class:"range-preview", img { src:"{label.preview}", alt:"先出纸边朝上的四边标定点阵" } }
                            }
                            p { "箭头朝上读。T 是先出纸边，B 是最后出纸边，L、R 分别是纸面左、右边。" }
                            p { "从每边向内数，每阶 1 点，每个长刻度 8 点（约 1 mm）。最外侧可见台阶的点数就是该边缺失的点数。" }
                            if model == Model::P2 { p { "40 × 30 mm 标签居中装纸。实线标出纸边，虚线距边 1 mm。" } }
                        } else {
                        p { if matches!(scope, PrintScope::Current(_)) { "仅打印当前预览的 1 张。" } else { "{label_count} 条记录，每条连续打印 {copy_count} 份。" } }
                        if model == Model::P2 {
                            p { "请将 {width_mm} × {length_mm} mm 间隙标签居中装入。发送后请核对实际出纸。" }
                        } else {
                            p { "请确认 {model_name} 已装好 {width_mm} × {length_mm} mm 标签及对应耗材。" }
                        }
                        }
                        div { class: "actions",
                            button { r#type: "button", onclick: move |_| confirm_print.set(None), "取消" }
                            button { r#type: "button", class: "primary", disabled:!linked, onclick: move |_| {
                                confirm_print.set(None);
                                if busy() || !supported_media { return; }
                                let Some(plan) = PrintJob::new(scope, label_count, copy_count) else { return; };
                                let repeat = plan.copies;
                                let job = match scope {
                                    PrintScope::Range => Ok(Vec::new()),
                                    PrintScope::Current(_) => preview().map(|svg| vec![svg]),
                                    PrintScope::Batch => batch_rows.read().as_ref().map_err(Clone::clone).and_then(|rows| {
                                        core::preflight(&template.read(), rows, count(), Sequence { start:start(), step:step(), pad:pad() })
                                    }),
                                };
                                let items = match job {
                                    Ok(items) => items,
                                    Err(reason) => { notice.set((reason, true)); return; }
                                };
                                let printer = connection.read().clone();
                                if let Some(printer) = printer {
                                    busy.set(true);
                                    let stock = template.read().media.clone();
                                    let calibration = if scope == PrintScope::Range { range_label.write().take() } else { None };
                                    printing.set(true);
                                    stop_requested.set(false);
                                    notice.set(("正在准备打印图像…".into(), false));
                                    spawn(async move {
                                        let pages = tokio::task::spawn_blocking(move || {
                                            if let Some(label) = calibration { return Ok(vec![label.page]); }
                                            items.iter().map(|svg| model.rasterize(svg, &stock)).collect::<Result<Vec<_>,_>>()
                                        }).await;
                                        match pages {
                                            Ok(Ok(pages)) => {
                                                let mut printer=printer.lock().await;
                                                if connection.peek().is_none() || !printer.is_connected().await {
                                                    connection.set(None);
                                                    notice.set(("打印尚未开始，请重新连接打印机。".into(),true));
                                                } else {
                                                    let total=pages.len() * repeat;
                                                    let mut completed=0;
                                                    let mut failure = None;
                                                    for buf in pages.iter().flat_map(|page| std::iter::repeat_n(page, repeat)) {
                                                        if stop_requested() { break; }
                                                        notice.set((format!("正在打印第 {} / {} 张…",completed+1,total),false));
                                                        match printer.print_page(buf).await {
                                                            Ok(())=>completed+=1,
                                                            Err(reason)=>{ failure=Some(reason); break; }
                                                        }
                                                    }
                                                    if completed==total {
                                                        notice.set((format!("{delivery} {total} 张，请核对实际出纸。"),false));
                                                    } else if let Some(reason) = failure {
                                                        if connection.peek().is_some() && !printer.is_connected().await { connection.set(None); }
                                                        notice.set((format!("{reason} {delivery} {completed}/{total} 张，出纸情况请核对实物后再继续。"),true));
                                                    } else {
                                                        notice.set((format!("已停止，{delivery} {completed}/{total} 张。"),false));
                                                    }
                                                }
                                            }
                                            Ok(Err(e))=>notice.set((e,true)),
                                            Err(_)=>notice.set(("打印图像生成失败；没有发送标签。".into(),true)),
                                        }
                                        if connection.peek().is_none() {
                                            device_panel.set(true);
                                            if !usb_mode() {
                                                let detail = notice.peek().0.clone();
                                                notice.set((format!("{model_name} 蓝牙连接已断开。{detail}"), true));
                                            }
                                        }
                                        printing.set(false);
                                        busy.set(false);
                                    });
                                }
                            }, if scope == PrintScope::Range && !linked { "请先连接 {model_name}" } else { "开始打印" } }
                        }
                    }
            }
        }
    }
}
