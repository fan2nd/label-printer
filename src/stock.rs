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
