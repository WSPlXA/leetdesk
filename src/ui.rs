use gpui_kit::{App, Global};
use leetdesk::preferences::InterfaceLanguage;
use leetdesk::{i18n::UiLanguage, online::Site};

#[derive(Default)]
struct Locale {
    site: Site,
    preference: InterfaceLanguage,
}
impl Global for Locale {}

pub fn language(cx: &App) -> UiLanguage {
    cx.try_global::<Locale>()
        .map_or(UiLanguage::Chinese, |locale| {
            locale.preference.resolve(locale.site)
        })
}

pub fn set_site(site: Site, cx: &mut App) {
    let preference = preference(cx);
    configure(site, preference, cx);
}

pub fn preference(cx: &App) -> InterfaceLanguage {
    cx.try_global::<Locale>()
        .map_or(InterfaceLanguage::Auto, |locale| locale.preference)
}

pub fn configure(site: Site, preference: InterfaceLanguage, cx: &mut App) {
    cx.set_global(Locale { site, preference });
    cx.refresh_windows();
}

pub fn text(value: impl AsRef<str>, cx: &App) -> String {
    language(cx).text(value.as_ref()).into_owned()
}
