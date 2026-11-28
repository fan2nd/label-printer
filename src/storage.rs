use crate::core::{validate_template, Field, Media, Template};
use directories::ProjectDirs;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

fn path(file: &str) -> Result<PathBuf, String> {
    Ok(ProjectDirs::from("", "N1 Label Studio", "N1 Label Studio")
        .ok_or_else(|| "系统没有提供用户数据目录。".to_string())?
        .data_local_dir()
        .join(file))
}

fn load<T: DeserializeOwned + Default>(file: &str) -> Result<T, String> {
    match fs::read(path(file)?) {
        Ok(data) => {
            serde_json::from_slice(&data).map_err(|_| "本机数据无法读取，请勿覆盖原文件。".into())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(_) => Err("无法读取本机数据。".into()),
    }
}
fn save<T: Serialize + ?Sized>(file: &str, value: &T) -> Result<(), String> {
    let target = path(file)?;
    let dir = target.parent().ok_or("无法确定数据目录。")?;
    fs::create_dir_all(dir).map_err(|_| "无法创建用户数据目录。")?;
    let bytes = serde_json::to_vec_pretty(value).map_err(|_| "无法编码本机数据。")?;
    atomic_write(&target, &bytes)
}
pub fn available_name(base: &str, templates: &[Template]) -> String {
    let exists = |name: &str| templates.iter().any(|template| template.name == name);
    if !exists(base) {
        return base.into();
    }
    let mut number = 2;
    loop {
        let candidate = format!("{base} {number}");
        if !exists(&candidate) {
            return candidate;
        }
        number += 1;
    }
}

pub struct Preset {
    pub template: Template,
    pub values: crate::core::Row,
}

pub fn presets() -> &'static [Preset] {
    #[derive(Deserialize)]
    struct Entry {
        name: String,
        id: uuid::Uuid,
        media_id: uuid::Uuid,
        values: crate::core::Row,
        elements: Vec<Field>,
    }
    static LIBRARY: std::sync::OnceLock<Vec<Preset>> = std::sync::OnceLock::new();
    LIBRARY.get_or_init(|| {
        let entries: Vec<Entry> = serde_json::from_str(include_str!("../assets/templates.json"))
            .expect("内置模板格式错误");
        entries
            .into_iter()
            .map(|entry| {
                let (_, media, _) = crate::core::builtin_media()
                    .iter()
                    .find(|(_, media, _)| media.id == entry.media_id)
                    .expect("内置模板引用了未知耗材");
                Preset {
                    template: Template {
                        id: entry.id,
                        name: entry.name,
                        media: media.clone(),
                        elements: entry.elements,
                    },
                    values: entry.values,
                }
            })
            .collect()
    })
}

pub fn templates() -> Result<Vec<Template>, String> {
    let mut items: Vec<Template> = load("templates.json")?;
    let migrate = items.iter().any(|t| t.id.is_nil() || t.media.id.is_nil());
    for item in &mut items {
        ensure_template_ids(item);
    }
    if migrate {
        save("templates.json", &items)?;
    }
    Ok(items)
}
pub fn recent_templates() -> Result<Vec<String>, String> {
    load("recent-templates.json")
}

pub fn record_template(name: &str) -> Result<Vec<String>, String> {
    let mut names = recent_templates()?;
    names.retain(|n| n != name);
    names.insert(0, name.to_owned());
    names.truncate(4);
    save("recent-templates.json", &names)?;
    Ok(names)
}
pub fn media() -> Result<Vec<Media>, String> {
    let mut items: Vec<Media> = load("media.json")?;
    if items.iter().any(|m| m.id.is_nil()) {
        for m in &mut items {
            resolve_media_id(m);
        }
        save("media.json", &items)?;
    }
    Ok(items)
}

pub fn save_templates(value: &[Template]) -> Result<(), String> {
    let _ = templates()?; // avoid overwriting an unreadable/corrupt library
    save("templates.json", value)
}
// The public exchange format is deliberately independent of the local library
// files. In particular, version 1 must include media; it cannot silently apply
// the local library's legacy default for a missing media object.
const TEMPLATE_FORMAT: &str = "n1-label-studio-template";
const TEMPLATE_VERSION: u32 = 2;
const MAX_PORTABLE_BYTES: u64 = 24 * 1024 * 1024;

#[derive(Serialize)]
struct ExportTemplate<'a> {
    format: &'static str,
    version: u32,
    template: &'a Template,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportTemplate {
    format: String,
    version: u32,
    template: ImportPayload,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportPayload {
    #[serde(default)]
    id: uuid::Uuid,
    name: String,
    media: Media,
    elements: Vec<Field>,
}

fn resolve_media_id(media: &mut Media) {
    if media.id.is_nil() {
        media.id = crate::core::builtin_media()
            .iter()
            .find(|(_, stock, _)| {
                stock.name == media.name
                    && stock.width_mm == media.width_mm
                    && stock.length_mm == media.length_mm
            })
            .map(|(_, stock, _)| stock.id)
            .unwrap_or_else(uuid::Uuid::new_v4);
    }
}

fn ensure_template_ids(template: &mut Template) {
    if template.id.is_nil() {
        template.id = uuid::Uuid::new_v4();
    }
    resolve_media_id(&mut template.media);
    let elements = &mut template.elements;
    let mut seen = std::collections::HashSet::new();
    for field in elements {
        while !seen.insert(field.id) {
            field.id = uuid::Uuid::new_v4();
        }
    }
}

fn validate_portable(template: &Template) -> Result<(), String> {
    for (name, label) in [
        (&template.name, "模板名称"),
        (&template.media.name, "规格名称"),
    ] {
        if name.trim().is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
            return Err(format!("{label}不能为空、超过 256 字节或包含控制字符。"));
        }
    }
    validate_template(template)
}

fn atomic_write(target: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(|_| "无法创建临时文件。")?;
    tmp.write_all(bytes)
        .and_then(|_| tmp.as_file_mut().sync_all())
        .map_err(|_| "无法写入文件。")?;
    tmp.persist(target).map_err(|_| "无法保存目标文件。")?;
    Ok(())
}

pub fn export_template(path: &Path, template: &Template) -> Result<(), String> {
    validate_portable(template)?;
    let envelope = ExportTemplate {
        format: TEMPLATE_FORMAT,
        version: TEMPLATE_VERSION,
        template,
    };
    let data = serde_json::to_vec_pretty(&envelope).map_err(|_| "无法编码模板文件。")?;
    if data.len() as u64 > MAX_PORTABLE_BYTES {
        return Err("模板文件不能超过 24 MiB。".into());
    }
    atomic_write(path, &data)
}

pub fn import_template(path: &Path) -> Result<Template, String> {
    let file = fs::File::open(path).map_err(|_| "无法打开模板文件。")?;
    if !file.metadata().map_err(|_| "无法读取模板文件。")?.is_file() {
        return Err("请选择普通模板文件。".into());
    }
    let mut data = Vec::new();
    file.take(MAX_PORTABLE_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|_| "无法读取模板文件。")?;
    if data.len() as u64 > MAX_PORTABLE_BYTES {
        return Err("模板文件不能超过 24 MiB。".into());
    }
    // Read only the header first so unsupported future formats get a meaningful
    // error even if their payload schema differs from v1.
    #[derive(Deserialize)]
    struct Header {
        format: String,
        version: u32,
    }
    let header: Header = serde_json::from_slice(&data).map_err(|_| "无效的模板文件。")?;
    if header.format != TEMPLATE_FORMAT {
        return Err("文件不是 N1 标签模板。".into());
    }
    if !(1..=TEMPLATE_VERSION).contains(&header.version) {
        return Err(format!("不支持模板版本 {}。", header.version));
    }
    let parsed: ImportTemplate =
        serde_json::from_slice(&data).map_err(|_| "模板内容不完整或无效。")?;
    debug_assert_eq!(parsed.format, TEMPLATE_FORMAT);
    debug_assert_eq!(parsed.version, header.version);
    let mut template = Template {
        id: parsed.template.id,
        name: parsed.template.name,
        media: parsed.template.media,
        elements: parsed.template.elements,
    };
    ensure_template_ids(&mut template);
    validate_portable(&template)?;
    Ok(template)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    #[test]
    fn catalog_ids_are_unique_and_copies_keep_their_stock() {
        let mut ids = std::collections::HashSet::new();
        for (_, m, _) in crate::core::builtin_media() {
            assert!(ids.insert(m.id));
        }
        for p in presets() {
            let t = &p.template;
            assert!(ids.insert(t.id));
            for f in &t.elements {
                assert!(ids.insert(f.id));
            }
            let copy = t.duplicate();
            assert_eq!(copy.media.id, t.media.id);
            assert!(ids.insert(copy.id));
            for f in copy.elements {
                assert!(ids.insert(f.id));
            }
        }
        assert!(!ids.contains(&uuid::Uuid::nil()));
    }

    #[test]
    fn preset_samples_fit_their_stock() {
        for preset in presets() {
            let t = &preset.template;

            let svg = crate::core::render_svg(t, &preset.values, "0001").unwrap();
            let shift = if t.media.is_known_n1() {
                assert_eq!(
                    crate::raster::packed_n1(&svg, &t.media).unwrap().len(),
                    200 * 12
                );
                crate::raster::N1_FEED_SHIFT
            } else {
                crate::raster::stock_preview_png(&svg, &t.media).unwrap();
                0
            };
            let mut previous_bottom = 0;
            for (i, f) in t.elements.iter().enumerate() {
                let (bounds, art) = crate::core::editor_art(f, &preset.values, "0001", i).unwrap();
                assert!(
                    !crate::core::outside_media(&t.media, &bounds, Some(&art), shift),
                    "{}",
                    t.name
                );
                assert!(bounds.y >= previous_bottom, "{}", t.name);
                previous_bottom = bounds.y + i64::from(bounds.h);
            }
        }
    }

    #[test]
    fn portable_roundtrip_preserves_media_and_unresolved_variables() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("模板.n1.json");
        let mut t = Template::example();
        t.media.name = "自定义标签".into();
        t.media.length_mm = 60;
        t.media.printable_length_dots = 480;
        t.elements[0].text = "批次 {批号} / {seq}".into();
        export_template(&file, &t).unwrap();
        // Ensure updates replace the file and retain both media + placeholders.
        t.name = "第二版".into();
        export_template(&file, &t).unwrap();
        let saved: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        assert_eq!(saved["version"], TEMPLATE_VERSION);
        assert_eq!(saved.as_object().unwrap().len(), 3);
        assert_eq!(import_template(&file).unwrap(), t);
    }

    #[test]
    fn imports_v1_templates_with_legacy_element_boxes() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("old.json");
        let mut legacy = serde_json::to_value(Template::example()).unwrap();
        for field in legacy["elements"].as_array_mut().unwrap() {
            field.as_object_mut().unwrap().remove("id");
            field.as_object_mut().unwrap().remove("font_size");
            field.as_object_mut().unwrap().remove("module_size");
        }
        fs::write(
            &file,
            json!({"format": TEMPLATE_FORMAT, "version": 1, "template":legacy}).to_string(),
        )
        .unwrap();
        let template = import_template(&file).unwrap();
        assert_eq!(template.elements[2].module_size, None);
        crate::core::render_svg(&template, &Default::default(), "0001").unwrap();
    }

    #[test]
    fn rejects_malformed_unknown_missing_media_and_oversize() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("bad.n1.json");
        fs::write(&file, "{ broken").unwrap();
        assert!(import_template(&file).is_err());

        fs::write(
            &file,
            json!({"format": TEMPLATE_FORMAT, "version": 25}).to_string(),
        )
        .unwrap();
        assert!(import_template(&file).unwrap_err().contains("25"));

        let mut legacy = serde_json::to_value(Template::example()).unwrap();
        legacy.as_object_mut().unwrap().remove("media");
        let envelope = json!({"format": TEMPLATE_FORMAT, "version": 1, "template": legacy});
        fs::write(&file, envelope.to_string()).unwrap();
        assert!(import_template(&file).unwrap_err().contains("不完整"));

        fs::write(&file, vec![b' '; MAX_PORTABLE_BYTES as usize + 1]).unwrap();
        assert!(import_template(&file).unwrap_err().contains("24 MiB"));
    }

    #[test]
    fn unfinished_layouts_roundtrip_and_render() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("draft.json");
        let mut overlapping = Template::example();
        overlapping.elements[1].y = overlapping.elements[0].y;
        let mut outside = Template::example();
        outside.elements[0].x = outside.media.printable_length_dots.into();
        let empty = Template {
            elements: Vec::new(),
            ..Template::example()
        };
        for draft in [empty, overlapping, outside] {
            export_template(&file, &draft).unwrap();
            assert_eq!(import_template(&file).unwrap(), draft);
            assert!(crate::core::render_svg(&draft, &Default::default(), "0001").is_ok());
        }
    }

    #[test]
    fn invalid_geometry_never_replaces_existing_export() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("existing.n1.json");
        let good = Template::example();
        export_template(&file, &good).unwrap();
        let mut bad = good.clone();
        bad.elements[1].w = 0;
        assert!(export_template(&file, &bad).is_err());
        assert_eq!(import_template(&file).unwrap(), good);
        bad.elements[1].w = 1;
        bad.elements[1].x = i64::MAX;
        let envelope = json!({"format": TEMPLATE_FORMAT, "version": 1, "template": bad});
        fs::write(&file, envelope.to_string()).unwrap();
        assert!(import_template(&file).is_err());
    }
}
