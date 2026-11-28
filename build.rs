#[path = "src/stock.rs"]
mod stock;

use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    fmt::Write,
    fs,
    path::PathBuf,
};
use stock::{Model, MAX_MEDIA_LENGTH_DOTS};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Label {
    id: uuid::Uuid,
    name: Option<String>,
    #[serde(default)]
    barcodes: Vec<String>,
    #[serde(default)]
    cut: stock::Cut,
    width_mm: u16,
    length_mm: u16,
}

fn main() {
    println!("cargo:rerun-if-changed=assets/labels.yaml");
    println!("cargo:rerun-if-changed=src/stock.rs");
    let yaml = fs::read_to_string("assets/labels.yaml").expect("读取 assets/labels.yaml");
    let catalog: BTreeMap<Model, Vec<Label>> =
        yaml_serde::from_str(&yaml).expect("解析 assets/labels.yaml");
    let mut generated = String::from("vec![\n");
    let mut ids = BTreeSet::new();
    for model in [Model::N1, Model::P2, Model::P0] {
        let entries = catalog
            .get(&model)
            .filter(|e| !e.is_empty())
            .unwrap_or_else(|| panic!("assets/labels.yaml 中 {model:?} 至少需要一个默认规格"));
        let mut names = BTreeSet::new();
        let mut barcode_set = BTreeSet::new();
        for label in entries {
            assert!(
                !label.id.is_nil() && ids.insert(label.id),
                "标签 UUID 无效或重复"
            );
            let id = label.id.as_u128();
            for code in &label.barcodes {
                assert!(
                    !code.trim().is_empty() && barcode_set.insert(code),
                    "assets/labels.yaml 中 {model:?} 的耗材编码为空或重复：{code}"
                );
            }
            let barcodes = &label.barcodes;
            let (w, h) = (label.width_mm, label.length_mm);
            let name = label
                .name
                .clone()
                .unwrap_or_else(|| format!("{model:?} · {w} × {h} mm"));
            assert!(
                !name.trim().is_empty() && names.insert(name.clone()),
                "assets/labels.yaml 中 {model:?} 的规格名为空或重复：{name}"
            );
            let paper_width = stock::dots_at(w, stock::DPI);
            let width = model
                .head_dots()
                .map_or(paper_width, |head| paper_width.min(head));
            let length = stock::dots_at(h, stock::DPI);
            assert!(
                width > 0 && (1..=MAX_MEDIA_LENGTH_DOTS).contains(&length),
                "assets/labels.yaml 中 {name} 的纸张尺寸超出范围"
            );
            let cut = &label.cut;
            cut.validate(length, width)
                .unwrap_or_else(|e| panic!("assets/labels.yaml {name}: {e}"));
            let cut=format!("crate::stock::Cut {{rounded:{:?},combined:{:?},radius_dots:{},old_margin:(),regions_dots:vec!{:?} }}",cut.rounded,cut.combined,cut.radius_dots,cut.regions_dots);
            let dpi = stock::DPI;
            writeln!(generated,"(Model::{model:?}, Media {{ id:uuid::Uuid::from_u128({id}), dpi:{dpi}, cut:{cut}, name: {name:?}.into(), width_mm: {w}, length_mm: {h}, printable_width_dots: {width}, printable_length_dots: {length} }}, &{barcodes:?}[..]),").unwrap();
        }
    }
    generated.push(']');
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("labels.rs"),
        generated,
    )
    .unwrap();
}
