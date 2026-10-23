use crate::{appearance::Appearance, i18n::UiLanguage, online::Site};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterfaceLanguage {
    #[default]
    Auto,
    Chinese,
    English,
}
impl InterfaceLanguage {
    pub const ALL: [Self; 3] = [Self::Auto, Self::Chinese, Self::English];
    pub fn resolve(self, site: Site) -> UiLanguage {
        match self {
            Self::Auto => UiLanguage::for_site(site),
            Self::Chinese => UiLanguage::Chinese,
            Self::English => UiLanguage::English,
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Chinese => "chinese",
            Self::English => "english",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "跟随站点",
            Self::Chinese => "简体中文",
            Self::English => "English",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditorTheme {
    #[default]
    Red,
    Amber,
    Mono,
}
impl EditorTheme {
    pub const ALL: [Self; 3] = [Self::Red, Self::Amber, Self::Mono];
    pub fn key(self) -> &'static str {
        match self {
            Self::Red => "red",
            Self::Amber => "amber",
            Self::Mono => "mono",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Red => "高对比红色",
            Self::Amber => "高对比琥珀",
            Self::Mono => "高对比单色",
        }
    }
    pub fn color(self, original: u32, appearance: Appearance) -> u32 {
        if self == Self::Red {
            return appearance.color(original);
        }
        if self == Self::Mono {
            return appearance.color(if original == 0xc7c3c9 {
                0xc7c3c9
            } else {
                0xf4f5f7
            });
        }
        let dark = appearance == Appearance::Dark;
        match original {
            0xff8f9c => {
                if dark {
                    0xffd479
                } else {
                    0x694000
                }
            }
            0xffe2e7 => {
                if dark {
                    0xffecc4
                } else {
                    0x503509
                }
            }
            0xffdfcf => {
                if dark {
                    0xffe0a8
                } else {
                    0x653a0b
                }
            }
            0xffbec9 => {
                if dark {
                    0xffdc92
                } else {
                    0x633a00
                }
            }
            0xf4bd9f => {
                if dark {
                    0xffd4ad
                } else {
                    0x6a361a
                }
            }
            0xf4d4db | 0xffd1d9 => {
                if dark {
                    0xf2dfba
                } else {
                    0x59401c
                }
            }
            _ => appearance.color(original),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorPreferences {
    pub font_size: u8,
    pub soft_wrap: bool,
    pub interface_language: InterfaceLanguage,
    /// Empty uses the platform's default monospace font.
    pub font_family: String,
    pub editor_theme: EditorTheme,
}
impl Default for EditorPreferences {
    fn default() -> Self {
        Self {
            font_size: 16,
            soft_wrap: true,
            interface_language: InterfaceLanguage::Auto,
            font_family: String::new(),
            editor_theme: EditorTheme::Red,
        }
    }
}
impl EditorPreferences {
    pub fn normalized(mut self) -> Self {
        self.font_size = self.font_size.clamp(12, 24);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_preferences_keep_site_language_and_manual_overrides_take_priority() {
        let old: EditorPreferences =
            serde_json::from_str(r#"{"font_size":18,"soft_wrap":false}"#).unwrap();
        assert_eq!(old.interface_language, InterfaceLanguage::Auto);
        assert_eq!(old.editor_theme, EditorTheme::Red);
        assert!(old.font_family.is_empty());
        assert_eq!(
            old.interface_language.resolve(Site::Global),
            UiLanguage::English
        );
        assert_eq!(
            InterfaceLanguage::English.resolve(Site::China),
            UiLanguage::English
        );
        assert_eq!(
            InterfaceLanguage::Chinese.resolve(Site::Global),
            UiLanguage::Chinese
        );
    }

    #[test]
    fn editor_themes_remain_readable_on_all_appearances_and_selected_lines() {
        fn luminance(color: u32) -> f64 {
            let linear = |v: u32| {
                let v = f64::from(v & 255) / 255.;
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(color >> 16) + 0.7152 * linear(color >> 8) + 0.0722 * linear(color)
        }
        for appearance in Appearance::ALL {
            for theme in EditorTheme::ALL {
                for token in [
                    0xff8f9c, 0xffe2e7, 0xffdfcf, 0xffbec9, 0xf4bd9f, 0xf4f5f7, 0xf4d4db, 0xffd1d9,
                    0xc7c3c9, 0xeee3e6,
                ] {
                    for background in [0x1b1c23, 0x222027, 0x663341] {
                        let a = luminance(theme.color(token, appearance));
                        let b = luminance(appearance.color(background));
                        let contrast = (a.max(b) + 0.05) / (a.min(b) + 0.05);
                        assert!(
                            contrast >= 4.5,
                            "{appearance:?} {theme:?} {token:x} on {background:x}: {contrast}"
                        );
                    }
                }
            }
        }
    }
}
