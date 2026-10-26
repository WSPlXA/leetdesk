use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Sizable, TitleBar,
    button::{Button, ButtonVariants},
};
use gpui_kit::{App, ClickEvent, Window, WindowOptions, div, prelude::*, px};

/// Keep the native resizable/minimizable style flags, even with custom content.
pub fn options() -> WindowOptions {
    TitleBar::window_options()
}

pub fn title_bar(
    window: &Window,
    cx: &App,
    on_close: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let english = crate::ui::language(cx) == leetdesk::i18n::UiLanguage::English;
    let label = if window.is_maximized() {
        if english {
            "Restore window"
        } else {
            "还原窗口"
        }
    } else if english {
        "Maximize window"
    } else {
        "最大化窗口"
    };
    TitleBar::new()
        .on_close_window(on_close)
        .child(div().flex_1().text_size(px(12.)).child("LeetDesk"))
        .when(cfg!(target_os = "macos"), |bar| {
            bar.child(
                Button::new("maximize-window")
                    .icon(if window.is_maximized() {
                        IconName::WindowRestore
                    } else {
                        IconName::WindowMaximize
                    })
                    .small()
                    .ghost()
                    .tooltip(label)
                    .accessibility_label(label)
                    .on_click(|_, window, _| window.zoom_window()),
            )
        })
}

#[cfg(test)]
mod tests {
    #[test]
    fn custom_titlebar_preserves_native_window_capabilities() {
        let options = super::options();
        let titlebar = options
            .titlebar
            .expect("native style flags require a titlebar");
        assert!(titlebar.appears_transparent);
        assert!(options.is_resizable);
        assert!(options.is_minimizable);
        assert!(options.app_owns_titlebar_drag);
    }
}
