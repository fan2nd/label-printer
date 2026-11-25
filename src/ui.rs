use crate::core::{self, Field, Kind, Media, Sequence, Template};
use crate::{
    device::{Device, Model},
    editor::{self, Gesture},
    print_job::{PrintJob, PrintScope},
    storage,
};
use dioxus::prelude::*;
use std::sync::Arc;
use tokio::sync::Mutex;

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
    delta: (i32, i32),
}

impl CanvasDrag {
    fn delta_at(self, event: &PointerEvent) -> (i32, i32) {
        let point = event.client_coordinates();
        (
            ((point.x - self.start.0) * self.dots_per_pixel.0).round() as i32,
            ((point.y - self.start.1) * self.dots_per_pixel.1).round() as i32,
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
    }
}

#[component]
pub fn App() -> Element {
    let mut template = use_signal(Template::default);
    let mut library = use_signal(|| storage::templates().unwrap_or_default());
    let media_library = use_signal(|| storage::media().unwrap_or_default());
    let mut csv_text = use_signal(String::new);
    let mut count = use_signal(|| 1usize);
    let mut start = use_signal(|| 1i64);
    let mut step = use_signal(|| 1i64);
    let mut pad = use_signal(|| 4usize);
    let mut zoom_dots = use_signal(|| false);
    let mut page = use_signal(|| 0usize);
    let mut field = use_signal(|| 0usize);
    let mut canvas_pixels = use_signal(|| (0.0, 0.0));
    let mut drag = use_signal(|| None::<CanvasDrag>);
    let mut notice = use_signal(|| {
        match storage::templates()
            .map(|_| ())
            .and_then(|_| storage::media().map(|_| ()))
        {
            Ok(()) => (String::new(), false),
            Err(e) => (e, true),
        }
    });
    let mut busy = use_signal(|| false);
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
    let mut remove_element = move |i: usize| {
        if busy() || i >= template.read().elements.len() {
            return;
        }
        template.write().elements.remove(i);
        let selected = field();
        if i <= selected {
            field.set(selected.saturating_sub(1));
        }
    };
    let mut connection = use_signal(|| None::<Connection>);
    let mut copies = use_signal(|| "1".to_owned());
    let mut confirm_print = use_signal(|| None::<PrintScope>);
    let mut printing = use_signal(|| false);
    let mut stop_requested = use_signal(|| false);

    let csv_rows = use_memo(move || {
        let input = csv_text.read();
        if input.trim().is_empty() {
            Ok(Vec::new())
        } else {
            core::parse_csv(&input)
        }
    });
    let records = use_memo(move || {
        let rows = csv_rows.read();
        let rows = rows.as_ref().map_err(Clone::clone)?;
        let n = if rows.is_empty() { count() } else { rows.len() };
        if (1..=core::MAX_BATCH).contains(&n) {
            Ok(n)
        } else {
            Err(format!("记录数须为 1～{} 条。", core::MAX_BATCH))
        }
    });
    let label_count = records().unwrap_or(0);
    let current_page = page().min(label_count.saturating_sub(1));
    let number = current_page + 1;
    let preview = use_memo(move || {
        let n = records()?;
        let index = page().min(n - 1);
        let rows = csv_rows.read();
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
    let delivery = if model == Model::N1 {
        "打印机已确认"
    } else {
        "已发送"
    };
    let status = if linked { "已连接" } else { "未连接" };
    let (msg, error) = notice();
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
    let canvas_cap = (260.0 * f64::from(w) / f64::from(h))
        .min(640.0)
        .min(f64::from(w) * 1.6);
    let canvas_items = use_memo(move || {
        let idx = page().min(records().unwrap_or(0).saturating_sub(1));
        let rows = csv_rows.read();
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
    let physical_caption = format!("{width_mm} × {length_mm} mm · {} dpi", core::DPI);
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
    let transport = if model == Model::N1 && usb_mode() {
        "USB"
    } else {
        "蓝牙"
    };
    let feed_margin = cut.feed_margin_mm;
    let usable_length = printable_length_dots.saturating_sub(2 * crate::stock::dots(feed_margin));
    let layout_name = match cut.combined {
        Some(true) => "组合标签",
        Some(false) => "单张标签",
        None => "组合类型未标注",
    };
    let corner_name = match cut.rounded {
        Some(true) => "圆角",
        Some(false) => "直角",
        None => "角型未标注",
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
        .position(|t| *t == *template.read())
        .map(|i| i.to_string())
        .unwrap_or_default();

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
        field.set(index);
        drag.set(Some(CanvasDrag {
            pointer: event.pointer_id(),
            index,
            id: template.read().elements[index].id,
            mode,
            start: (point.x, point.y),
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
                div { class: "actions",
                    if !linked {
                    select { class:"printer-model", "aria-label":"打印机型号", disabled:is_busy || linked, value:"{model_name}", onchange: move |e| {
                        if busy() || connection.read().is_some() { return; }
                        let next = match e.value().as_str() {
                            "P2" => Model::P2, "N1" => Model::N1, _ => return,
                        };
                        let builtin = core::builtin_media().iter().any(|(_, m, _)| *m == template.read().media);
                        if builtin { template.write().media = Media::for_model(next); }
                        selected_model.set(next);
                        page.set(0);
                    },
                        option { value:"N1", "精臣 N1" }
                        option { value:"P2", "德佟 P2" }
                    }
                    if model == Model::N1 {
                        select { "aria-label":"N1 连接方式", disabled:is_busy || linked, value:if usb_mode() {"usb"} else {"ble"},
                            onchange:move |e| { let usb=e.value()=="usb"; usb_mode.set(usb); if usb {refresh_ports();} },
                            option { value:"ble", "蓝牙" }
                            option { value:"usb", "USB" }
                        }
                        if usb_mode() {
                            select { class:"usb-port", title:"{usb_port}", "aria-label":"USB 串口", disabled:is_busy || linked, value:"{usb_port}", onchange:move |e| usb_port.set(e.value()),
                                option { value:"", if usb_ports.read().is_empty() {"未发现 USB 串口"} else {"选择打印机端口"} }
                                for (path,label) in usb_ports.read().iter() {
                                    option { key:"{path}", value:"{path}", "{label}" }
                                }
                            }
                            button { r#type:"button", disabled:is_busy, onclick:move |_| refresh_ports(), "刷新端口" }
                        }
                    }
                    }
                    if linked { span { class:"status good", "{model_name} · {transport} {status}" } }
                    button { r#type: "button", disabled: is_busy || (!linked && model == Model::N1 && usb_mode() && usb_port.read().is_empty()), onclick: move |_| {
                        if busy() { return; }
                        busy.set(true);
                        let previous = connection.read().clone();
                        if let Some(printer) = previous {
                            notice.set(("正在断开打印机…".into(), false));
                            spawn(async move {
                                let result = printer.lock().await.disconnect().await;
                                connection.set(None);
                                busy.set(false);
                                notice.set(match result { Ok(())=>("打印机已断开。".into(),false), Err(e)=>(e,true) });
                            });
                        } else {
                            notice.set((format!("正在搜索并识别 {model_name}…"), false));
                            spawn(async move {
                                let port = (model == Model::N1 && usb_mode()).then(|| usb_port());
                                match model.connect(port).await {
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
                                    }
                                    Err(e)=>notice.set((e,true)),
                                }
                                busy.set(false);
                            });
                        }
                    }, if linked { "断开连接" } else { "连接 {model_name}" } }
                    if printing() {
                        button { r#type:"button", disabled:stop_requested(), onclick:move |_| stop_requested.set(true),
                            if stop_requested() { "正在停止…" } else { "停止后续打印" }
                        }
                    } else {
                        button { r#type:"button", disabled:!can_print || invalid_page, onclick:move |_| confirm_print.set(Some(PrintScope::Current(current_page))), "试打当前张" }
                        button { r#type:"button", class:"primary", disabled:!can_print || invalid_page || !batch_valid, onclick:move |_| confirm_print.set(Some(PrintScope::Batch)),
                            if batch_valid { "打印 {total} 张" } else { "批量打印" }
                        }
                    }
                }
                if !error && !msg.is_empty() {
                    span { class:"toolbar-notice muted", role:"status", "{msg}" }
                }
            }
            main { class: "workspace",
                section { class: "editor", "aria-label": "标签设计",
                    div { class: "section-row",
                        h2 { "打印预览" }
                        div { class:"actions",
                            button { r#type:"button", disabled: dot_preview.read().is_err(), onclick:move |_| zoom_dots.toggle(),
                                if zoom_dots() { "适应标签" } else { "放大查看" }
                            }
                            button { r#type: "button", disabled: invalid_page || is_busy,
                                onclick: move |_| {
                                    if busy() { return; }
                                    let svg = preview.read().as_ref().ok().cloned();
                                    if let Some(svg)=svg {
                                        busy.set(true);
                                        spawn(async move {
                                            let handle = rfd::AsyncFileDialog::new().set_file_name("N1-label.svg").add_filter("SVG", &["svg"]).save_file().await;
                                            if let Some(handle)=handle {
                                                notice.set(match std::fs::write(handle.path(),svg) {
                                                    Ok(())=>("已导出 SVG。".into(),false),
                                                    Err(_)=>("无法写入 SVG 文件。".into(),true),
                                                });
                                            }
                                            busy.set(false);
                                        });
                                    }
                                }, "导出当前 SVG" }
                        }
                    }
                    div { class: "preview",
                        div { class:"dot-viewport", tabindex:if zoom_dots() { "0" } else { "-1" },
                            div { class:"label-sheet",
                                style: if zoom_dots() {
                                    format!("aspect-ratio:{w}/{h};width:{}px;max-width:none", w * 2)
                                } else {
                                    format!("aspect-ratio:{w}/{h};width:100%;max-width:{canvas_cap}px")
                                },
                                div { id:"label-canvas", class:"label-canvas", style:"{paper_outline}", "aria-label":"标签画布",
                                    "data-w":"{w}", "data-h":"{h}", "data-disabled":"{is_busy}",
                                    onresize:move |event| {
                                        if let Ok(size) = event.get_content_box_size() {
                                            canvas_pixels.set((size.width,size.height));
                                        }
                                    },
                                    if let Ok(src) = &*dot_preview.read() {
                                        img { class:"dot-print", "data-zoomed":zoom_dots(), src:"{src}", draggable:false, alt:"标签打印效果" }
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
                                            "aria-label":"画布元素 {i + 1} {kind_name(item.kind)}",
                                            "aria-pressed":field() == i, "data-invalid":!valid,
                                            disabled:is_busy,
                                            "data-kind": match item.kind { Kind::Qr=>"qr", Kind::Image=>"image", _=>"other" }, "data-module":item.module_size.unwrap_or(0),
                                            "data-index":"{i}", "data-x":"{item.x}", "data-y":"{item.y}", "data-w":"{item.w}", "data-h":"{item.h}",
                                            style:"left:{100.0 * (item.x as f64 - f64::from(feed_shift)) / f64::from(w)}%;top:{100.0 * item.y as f64 / f64::from(h)}%;width:{100.0 * f64::from(item.w.max(1)) / f64::from(w)}%;height:{100.0 * f64::from(item.h.max(1)) / f64::from(h)}%",
                                            onpointerdown:move |event| start_drag(event,i,Gesture::Move),
                                            onclick:move |_| field.set(i),
                                            onkeydown:move |event| {
                                                let unit: i32=if event.modifiers().shift() {10} else {1};
                                                let (mut dx,mut dy)=match event.key() {Key::ArrowLeft=>(-unit,0),Key::ArrowRight=>(unit,0),Key::ArrowUp=>(0,-unit),Key::ArrowDown=>(0,unit),_=>return};
                                                let mode = if event.modifiers().alt() {
                                                    if item.kind == Kind::Text { return; }
                                                    let columns = i32::try_from((item.w / item.module_size.unwrap_or(1).max(1)).max(1)).unwrap_or(i32::MAX);
                                                    if item.kind != Kind::Image { dx = dx.saturating_mul(columns); }
                                                    if item.kind == Kind::Qr { dy = dy.saturating_mul(columns); }
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
                                if let Some(item) = &chosen {
                                    button { class:"element-remove canvas-remove danger", hidden:drag().is_some(), r#type:"button", disabled:is_busy,
                                        style:"--corner-x:{100.0 * (item.x as f64 + f64::from(item.w) - f64::from(feed_shift)) / f64::from(w)}%;--corner-y:{100.0 * item.y as f64 / f64::from(h)}%",
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
                        p { class: "muted", "当前规格仅可预览和导出。{model_name} 支持 {supported_stock}。" }
                    }
                    if template.read().elements.is_empty() {
                        p { class:"muted", "添加元素开始排版，也可以先保存空白模板。" }
                    } else if let Some(reason) = validation.or_else(|| dot_preview.read().as_ref().err().cloned()) {
                        p { class: "error inline", "{reason}" }
                    }
                    if !outside.read().is_empty() { p {class:"muted", "有 {outside.read().len()} 个元素超出可打印区域，实物可能印不全。"} }
                    if let Some(reason)=overlap() {p {class:"muted", "{reason}"}}
                    section { class: "region",
                        div { class: "section-row",
                            h2 { "元素" }
                            div { class: "actions",
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
                                    }, "删除越界元素"
                                }
                                for (kind,text,w,h) in [(Kind::Text,"新文字",136,24),(Kind::Qr,"N1",86,86),(Kind::Barcode,"0001",190,40)] {
                                    button { r#type: "button", disabled: is_busy, onclick: move |_| {
                                        let m = template.read().media.clone();
                                        let (across, down) = m.canvas_size();
                                        let i = template.read().elements.len();
                                        if i >= core::MAX_FIELDS { notice.set((format!("每个模板最多放置 {} 个元素。", core::MAX_FIELDS), true)); return; }
                                        let w = w.min(across).max(1);
                                        let h = h.min(down).max(1);
                                        let x = 4.min(across.saturating_sub(w));
                                        let y = 4.min(down.saturating_sub(h));
                                        template.write().elements.push(Field {id:uuid::Uuid::new_v4(), picture:None, kind, text: text.into(), x:x.into(), y:y.into(), w, h, font_size:(kind == Kind::Text).then_some(22), module_size:(kind != Kind::Text).then_some(2)});
                                        field.set(i);
                                    }, "加 {kind_name(kind)}" }
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
                                                Ok(Ok(item))=>{let index=template.read().elements.len(); template.write().elements.push(item); field.set(index);}
                                                Ok(Err(reason))=>notice.set((reason,true)),
                                                Err(_)=>notice.set(("读取图片失败。".into(),true)),
                                            }
                                        }
                                        busy.set(false);
                                    });
                                }, "加 图片" }
                            }
                        }
                        div { class: "elements",
                            for (i, object) in template.read().elements.iter().enumerate() {
                                div { key:"{object.id}", class:"element-entry",
                                    button { r#type:"button", disabled:is_busy, "aria-pressed":field() == i, onclick:move |_| field.set(i), "{kind_name(object.kind)}" }
                                    button { class:"element-remove danger", r#type:"button", disabled:is_busy,
                                        "aria-label":"删除{kind_name(object.kind)}", title:"删除元素",
                                        onclick:move |_| remove_element(i), "×"
                                    }
                                }
                            }
                        }
                        if let Some(object) = chosen {
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
                            } else {
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
                                if object.kind == Kind::Text {
                                    label { "字号 / 点", input { r#type:"number", min:"1", max:core::MAX_MEDIA_WIDTH_DOTS, disabled:is_busy,
                                        value:object.text_size(), oninput:move |e| {
                                            if let Ok(size @ 1..=core::MAX_MEDIA_WIDTH_DOTS) = e.value().parse::<u32>() {
                                                edit_item(template,canvas_items,field(),|item| item.font_size = Some(size));
                                            }
                                        }
                                    } }
                                } else if object.kind == Kind::Image {
                                    for (name,value,is_width) in [("宽 / 点",object.w,true),("高 / 点",object.h,false)] {
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
                            span { class:"muted", "尺寸 {object.w} × {object.h} 点" }
                        }
                    }
                }
                aside { class: "inspector",
                    section { class: "region",
                        h2 { "标签规格" }
                        label { "选用规格", select { disabled: is_busy, value: "{media_selector}", onchange: move |e| {
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
                        span { class:"muted", "{layout_name} · {corner_name} · 可打区 {usable_length} × {printable_width_dots} 点" }
                        if feed_margin > 0 { span { class:"muted", "走纸首尾各留 {feed_margin} mm" } }
                    }
                    section { class: "region",
                        h2 { "模板" }
                        label { "模板名称", input { id:"template-name", disabled: is_busy, value: "{save_name}", oninput: move |e| template.write().name=e.value() } }
                        div { class: "actions",
                            button { r#type: "button", disabled: is_busy, onclick: move |_| {
                                let mut item = template();
                                item.name = item.name.trim().to_owned();
                                if item.name.is_empty() { notice.set(("请输入模板名称。".to_string(), true)); return; }
                                if let Err(e)=core::validate_template(&item) { notice.set((e,true)); return; }
                                let mut entries = library();
                                if let Some(old) = entries.iter_mut().find(|existing| existing.name == item.name) { *old = item.clone(); }
                                else { entries.push(item.clone()); }
                                match storage::save_templates(&entries).is_ok() {
                                    true => { library.set(entries); template.set(item); notice.set(("模板已保存在本机。".to_string(), false)); }
                                    false => notice.set(("保存失败。请检查本机数据目录。".to_string(), true)),
                                }
                            }, "保存模板" }
                            button { r#type: "button", disabled: is_busy, onclick: move |_| {
                                let media = template.read().media.clone();
                                let draft = Template {media, ..Template::default()};
                                template.set(draft); field.set(0); page.set(0);
                                notice.set(("已载入默认模板，已保存的模板不会被改动。".to_string(), false));
                            }, "载入默认" }
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
                            }, "导出模板" }
                            button { r#type:"button", disabled:is_busy, onclick:move |_| {
                                if busy() {return;}
                                busy.set(true);
                                spawn(async move {
                                    if let Some(path)=rfd::AsyncFileDialog::new().add_filter("N1 标签模板", &["json"]).pick_file().await {
                                        match storage::import_template(path.path()) {
                                            Ok(item)=>{template.set(item);field.set(0);page.set(0);notice.set(("已导入模板，保存后才会加入本机模板列表。".into(),false));}
                                            Err(e)=>notice.set((e,true)),
                                        }
                                    }
                                    busy.set(false);
                                });
                            }, "导入模板" }

                        }
                        if !library.read().is_empty() {
                            label { "本机模板", select { disabled: is_busy, value: "{template_selector}", onchange: move |e| {
                                if let Ok(i) = e.value().parse::<usize>() {
                                    if let Some(saved) = library.read().get(i).cloned() { template.set(saved); field.set(0); page.set(0); }
                                }
                            },
                                option { value: "", "选择模板…" }
                                for (i, entry) in library.read().iter().enumerate() {
                                    option { key:"{i}", value:"{i}", "{entry.name}" }
                                }
                            } }
                        }
                    }
                    section { class: "region",
                        h2 { "批量与递增" }
                        label { "CSV 数据（可选）", textarea { rows: 3, disabled: is_busy, spellcheck: false, value: "{csv_text}", placeholder: "名称,编号", oninput: move |e| { csv_text.set(e.value()); page.set(0); } } }
                        button { r#type:"button", disabled:is_busy, onclick: move |_| {
                            if busy() { return; }
                            busy.set(true);
                            spawn(async move {
                                if let Some(handle)=rfd::AsyncFileDialog::new().add_filter("CSV", &["csv"]).pick_file().await {
                                    let bytes=std::fs::metadata(handle.path()).map(|m|m.len());
                                    if bytes.is_ok_and(|n|n <= 2*1024*1024) {
                                        match std::fs::read_to_string(handle.path()) {
                                            Ok(s)=>{ csv_text.set(s);page.set(0);notice.set(("已导入 CSV。".into(),false)); }
                                            Err(_)=>notice.set(("读取失败。请使用 UTF-8 编码的 CSV。".into(),true))
                                        }
                                    } else {
                                        notice.set(("CSV 文件无法读取或已超过 2 MB。".into(),true));
                                    }
                                }
                                busy.set(false);
                            });
                        }, "导入 CSV 文件" }
                        div { class: "coords numeric",
                            label { "记录数", input { r#type: "number", min: 1, max: "500", disabled: is_busy || !csv_text.read().trim().is_empty(), value: if csv_text.read().trim().is_empty() { count() } else { csv_rows.read().as_ref().map(Vec::len).unwrap_or(0) }, oninput: move |e| if let Ok(v) = e.value().parse() {count.set(v); page.set(0);} } }
                            label { "起号", input { r#type: "number", disabled: is_busy, value: "{start}", oninput: move |e| if let Ok(v) = e.value().parse() {start.set(v);} } }
                            label { "步长", input { r#type: "number", disabled: is_busy, value: "{step}", oninput: move |e| if let Ok(v) = e.value().parse() {step.set(v);} } }
                            label { "位数", input { r#type: "number", min: 0, max: "18", disabled: is_busy, value: "{pad}", oninput: move |e| if let Ok(v) = e.value().parse() {pad.set(v);} } }
                        }
                        p { class: "muted", "用 {{seq}} 代表递增号，用 {{列名}} 读取 CSV。同一记录的各份使用相同序号。" }
                    }
                }
            }
            if error {
                dialog { id:"n1-error-dialog", class:"dialog native-dialog", "aria-labelledby":"n1-error-title", "aria-describedby":"n1-error-message",
                    onmounted: move |_| {
                        let _=document::eval("const dialog = document.getElementById('n1-error-dialog'); if (dialog && !dialog.open) { dialog.addEventListener('cancel', e=>e.preventDefault()); dialog.addEventListener('click', e=>{ if(e.target.closest('button')) { dialog.close(); const name=document.getElementById('template-name'); if(dialog.textContent.includes('请输入模板名称') && name) name.focus(); } }); dialog.showModal(); dialog.querySelector('button').focus(); }");
                    },
                    h2 { id:"n1-error-title", "操作未完成" }
                    p { id:"n1-error-message", "{msg}" }
                    div { class:"actions",
                        button { r#type:"button", class:"primary", onclick: move |_| notice.set((String::new(),false)), "确认" }
                    }
                }
            }
            if let Some(scope) = confirm_print() {
                    dialog { id:"print-dialog", class:"dialog native-dialog", "aria-labelledby":"print-title",
                        onmounted:move |_| { let _=document::eval("const d=document.getElementById('print-dialog'); d.addEventListener('cancel', e=>{e.preventDefault();d.querySelector('button').click()}); d.addEventListener('click', e=>{if(e.target.closest('button')) d.close()}); d.showModal(); d.querySelector('button').focus()"); },
                        h2 { id:"print-title", if matches!(scope, PrintScope::Current(_)) { "试打第 {number} 条？" } else { "打印 {total} 张标签？" } }
                        p { if matches!(scope, PrintScope::Current(_)) { "仅打印当前预览的 1 张。" } else { "{label_count} 条记录，每条连续打印 {copy_count} 份。" } }
                        if model == Model::P2 {
                            p { "请将 {width_mm} × {length_mm} mm 间隙标签居中装入。发送后请核对实际出纸。" }
                        } else {
                            p { "请确认 {model_name} 已装好 {width_mm} × {length_mm} mm 标签及对应耗材。" }
                        }
                        div { class: "actions",
                            button { r#type: "button", onclick: move |_| confirm_print.set(None), "取消" }
                            button { r#type: "button", class: "primary", onclick: move |_| {
                                confirm_print.set(None);
                                if busy() || !supported_media { return; }
                                let Some(plan) = PrintJob::new(scope, label_count, copy_count) else { return; };
                                let repeat = plan.copies;
                                let job = match scope {
                                    PrintScope::Current(_) => preview().map(|svg| vec![svg]),
                                    PrintScope::Batch => csv_rows.read().as_ref().map_err(Clone::clone).and_then(|rows| {
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
                                    printing.set(true);
                                    stop_requested.set(false);
                                    notice.set(("正在准备打印图像…".into(), false));
                                    spawn(async move {
                                        let pages = tokio::task::spawn_blocking(move || items.iter()
                                            .map(|svg| model.rasterize(svg, &stock)).collect::<Result<Vec<_>,_>>()).await;
                                        match pages {
                                            Ok(Ok(pages)) => {
                                                let mut printer=printer.lock().await;
                                                if !printer.is_connected().await {
                                                    connection.set(None);
                                                    notice.set(("连接已断开，请重新连接打印机。".into(),true));
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
                                                        if !printer.is_connected().await { connection.set(None); }
                                                        notice.set((format!("{reason} {delivery} {completed}/{total} 张，出纸情况请核对实物后再继续。"),true));
                                                    } else {
                                                        notice.set((format!("已停止，{delivery} {completed}/{total} 张。"),false));
                                                    }
                                                }
                                            }
                                            Ok(Err(e))=>notice.set((e,true)),
                                            Err(_)=>notice.set(("打印图像生成失败；没有发送标签。".into(),true)),
                                        }
                                        printing.set(false);
                                        busy.set(false);
                                    });
                                }
                            }, "开始打印" }
                        }
                    }
            }
        }
    }
}
