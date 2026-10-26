//! The complete upstream course, served by its owner with the user's own session.
use anyhow::{Context as _, Result};
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonVariants},
};
use gpui_kit::{
    Bounds, Entity, Task, Window, WindowBounds, WindowOptions, div, prelude::*, px, size,
};
use gpui_wry::WebView;
use std::{path::Path, sync::mpsc, time::Duration};

const HOME: &str = "https://labuladong.online/zh/algo/home/";

struct CourseBrowser {
    browser: Option<Entity<WebView>>,
    error: Option<String>,
    current_url: String,
    _poll: Task<()>,
    // The context must outlive its native view. Never extract credentials or
    // inject application IPC. macOS uses the persistent WKWebsiteDataStore.
    _context: wry::WebContext,
}

pub fn open(root: &Path, cx: &mut gpui_kit::App) -> Result<()> {
    let directory = root.join("browser/labuladong");
    std::fs::create_dir_all(&directory)?;
    let mut context = wry::WebContext::new(Some(directory.clone()));
    let last_page = directory.join("last-page.txt");
    let start_url = std::fs::read_to_string(&last_page)
        .ok()
        .filter(|url| course_url(url))
        .unwrap_or_else(|| HOME.into());
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(1280.), px(860.)),
            cx,
        ))),
        window_min_size: Some(size(px(900.), px(600.))),
        ..Default::default()
    };
    gpui_kit::open_window(options, cx, move |window, cx| {
        window.set_window_title("LeetDesk · labuladong 全站课程");
        let (sender, events) = mpsc::channel();
        let raw = wry::WebViewBuilder::new_with_web_context(&mut context)
            .with_url(&start_url)
            .with_on_page_load_handler(move |event, url| {
                if matches!(event, wry::PageLoadEvent::Finished) {
                    let _ = sender.send(url);
                }
            })
            .with_new_window_req_handler(|url, _| {
                if url == "about:blank"
                    || url::Url::parse(&url)
                        .is_ok_and(|url| url.scheme() == "https" && url.host_str().is_some())
                {
                    wry::NewWindowResponse::Allow
                } else {
                    wry::NewWindowResponse::Deny
                }
            })
            .build_as_child(&*window)
            .context("无法打开课程浏览器");
        let (browser, error) = match raw {
            Ok(raw) => (Some(cx.new(|cx| WebView::new(raw, window, cx))), None),
            Err(error) => (None, Some(format!("{error:#}"))),
        };
        cx.new(|cx| {
            let poll = cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(200))
                        .await;
                    let Some(url) = events.try_iter().last() else {
                        continue;
                    };
                    if this
                        .update(cx, |this: &mut CourseBrowser, cx| {
                            if course_url(&url) {
                                // Only remember a course page, never authentication redirects.
                                let _ = leetdesk::store::atomic_write(&last_page, url.as_bytes());
                            }
                            this.current_url = url;
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            });
            CourseBrowser {
                browser,
                error,
                current_url: start_url,
                _poll: poll,
                _context: context,
            }
        })
    })?;
    Ok(())
}

impl Render for CourseBrowser {
    fn render(&mut self, _: &mut Window, cx: &mut gpui_kit::Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .p_2()
                    .child(
                        Button::new("course-back")
                            .label("后退")
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.browser.as_ref().map(|browser| {
                                    browser.update(cx, |view, _| {
                                        let _ = view.back();
                                    })
                                });
                            })),
                    )
                    .child(
                        Button::new("course-forward")
                            .label("前进")
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(browser) = &this.browser {
                                    browser.update(cx, |view, _| {
                                        let _ = view.raw().evaluate_script("history.forward()");
                                    });
                                }
                            })),
                    )
                    .child(
                        Button::new("course-home")
                            .label("全部课程")
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.browser.as_ref().map(|browser| {
                                    browser.update(cx, |view, _| view.load_url(HOME))
                                });
                            })),
                    )
                    .child(
                        Button::new("course-reload")
                            .label("刷新")
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.browser.as_ref().map(|browser| {
                                    browser.update(cx, |view, _| {
                                        let _ = view.raw().evaluate_script("location.reload()");
                                    })
                                });
                            })),
                    )
                    .child(
                        Button::new("course-external")
                            .label("在浏览器打开")
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| cx.open_url(&this.current_url))),
                    )
                    .child("原站在线课程 · 会员内容请在页面内登录"),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .when_some(self.browser.clone(), |el, browser| el.child(browser))
                    .when_some(self.error.clone(), |el, error| el.child(error)),
            )
    }
}

// Do not restore OAuth callback URLs or a destination outside the course.
fn course_url(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str() == Some("labuladong.online")
            && url.path().starts_with("/zh/algo/")
            && url.query().is_none()
            && url.username().is_empty()
            && url.password().is_none()
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn resume_only_course_pages() {
        for url in [
            super::HOME,
            "https://labuladong.online/zh/algo/data-structure-basic/graph-traverse-basic/#bfs",
        ] {
            assert!(super::course_url(url));
        }
        for url in [
            "https://labuladong.online/login?token=secret",
            "https://labuladong.online/zh/algo/home/?code=secret",
            "https://labuladong.online.attacker.com/zh/algo/home/",
            "http://labuladong.online/zh/algo/home/",
        ] {
            assert!(!super::course_url(url));
        }
    }
}
