use crate::{
    core::{self, Field, Kind, Row, Template},
    device::Model,
    storage,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

impl ValueEnum for Model {
    fn value_variants<'a>() -> &'a [Self] {
        &[Self::N1, Self::P2]
    }
    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        Some(clap::builder::PossibleValue::new(match self {
            Self::N1 => "n1",
            Self::P2 => "p2",
        }))
    }
}

#[derive(Parser)]
#[command(name = "label-printer", version, about = "标签排版、点阵预览和打印")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 列出内置耗材规格
    Media,
    /// 列出 N1 USB 串口
    Ports,
    /// 列出桌面端保存的本机模板
    Templates,
    /// 生成单张标签的点阵 PNG
    Preview {
        #[arg(value_enum, ignore_case = true)]
        model: Model,
        output: PathBuf,
        #[command(flatten)]
        content: Content,
    },
    /// 向指定机型发送一张标签
    Print {
        #[arg(value_enum, ignore_case = true)]
        model: Model,
        #[arg(value_enum)]
        transport: Transport,
        /// N1 USB 串口路径，用 ports 查看
        #[arg(long)]
        port: Option<String>,
        /// P2 打印浓度（1～15，默认 1）
        #[arg(long, value_parser = clap::value_parser!(u8).range(1..=15))]
        density: Option<u8>,
        #[command(flatten)]
        content: Content,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Transport {
    Usb,
    Ble,
}

#[derive(Args)]
#[command(group(clap::ArgGroup::new("source").required(true).args(["text","template","saved"])))]
struct Content {
    /// 打印单行原文
    #[arg(long, conflicts_with_all = ["set","seq"])]
    text: Option<String>,
    /// 读取桌面端导出的 JSON 模板
    #[arg(long)]
    template: Option<PathBuf>,
    /// 按名称读取本机模板
    #[arg(long)]
    saved: Option<String>,
    /// 原文模式的纸宽x走纸长（mm）
    #[arg(long, requires = "text")]
    media: Option<String>,
    /// 填写模板变量，可重复使用
    #[arg(long, value_name = "字段=值", value_parser = parse_binding)]
    set: Vec<(String, String)>,
    /// 设置本张模板序号，默认 0001
    #[arg(long)]
    seq: Option<String>,
}

fn parse_binding(pair: &str) -> Result<(String, String), String> {
    let (key, value) = pair.split_once('=').ok_or("--set 需要 字段=值。")?;
    if key.is_empty() || key == "seq" {
        return Err("请使用非空字段名，序号用 --seq 指定。".into());
    }
    Ok((key.into(), value.into()))
}

impl Content {
    fn render(self, model: Model) -> Result<(String, core::Media), String> {
        let Self {
            text,
            template: template_path,
            saved: saved_name,
            media,
            set,
            seq,
        } = self;
        let mut row = Row::new();
        for (key, value) in set {
            if row.insert(key.clone(), value).is_some() {
                return Err(format!("字段 {key} 重复。"));
            }
        }
        let template = match (text, template_path, saved_name) {
            (Some(value), None, None) => {
                let stock = core::builtin_media()
                    .iter()
                    .find(|(m, stock, _)| {
                        *m == model
                            && media.as_ref().is_none_or(|s| {
                                *s == format!("{}x{}", stock.width_mm, stock.length_mm)
                            })
                    })
                    .ok_or("未知纸张规格，请用 media 查看。")?;
                row.insert("内容".into(), value);
                Template {
                    id: uuid::Uuid::new_v4(),
                    name: "CLI".into(),
                    media: stock.1.clone(),
                    elements: vec![Field {
                        id: uuid::Uuid::new_v4(),
                        kind: Kind::Text,
                        text: "{内容}".into(),
                        x: 12,
                        y: 8,
                        w: 1,
                        h: 1,
                        font_family: None,
                        font_size: Some(22),
                        rotation: 0,
                        stroke_dots: 1,
                        filled: false,
                        module_size: None,
                        picture: None,
                    }],
                }
            }
            (None, Some(path), None) => storage::import_template(&path)?,
            (None, None, Some(name)) => {
                if let Some(t) = storage::templates()?.into_iter().find(|t| t.name == name) {
                    t
                } else if let Some(p) = storage::presets().iter().find(|p| p.template.name == name)
                {
                    let mut defaults = p.values.clone();
                    defaults.extend(row);
                    row = defaults;
                    p.template.clone()
                } else {
                    return Err(format!("找不到模板 {name}。"));
                }
            }
            _ => return Err("请选择 --text、--template 或 --saved 作为唯一内容来源。".into()),
        };
        if !model.supports(&template.media) {
            return Err("模板的标签规格与机型不匹配。".into());
        }
        let svg = core::render_svg(&template, &row, seq.as_deref().unwrap_or("0001"))?;
        Ok((svg, template.media))
    }
}

pub fn run() -> Result<(), String> {
    match Cli::parse().command {
        Command::Ports => {
            for (path, name) in crate::printer::usb::ports()? {
                println!("{path}\t{name}");
            }
        }
        Command::Media => {
            for (model, stock, _) in core::builtin_media() {
                println!(
                    "{}\t{}x{}\t{}",
                    model.name(),
                    stock.width_mm,
                    stock.length_mm,
                    stock.name
                );
            }
        }
        Command::Templates => {
            let mut entries = storage::templates()?;
            for p in storage::presets() {
                if !entries.iter().any(|t| t.name == p.template.name) {
                    entries.push(p.template.clone());
                }
            }
            for t in entries {
                println!(
                    "{}\t{}x{} mm\t{} 个元素",
                    t.name,
                    t.media.width_mm,
                    t.media.length_mm,
                    t.elements.len()
                );
            }
        }
        Command::Preview {
            model,
            output,
            content,
        } => {
            let (svg, media) = content.render(model)?;
            let url = model.preview_png(&svg, &media)?;
            let png = STANDARD
                .decode(
                    url.strip_prefix("data:image/png;base64,")
                        .ok_or("无法生成 PNG。")?,
                )
                .map_err(|e| e.to_string())?;
            std::fs::write(output, png).map_err(|e| e.to_string())?;
        }
        Command::Print {
            model,
            transport,
            port,
            density,
            content,
        } => {
            let (svg, media) = content.render(model)?;
            if density.is_some() && model != Model::P2 {
                return Err("--density 仅适用于 P2。".into());
            }
            match (model, transport, port.is_some()) {
                (Model::N1, Transport::Usb, true)
                | (Model::P2, Transport::Usb, false)
                | (Model::N1 | Model::P2, Transport::Ble, false) => (),
                _ => return Err("机型、连接方式和端口组合不匹配，请查看 --help。".into()),
            }
            let page = model.rasterize(&svg, &media)?;
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| e.to_string())?
                .block_on(async {
                    let mut device = model
                        .connect(matches!(transport, Transport::Usb), port)
                        .await?;
                    let sent = device
                        .print_page(&page, density.unwrap_or(crate::p2::DEFAULT_DENSITY))
                        .await;
                    let closed = device.disconnect().await;
                    sent?;
                    println!("{} 已发送 1 张。", model.name());
                    closed.map_err(|e| format!("打印已发送，断开连接失败：{e}"))
                })?;
        }
    }
    Ok(())
}
