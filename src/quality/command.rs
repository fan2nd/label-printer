use super::*;
use crate::device::Model;
use clap::{Args, Subcommand};
use std::path::PathBuf;
#[derive(Args)]
pub struct QualityArgs {
    #[command(subcommand)]
    action: Action,
}
#[derive(Subcommand)]
enum Action {
    /// 列出纸材记录、UUID 和评分
    List,
    /// 建立 P2 与纸材的校准记录
    New {
        #[arg(long)]
        device: String,
        #[arg(long)]
        material: String,
        #[arg(long, default_value = "40x30")]
        media: String,
        #[arg(long, default_value = "sans-serif")]
        font: String,
    },
    /// 查看样张项目的字段名及可用评价
    Items,
    /// 按当前纸材评价分析一张标签
    Recommend {
        profile: Uuid,
        #[command(flatten)]
        content: crate::cli::Content,
    },
    /// 设置一个样张项目的评价
    Rate {
        profile: Uuid,
        level: u8,
        item: String,
        value: String,
    },
    /// 导出样张点阵 PNG
    Preview {
        profile: Uuid,
        output: PathBuf,
        #[arg(long, default_value_t = 1)]
        density: u8,
        #[arg(long, default_value_t = 1)]
        sheet: usize,
    },
    /// 打印四档样张，或只补打指定档
    Print {
        profile: Uuid,
        #[arg(value_enum)]
        transport: crate::cli::Transport,
        #[arg(long)]
        density: Option<u8>,
        #[arg(long)]
        sheet: Option<usize>,
    },
}
pub fn get(id: Uuid) -> Result<Profile, String> {
    profiles()?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or("找不到校准记录。".into())
}
impl QualityArgs {
    pub fn run(self) -> Result<(), String> {
        match self.action {
            Action::Items => {
                for (i, key) in ITEMS.iter().enumerate() {
                    println!("{key}\t{}\t{}", LABELS[i], choices(i).join(" / "));
                }
            }
            Action::Recommend { profile, content } => {
                let (svg, m) = content.render(Model::P2)?;
                let p = get(profile)?;
                let d = recommend(&p, &m, &Model::P2.rasterize(&svg, &m)?, &svg)?;
                println!("推荐浓度 {d}");
            }
            Action::List => {
                for p in profiles()? {
                    println!(
                        "{}  {}  {}  {}  {:?}",
                        p.id, p.device, p.media.name, p.material, p.scores
                    )
                }
            }
            Action::New {
                device,
                material,
                media,
                font,
            } => {
                let m = crate::core::builtin_media()
                    .iter()
                    .map(|(model, m, _)| (*model, m.clone()))
                    .chain(storage::media()?.into_iter().map(|m| (Model::P2, m)))
                    .find(|(model, m)| {
                        *model == Model::P2
                            && (format!("{}x{}", m.width_mm, m.length_mm) == media
                                || m.id.to_string() == media)
                    })
                    .ok_or("找不到 P2 纸张。")?
                    .1;
                let p = Profile::new(device, m, material, font)?;
                let plan = sample::Layout::build(&p)?;
                save(&p, None)?;
                println!("{} · {} 张", p.id, plan.count() * 4);
            }
            Action::Rate {
                profile,
                level,
                item,
                value,
            } => {
                let base = get(profile)?;
                let mut p = base.clone();
                p.mark(level, &item, &value)?;
                save(&p, Some(&base))?;
            }
            Action::Preview {
                profile,
                output,
                density,
                sheet,
            } => {
                use base64::Engine;
                let p = get(profile)?;
                let plan = sample::Layout::build(&p)?;
                if !LEVELS.contains(&density) {
                    return Err("请选择样张浓度 1、5、10、15。".into());
                }
                let png = plan
                    .render(
                        &p,
                        density,
                        sheet.checked_sub(1).ok_or("样张页码从 1 开始。")?,
                    )?
                    .preview;
                let data = base64::engine::general_purpose::STANDARD
                    .decode(png.split_once(',').ok_or("预览格式错误")?.1)
                    .map_err(|e| e.to_string())?;
                std::fs::write(output, data).map_err(|e| e.to_string())?;
            }
            Action::Print {
                profile,
                transport,
                density,
                sheet,
            } => {
                let p = get(profile)?;
                let plan = sample::Layout::build(&p)?;
                if density.is_some_and(|l| !LEVELS.contains(&l))
                    || sheet.is_some_and(|s| s == 0 || s > plan.count())
                {
                    return Err("无此浓度或分版。".into());
                }
                println!(
                    "每档 {} 页，未覆盖项目 {:?}",
                    plan.count(),
                    plan.omitted.iter().map(|i| LABELS[*i]).collect::<Vec<_>>()
                );
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| e.to_string())?
                    .block_on(async {
                        let mut d = Model::P2
                            .connect(matches!(transport, crate::cli::Transport::Usb), None)
                            .await?;
                        let result = async {
                            for l in LEVELS
                                .into_iter()
                                .filter(|l| density.is_none_or(|x| x == *l))
                            {
                                for i in
                                    (0..plan.count()).filter(|i| sheet.is_none_or(|x| x == i + 1))
                                {
                                    let page = plan.render(&p, l, i)?.page;
                                    d.print_page(&page, l).await.map_err(|e| {
                                        format!("D{l} 第 {} 页待核对，后续未发送：{e}", i + 1)
                                    })?;
                                    println!("D{l} 第 {} 页已发送", i + 1);
                                }
                            }
                            Ok::<(), String>(())
                        }
                        .await;
                        let closed = d.disconnect().await;
                        result.and(closed)
                    })?;
            }
        }
        Ok(())
    }
}
