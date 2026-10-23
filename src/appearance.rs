use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    Dark,
    White,
    #[default]
    Beige,
}

impl Appearance {
    pub const ALL: [Self; 3] = [Self::Dark, Self::White, Self::Beige];

    pub fn label(self) -> &'static str {
        match self {
            Self::Dark => "黑色",
            Self::White => "白色",
            Self::Beige => "米色",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::White => "white",
            Self::Beige => "beige",
        }
    }

    /// Resolve the existing dark palette into the selected application palette.
    /// UI surfaces, document styles and syntax highlighting share these colors.
    pub fn color(self, dark: u32) -> u32 {
        if self == Self::Dark {
            return dark;
        }
        let beige = self == Self::Beige;
        match dark {
            0x000000 | 0x1b1c23 => {
                if beige {
                    0xf8f0e0
                } else {
                    0xffffff
                }
            }
            0x0a0a0a | 0x23242d => {
                if beige {
                    0xf4e9d5
                } else {
                    0xf5f5f8
                }
            }
            0x101010 | 0x1f2029 | 0x101715 | 0x181818 => {
                if beige {
                    0xf1e4ce
                } else {
                    0xeeeef3
                }
            }
            0x222027 => {
                if beige {
                    0xead8ba
                } else {
                    0xeee5e9
                }
            }
            0x81808b | 0x292929 | 0x536861 => {
                if beige { 0xdecfb5 } else { 0xd7cfd3 }
            }
            0x68606a => {
                if beige {
                    0x89765e
                } else {
                    0x867a83
                }
            }
            0xc8c4cb | 0xa9b0bb | 0xc7c3c9 => 0x50434a,
            0xf4f5f7 | 0xeee3e6 => 0x26191f,
            0xff9da8 | 0x79e6bd => 0x8b1838,
            0x18352a | 0x322630 | 0x203c34 | 0x4b2531 => {
                if beige {
                    0xecd6c0
                } else {
                    0xf4dce5
                }
            }
            0x663341 => {
                if beige {
                    0xd8b59f
                } else {
                    0xeac1d0
                }
            }
            0x231313 => {
                if beige {
                    0xecd0be
                } else {
                    0xffe3e6
                }
            }
            0x102018 => {
                if beige {
                    0xd9dec2
                } else {
                    0xe4eee6
                }
            }
            0x61332f | 0xff928c => 0x9a2030,
            0x285440 | 0xc6ffe8 => 0x23583b,
            0xffcf79 | 0xd5ae74 => 0x754200,
            0x89baff | 0xa8d7ff => 0x244d7d,
            0xd9c0ff => 0x63346f,
            0xff8f9c => 0x95132f,
            0xffe2e7 => 0x5c2534,
            0xffdfcf => 0x713515,
            0xffbec9 => 0x74213c,
            0xf4bd9f => 0x663324,
            0xf4d4db => 0x542d3e,
            0xffd1d9 => 0x73283c,
            _ => dark,
        }
    }
}
