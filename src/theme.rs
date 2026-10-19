use gpui_kit::{App, Global, Rgba, rgb};
use leetdesk::appearance::Appearance;

#[derive(Default)]
struct SelectedAppearance(Appearance);
impl Global for SelectedAppearance {}

pub fn current(cx: &App) -> Appearance {
    cx.try_global::<SelectedAppearance>()
        .map_or(Appearance::Dark, |value| value.0)
}

pub fn select(appearance: Appearance, cx: &mut App) {
    cx.set_global(SelectedAppearance(appearance));
}

pub fn color(dark: u32, cx: &App) -> Rgba {
    rgb(current(cx).color(dark))
}
