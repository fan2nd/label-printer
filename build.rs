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
use stock::{dots, Model, MAX_MEDIA_LENGTH_DOTS};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Label {
    name: Option<String>,
    #[serde(default)]
    barcodes: Vec<String>,
    #[serde(default)]
    cut: stock::Cut,
    width_mm: u16,
    length_mm: u16,
}

fn main() {
    println!("cargo:rerun-if-changed=labels.yaml");
    println!("cargo:rerun-if-changed=src/stock.rs");
    let yaml = fs::read_to_string("labels.yaml").expect("读取 labels.yaml");
    let catalog: BTreeMap<Model, Vec<Label>> =
        yaml_serde::from_str(&yaml).expect("解析 labels.yaml");
    let mut generated = String::from("vec![\n");
    for model in [Model::N1, Model::P2] {
        let entries = catalog
            .get(&model)
            .filter(|e| !e.is_empty())
            .unwrap_or_else(|| panic!("labels.yaml 中 {model:?} 至少需要一个默认规格"));
        let mut names = BTreeSet::new();
        let mut barcode_set = BTreeSet::new();
        for label in entries {
            for code in &label.barcodes {
                assert!(
                    !code.trim().is_empty() && barcode_set.insert(code),
                    "labels.yaml 中 {model:?} 的耗材编码为空或重复：{code}"
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
                "labels.yaml 中 {model:?} 的规格名为空或重复：{name}"
            );
            let width = dots(w).min(model.head_dots());
            let length = dots(h);
            assert!(
                width > 0 && (1..=MAX_MEDIA_LENGTH_DOTS).contains(&length),
                "labels.yaml 中 {name} 的纸张尺寸超出范围"
            );
            let cut = &label.cut;
            cut.validate(length, width)
                .unwrap_or_else(|e| panic!("labels.yaml {name}: {e}"));
            let cut=format!("crate::stock::Cut {{rounded:{:?},combined:{:?},radius_dots:{},feed_margin_mm:{},regions_dots:vec!{:?} }}",cut.rounded,cut.combined,cut.radius_dots,cut.feed_margin_mm,cut.regions_dots);
            writeln!(generated,"(Model::{model:?}, Media {{ cut:{cut}, name: {name:?}.into(), width_mm: {w}, length_mm: {h}, printable_width_dots: {width}, printable_length_dots: {length} }}, &{barcodes:?}[..]),").unwrap();
        }
    }
    generated.push(']');
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("labels.rs"),
        generated,
    )
    .unwrap();
}
