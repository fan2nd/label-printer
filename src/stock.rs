use serde::Deserialize;

pub const DPI: u32 = 203;
pub const N1_HEAD_DOTS: u32 = 96;
pub const P2_HEAD_DOTS: u32 = 384;
pub const MAX_MEDIA_LENGTH_DOTS: u32 = 1600;

pub fn dots(mm: u16) -> u32 {
    (u32::from(mm) * DPI * 10 + 127) / 254
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, PartialOrd, Ord)]
pub enum Model {
    #[default]
    N1,
    P2,
}

impl Model {
    pub fn head_dots(self) -> u32 {
        match self {
            Self::N1 => N1_HEAD_DOTS,
            Self::P2 => P2_HEAD_DOTS,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Cut {
    pub rounded: Option<bool>,
    pub combined: Option<bool>,
    pub radius_dots: u32,
    pub feed_margin_mm: u16,
    pub regions_dots: Vec<[u32; 4]>,
}

impl Cut {
    pub fn validate(&self, w: u32, h: u32) -> Result<(), String> {
        let feed = dots(self.feed_margin_mm);
        if feed * 2 >= w {
            return Err("留白超过标签尺寸。".into());
        }
        if self.radius_dots > 0 && self.rounded != Some(true) {
            return Err("设置圆角半径时须标明圆角标签。".into());
        }
        if !self.regions_dots.is_empty()
            && (self.combined != Some(true)
                || self.regions_dots.len() < 2
                || self.regions_dots.len() > 64)
        {
            return Err("组合标签需要 2 至 64 个分区。".into());
        }
        let fits = |[x, y, rw, rh]: [u32; 4]| {
            rw > 0
                && rh > 0
                && x.checked_add(rw).is_some_and(|e| e <= w)
                && y.checked_add(rh).is_some_and(|e| e <= h)
                && self.radius_dots <= rw.min(rh) / 2
        };
        if !fits([0, 0, w, h]) || !self.regions_dots.iter().copied().all(fits) {
            return Err("标签分区或圆角尺寸越界。".into());
        }
        Ok(())
    }

    #[allow(dead_code)] // Used by the renderer; the build script shares only validation.
    pub fn contains(&self, x: u32, y: u32, w: u32, h: u32) -> bool {
        let feed = dots(self.feed_margin_mm);
        if x < feed || x >= w.saturating_sub(feed) {
            return false;
        }
        self.inside_die(x, y, w, h)
    }

    pub fn inside_die(&self, x: u32, y: u32, w: u32, h: u32) -> bool {
        let inside = |[ox, oy, rw, rh]: [u32; 4]| {
            if x < ox || y < oy || x - ox >= rw || y - oy >= rh {
                return false;
            }
            let r = i64::from(self.radius_dots) * 2;
            let px = 2 * i64::from(x - ox) + 1;
            let py = 2 * i64::from(y - oy) + 1;
            let dx = (r - px).max(px - (2 * i64::from(rw) - r)).max(0);
            let dy = (r - py).max(py - (2 * i64::from(rh) - r)).max(0);
            dx * dx + dy * dy <= r * r
        };
        if self.regions_dots.is_empty() {
            inside([0, 0, w, h])
        } else {
            self.regions_dots.iter().copied().any(inside)
        }
    }
}
