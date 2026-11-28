pub mod command;
pub mod panel;
pub mod sample;
use crate::{core::Media, storage};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
};
use uuid::Uuid;

pub const LEVELS: [u8; 4] = [1, 5, 10, 15];
pub const ITEMS: [&str; 11] = [
    "1mm",
    "3mm",
    "6mm",
    "12mm",
    "20mm",
    "small",
    "small-bold",
    "large",
    "large-bold",
    "qr",
    "barcode",
];
pub const LABELS: [&str; 11] = [
    "1 mm",
    "3 mm",
    "6 mm",
    "12 mm",
    "20 mm",
    "小字 · 常规",
    "小字 · 粗体",
    "大字 · 常规",
    "大字 · 粗体",
    "二维码 · P2",
    "条码 · 4030",
];
pub fn choices(i: usize) -> &'static [&'static str] {
    match i {
        0..5 => &["合格", "偏淡", "拖尾", "有白点"],
        5..9 => &["清晰", "偏淡", "糊字"],
        _ => &["读值正确", "读值不符", "无法识读"],
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub id: Uuid,
    pub device: String,
    pub media: Media,
    pub material: String,
    pub font: String,
    pub version: u8,
    pub scores: BTreeMap<u8, BTreeMap<String, String>>,
}
impl Profile {
    pub fn new(
        device: String,
        media: Media,
        material: String,
        font: String,
    ) -> Result<Self, String> {
        if device.trim().is_empty() || material.trim().is_empty() {
            return Err("请填写设备标识和纸材名称。".into());
        }
        Ok(Self {
            id: Uuid::new_v4(),
            device,
            media,
            material,
            font,
            version: 1,
            scores: Default::default(),
        })
    }
    pub fn mark(&mut self, level: u8, item: &str, value: &str) -> Result<(), String> {
        let i = ITEMS
            .iter()
            .position(|k| *k == item)
            .ok_or("未知的样张项目。")?;
        if !LEVELS.contains(&level) || (!value.is_empty() && !choices(i).contains(&value)) {
            return Err("该档位或评价与样张项目不匹配。".into());
        }
        let row = self.scores.entry(level).or_default();
        if value.is_empty() {
            row.remove(item);
        } else {
            row.insert(item.into(), value.into());
        }
        Ok(())
    }
    pub fn score(&self, level: u8, item: &str) -> &str {
        self.scores
            .get(&level)
            .and_then(|r| r.get(item))
            .map(String::as_str)
            .unwrap_or("")
    }
}

fn directory() -> Result<std::path::PathBuf, String> {
    storage::path("p2-quality")
}
pub fn profiles() -> Result<Vec<Profile>, String> {
    let dir = directory()?;
    let entries = match fs::read_dir(dir) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e.to_string()),
    };
    let mut all = vec![];
    for e in entries {
        let path = e.map_err(|e| e.to_string())?.path();
        if path.extension().is_some_and(|s| s == "json") {
            let p: Profile = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            if p.version != 1 {
                return Err("本机校准记录版本无法读取。".into());
            }
            all.push(p);
        }
    }
    all.sort_by_key(|p| p.material.clone());
    Ok(all)
}

fn write_directory() -> Result<(std::path::PathBuf, fs::File), String> {
    let dir = directory()?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("write.lock"))
        .map_err(|e| e.to_string())?;
    lock.lock().map_err(|e| e.to_string())?;
    Ok((dir, lock))
}

type Preferred = BTreeMap<String, BTreeMap<Uuid, Uuid>>;
fn preferences() -> Result<Preferred, String> {
    match fs::read(directory()?.join("preferred")) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Preferred::new()),
        Err(e) => Err(e.to_string()),
    }
}
pub fn remember(device: &str, media: Uuid, id: Option<Uuid>) -> Result<(), String> {
    if device.is_empty() || device == "P2" {
        return Ok(());
    }
    let (dir, _guard) = write_directory()?;
    let mut prefs = preferences()?;
    let entry = prefs.entry(device.to_owned()).or_default();
    if let Some(id) = id {
        entry.insert(media, id);
    } else {
        entry.remove(&media);
    }
    storage::atomic_write(
        &dir.join("preferred"),
        &serde_json::to_vec(&prefs).map_err(|e| e.to_string())?,
    )
}
pub fn preferred(device: &str, m: &Media) -> Result<Option<Profile>, String> {
    if device.is_empty() || device == "P2" {
        return Ok(None);
    }
    let Some(id) = preferences()?
        .get(device)
        .and_then(|p| p.get(&m.id))
        .copied()
    else {
        return Ok(None);
    };
    Ok(profiles()?
        .into_iter()
        .find(|p| p.id == id && p.media == *m))
}
// Compare-and-save protects a GUI draft from overwriting edits made by the CLI.
pub fn save(profile: &Profile, base: Option<&Profile>) -> Result<(), String> {
    let (dir, _guard) = write_directory()?;
    let path = dir.join(format!("{}.json", profile.id));
    let existing = profiles()?;
    if existing.iter().find(|p| p.id == profile.id) != base {
        return Err("校准记录已更新，请重新载入后保存。".into());
    }
    storage::atomic_write(
        &path,
        &serde_json::to_vec_pretty(profile).map_err(|e| e.to_string())?,
    )
}

#[derive(Clone, Debug, Default)]
struct InkLoad {
    peak: usize,
    sustained: [usize; 5],
}
impl InkLoad {
    fn measure(page: &[u8], dpi: u32) -> Self {
        let lengths = [1u32, 3, 6, 12, 20].map(|mm| (f64::from(mm * dpi) / 25.4).round() as usize);
        let mut load = Self::default();
        let mut runs = [0usize; 384];
        for row in page.chunks_exact(48) {
            let mut counts = [0; 5];
            let mut ink = 0;
            for (x, run) in runs.iter_mut().enumerate() {
                if row[x / 8] & (128 >> (x % 8)) == 0 {
                    *run = 0;
                    continue;
                }
                *run += 1;
                ink += 1;
                for (i, len) in lengths.iter().enumerate() {
                    if *run >= *len {
                        counts[i] += 1;
                    }
                }
            }
            load.peak = load.peak.max(ink);
            for (max, n) in load.sustained.iter_mut().zip(counts) {
                *max = (*max).max(n);
            }
        }
        load
    }
}
fn sample_load(p: &Profile) -> Result<InkLoad, String> {
    // Scores do not affect ink. Reuse the measured envelope for a batch on this stock.
    thread_local! {static LAST:std::cell::RefCell<Option<(Media,String,InkLoad)>>=const{std::cell::RefCell::new(None)};}
    if let Some(load) = LAST.with_borrow(|last| {
        last.as_ref()
            .filter(|(m, f, _)| m == &p.media && f == &p.font)
            .map(|(_, _, load)| load.clone())
    }) {
        return Ok(load);
    }
    let plan = sample::Layout::build(p)?;
    let mut envelope = InkLoad::default();
    for i in 0..plan.count() {
        let load = InkLoad::measure(&plan.render(p, 15, i)?.page, p.media.dpi);
        envelope.peak = envelope.peak.max(load.peak);
        for (max, n) in envelope.sustained.iter_mut().zip(load.sustained) {
            *max = (*max).max(n);
        }
    }
    LAST.with_borrow_mut(|last| *last = Some((p.media.clone(), p.font.clone(), envelope.clone())));
    Ok(envelope)
}
pub fn recommend(profile: &Profile, media: &Media, page: &[u8], svg: &str) -> Result<u8, String> {
    if profile.media != *media {
        return Err("此纸张规格缺少校准。".into());
    }
    let across = media.printable_width_dots as usize;
    let actual = InkLoad::measure(page, media.dpi);
    if actual.peak == 0 {
        return Err("页面为空，请手选浓度。".into());
    }
    let measured = sample_load(profile)?;
    if actual.peak > measured.peak
        || actual
            .sustained
            .iter()
            .zip(measured.sustained)
            .any(|(a, b)| *a > b)
    {
        return Err("同一行的着黑点数或连续着黑的点数超出样张覆盖，请手选浓度。".into());
    }
    let mut needs = BTreeSet::new();
    let mut runs = [0usize; 384];
    let mut longest = 0usize;
    let mut narrow_run = 0usize;
    for row in page.chunks_exact(48) {
        let mut width = 0;
        for x in 0..runs.len() {
            if row[x / 8] & (0x80 >> (x % 8)) != 0 {
                runs[x] += 1;
                width += 1;
                narrow_run = narrow_run.max(runs[x]);
                if width >= sample::BAR_WIDTH {
                    let run = *runs[x + 1 - sample::BAR_WIDTH..=x].iter().min().unwrap();
                    longest = longest.max(run);
                }
            } else {
                runs[x] = 0;
                width = 0
            }
        }
    }
    // Ink bounds come from the resolved, rotated artwork of this record.
    for (key, x, y, w, h, font) in crate::core::quality_regions(svg) {
        let visible = (x.max(0)..(x + w).min(page.len() as i64 / 48)).any(|feed| {
            (y.max(0)..(y + h).min(across as i64)).any(|a| {
                let dot = (384 - across) / 2 + across - 1 - a as usize;
                page[feed as usize * 48 + dot / 8] & (0x80 >> (dot % 8)) != 0
            })
        });
        if visible {
            if key == "text" {
                let (size, family) = font.split_once(':').unwrap_or(("0", ""));
                if family != profile.font && !(family.is_empty() && profile.font == "sans-serif") {
                    return Err("当前字体缺少校准，请手选浓度。".into());
                }
                let size = size.parse::<u32>().unwrap_or(0);
                if size < 12 {
                    return Err("文字小于已测字号，请手选浓度。".into());
                }
                needs.insert(if size < 18 { 5 } else { 7 });
            } else if key == "qr" {
                needs.insert(9);
            } else if key == "barcode" {
                needs.insert(10);
            }
        }
    }
    if needs.is_empty() {
        longest = longest.max(narrow_run);
    }
    if longest > 0 {
        let mm = longest as f64 * 25.4 / media.dpi as f64;
        let Some(i) = [1., 3., 6., 12., 20.].iter().position(|n| mm <= n + 0.06) else {
            return Err("连续黑区超过 20 mm，覆盖不足。".into());
        };
        needs.insert(i);
        if i > 0 && (mm - [1., 3., 6., 12., 20.][i]).abs() > 0.07 {
            needs.insert(i - 1);
        }
    }
    if needs.is_empty() {
        needs.insert(0);
    }
    let mut common = LEVELS.to_vec();
    let mut reasons = vec![];
    for i in needs {
        if LEVELS
            .iter()
            .any(|l| profile.score(*l, ITEMS[i]).is_empty())
        {
            return Err(format!("{} 评分未完整，请手选浓度。", LABELS[i]));
        }
        let good: Vec<_> = LEVELS
            .into_iter()
            .filter(|&l| profile.score(l, ITEMS[i]) == choices(i)[0])
            .collect();
        if good.is_empty() {
            return Err(format!("{} 缺少合格档，请手选浓度。", LABELS[i]));
        }
        common.retain(|l| good.contains(l));
        reasons.push(format!("{} {:?}", LABELS[i], good));
    }
    common
        .first()
        .copied()
        .ok_or_else(|| format!("档位冲突：{}。请手选浓度。", reasons.join("；")))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Profile {
        Profile::new(
            "P2-test".into(),
            Media::for_model(crate::device::Model::P2),
            "测试纸".into(),
            "sans-serif".into(),
        )
        .unwrap()
    }
    #[test]
    fn one_sheet_per_density_matches_physical_feed_and_clean_tails() {
        let p = fixture();
        let l = sample::Layout::build(&p).unwrap();
        assert_eq!(l.count(), 1);
        assert!(l.omitted.is_empty());
        for d in LEVELS {
            let s = l.render(&p, d, 0).unwrap();
            assert_eq!(s.page.len(), 240 * 48);
            let ink = |x: usize, y: usize| {
                let dot = 32 + x;
                s.page[y * 48 + dot / 8] & (128 >> (dot % 8)) != 0
            };
            for (i, mm) in [1, 3, 6, 12, 20].into_iter().enumerate() {
                let x = 4 + i * 38 + 10;
                assert_eq!((20..181).filter(|&y| ink(x, y)).count(), mm * 8);
                assert!((181..189).all(|y| !ink(x, y)));
            }
        }
    }
    #[test]
    fn mixed_content_requires_common_observed_density() {
        let mut p = fixture();
        let mut bytes = vec![0; 240 * 48];
        for row in bytes.chunks_exact_mut(48).take(48) {
            row[20..23].fill(255);
        }
        let svg = "<svg><g data-quality='qr' data-region='0 100 48 100'/></svg>";
        for level in LEVELS {
            p.mark(level, "6mm", if level <= 5 { "合格" } else { "有白点" })
                .unwrap();
            p.mark(
                level,
                "qr",
                if level == 1 {
                    "无法识读"
                } else {
                    "读值正确"
                },
            )
            .unwrap();
        }
        assert_eq!(recommend(&p, &p.media, &bytes, svg), Ok(5));
        p.mark(5, "qr", "无法识读").unwrap();
        assert!(recommend(&p, &p.media, &bytes, svg)
            .unwrap_err()
            .contains("冲突"));
        p.mark(10, "6mm", "").unwrap();
        assert!(recommend(&p, &p.media, &bytes, svg)
            .unwrap_err()
            .contains("未完整"));
    }
}
