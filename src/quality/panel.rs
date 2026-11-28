use super::*;
use crate::ui::Connection;
use dioxus::prelude::*;
#[component]
pub fn QualityPanel(
    media: Media,
    device: String,
    connection: Signal<Option<Connection>>,
    active: Signal<Option<Profile>>,
    open: Signal<bool>,
    notice: Signal<(String, bool)>,
    busy: Signal<bool>,
    density: Signal<u8>,
) -> Element {
    let mut all = use_signal(Vec::<Profile>::new);
    use_effect(move || match profiles() {
        Ok(v) => all.set(v),
        Err(e) => notice.set((e, true)),
    });
    let initial = active.peek().clone().filter(|p| p.media == media);
    let mut base = use_signal(|| initial.clone());
    let mut draft = use_signal(|| {
        initial.unwrap_or_else(|| Profile {
            id: Uuid::new_v4(),
            version: 1,
            device: device.clone(),
            media: media.clone(),
            material: String::new(),
            font: "sans-serif".into(),
            scores: Default::default(),
        })
    });
    let mut rating = use_signal(|| false);
    let mut group = use_signal(|| 0usize);
    let mut level = use_signal(|| 1u8);
    let mut sheet = use_signal(|| 0usize);
    let mut progress = use_signal(String::new);
    let mut show_image = use_signal(|| false);
    let mut sent = use_signal(BTreeSet::<(u8, usize)>::new);
    let mut uncertain = use_signal(|| None::<(u8, usize)>);
    let plan = use_memo(move || sample::Layout::build(&draft()).map(std::rc::Rc::new));
    let preview = use_memo(move || {
        plan()
            .and_then(|p| p.render(&draft(), level(), sheet()))
            .map(|s| s.preview)
    });
    let connected_name = device.clone();
    let save_record = move |enable: bool| {
        let p = draft();
        if p.material.trim().is_empty() || p.device.trim().is_empty() {
            notice.set(("请填写设备标识和纸材名称。".into(), true));
            return;
        }
        let result = save(&p, base.peek().as_ref());
        match result {
            Ok(()) => {
                base.set(Some(p.clone()));
                if enable {
                    if let Err(e) = remember(&connected_name, p.media.id, Some(p.id)) {
                        notice.set((e, true));
                        return;
                    }
                }
                if active.peek().as_ref().is_some_and(|a| a.id == p.id) || enable {
                    active.set(Some(p))
                }
                if enable {
                    density.set(0);
                    open.set(false)
                } else {
                    progress.set("已保存".into());
                    if let Ok(v) = profiles() {
                        all.set(v)
                    }
                }
            }
            Err(e) => notice.set((e, true)),
        }
    };
    let mut save_only = save_record.clone();
    let mut enable = save_record;
    let mut print_set = move |one: bool| {
        if busy() {
            return;
        }
        let Some(conn) = connection() else {
            notice.set(("请先连接 P2。".into(), true));
            return;
        };
        let layout = match plan() {
            Ok(l) => l,
            Err(e) => {
                notice.set((e, true));
                return;
            }
        };
        let profile = draft();
        let target = (level(), sheet());
        let jobs: Vec<_> = LEVELS
            .into_iter()
            .flat_map(|l| (0..layout.count()).map(move |s| (l, s)))
            .filter(|job| !one || *job == target)
            .collect();
        if !one {
            sent.set(BTreeSet::new());
            uncertain.set(None);
        }
        busy.set(true);
        spawn(async move {
            let mut device = conn.lock().await;
            for (l, i) in jobs {
                progress.set(format!("正在发送 D{l} · 第 {} 页", i + 1));
                let s = match layout.render(&profile, l, i) {
                    Ok(s) => s,
                    Err(e) => {
                        notice.set((e, true));
                        break;
                    }
                };
                if let Err(e) = device.print_page(&s.page, l).await {
                    uncertain.set(Some((l, i)));
                    progress.set(format!("D{l} 第 {} 页待核对", i + 1));
                    notice.set((e, true));
                    break;
                }
                sent.write().insert((l, i));
                if uncertain() == Some((l, i)) {
                    uncertain.set(None);
                }
                progress.set(format!("已发送 {} 张", sent.read().len()));
            }
            busy.set(false);
        });
    };
    let mut print_one = print_set;
    let p = draft();
    let count = plan().map(|p| p.count()).unwrap_or(0);
    let missing = plan().map(|p| p.omitted.clone()).unwrap_or_default();
    let groups = ["黑条", "字体", "机器码"];
    let range = [0..5, 5..9, 9..11][group()].clone();
    let completed = p.scores.values().map(|r| r.len()).sum::<usize>();
    let key = (level(), sheet());
    let status = if uncertain() == Some(key) {
        "待核对"
    } else if sent.read().contains(&key) {
        "已发送"
    } else {
        "未发送"
    };
    rsx! {
      div {class:"quality-view",role:"region","aria-label":"打印质量校准",
        header {h2 {"打印质量校准"} button {disabled:busy(),onclick:move |_|open.set(false),"返回编辑器"}}
        div {class:"actions",button {disabled:busy(),"aria-pressed":!rating(),onclick:move |_|rating.set(false),"样张"}button{disabled:busy(),"aria-pressed":rating(),onclick:move |_|rating.set(true),"评价样张"} span {"{media.name}"}}
        if !rating(){
          div {class:"quality-fields",
            label {"纸材记录" select {disabled:busy(),value:base().map(|p|p.id.to_string()).unwrap_or_default(),onchange:move|e|{
                if let Some(p)=all.read().iter().find(|p|p.id.to_string()==e.value()).cloned(){draft.set(p.clone());base.set(Some(p));}
                else{let mut p=draft();p.id=Uuid::new_v4();p.material.clear();p.scores.clear();draft.set(p);base.set(None);}
                sent.set(BTreeSet::new());uncertain.set(None);sheet.set(0);
            },option {value:"","新记录"} for record in all().into_iter().filter(|v|v.media==media){option{value:"{record.id}","{record.material} · {record.device}"}}}}
            label {"设备标识" input{disabled:busy()||base().is_some(),value:"{p.device}",oninput:move|e|draft.write().device=e.value()}}
            label {"纸材名称" input{disabled:busy(),value:"{p.material}",oninput:move|e|draft.write().material=e.value()}}
            label {"样张字体" select{disabled:busy(),value:"{p.font}",onchange:move|e|{
                let mut p=draft.write();p.font=e.value();for row in p.scores.values_mut(){for key in &ITEMS[5..9]{row.remove(*key);}}
            },option{value:"sans-serif","编辑器默认"}for f in crate::raster::font_families(){option{value:"{f}","{f}"}}}}
          }
        }
        div {class:"actions",for l in LEVELS {button {disabled:busy(),"aria-pressed":level()==l,onclick:move |_|level.set(l),"D{l}"}}}
        div {class:"quality-body",
          div {class:if rating()&&!show_image(){"quality-paper narrow-hide"}else{"quality-paper"},
            match preview(){Ok(url)=>rsx!{img{src:url,alt:"先出纸边在上方的校准样张"}},Err(e)=>rsx!{p{"{e}"}}}
          }
          if rating(){div {class:if show_image(){"quality-rating narrow-hide"}else{"quality-rating"},
            div {class:"actions",for (i,g) in groups.iter().enumerate(){button{"aria-pressed":group()==i,onclick:move |_|group.set(i),"{g}"}}}
            for i in range.clone().filter(|i|!missing.contains(i)){
                label {class:"quality-mark","{LABELS[i]}" select {value:p.score(level(),ITEMS[i]),onchange:move|e|{let _=draft.write().mark(level(),ITEMS[i],&e.value());},option{value:"","未评"}for &v in choices(i){option{value:v,"{v}"}}}}
            }
            button {onclick:move |_|{for i in range.clone().filter(|i|!missing.contains(i)){if draft.peek().score(level(),ITEMS[i]).is_empty(){let _=draft.write().mark(level(),ITEMS[i],choices(i)[0]);}}},"其余未评项合格"}
          }}
        }
        if rating(){button{class:"quality-toggle",onclick:move |_|show_image.toggle(),if show_image(){"返回评分"}else{"看样张"}}}
        if count>1{div{class:"actions",button{disabled:sheet()==0,onclick:move |_|sheet-=1,"上一版"}span{"第 {sheet()+1} / {count} 版"}button{disabled:sheet()+1>=count,onclick:move |_|sheet+=1,"下一版"}}}
        if let Ok(l)=plan(){if !l.omitted.is_empty(){small{"本规格放不下：{l.omitted.iter().map(|i|LABELS[*i]).collect::<Vec<_>>().join(\"、\")}"} }}
        if !rating(){small{"D{level()} · {status}　{progress}"}}
        footer {if rating(){span{"已评 {completed} 项"} button{onclick:move |_|save_only(false),"保存"}button{class:"primary",onclick:move |_|enable(true),"保存并启用"}}
            else{button{disabled:busy()||connection().is_none()||count==0,onclick:move |_|print_one(true),"打印当前样张"}button{class:"primary",disabled:busy()||connection().is_none()||count==0,onclick:move |_|print_set(false),"打印 {count*4} 张"}}}
      }
    }
}
