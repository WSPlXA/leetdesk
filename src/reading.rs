//! Reading-first native workspace. The editor entities live across mode changes.
use super::*;
use gpui_kit::{AnyElement, assets::IconName, component::Icon, img};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Clone, Copy, Default, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum WorkspaceMode {
    #[default]
    Study,
    Coding,
}

pub(super) fn load_mode(root: &std::path::Path) -> WorkspaceMode {
    fs::read(root.join("workspace-layout.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn label(zh: &str, en: &str, cx: &App) -> String {
    if ui::language(cx) == leetdesk::i18n::UiLanguage::Chinese {
        zh
    } else {
        en
    }
    .to_owned()
}

fn icon(name: IconName) -> Icon {
    Icon::new(name).size(px(18.))
}

const SECTIONS: [(&str, &str); 6] = [
    ("从大类方法到本题", "Applying the general method"),
    ("为什么这样做是对的", "Why this is correct"),
    ("用一个例子走一遍", "Step-by-step example"),
    ("Go 参考代码", "Go reference code"),
    ("复杂度与易错点", "Complexity and pitfalls"),
    ("大类方法与同类练习", "General method and related practice"),
];

impl Workspace {
    pub(super) fn set_workspace_mode(
        &mut self,
        mode: WorkspaceMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace_mode = mode;
        self.document_view = None;
        self.settings_open = false;
        self.active_section = 0;
        self.pending_section = None;
        self.focus_mode = false;
        if mode == WorkspaceMode::Coding && leetdesk::learning::is_offline_view(&self.filter) {
            self.filter = "全部题目".into();
            self.learning_overview = false;
        }
        if let Err(error) = leetdesk::store::atomic_write(
            &self.store.root.join("workspace-layout.json"),
            &serde_json::to_vec(&mode).unwrap(),
        ) {
            self.notice = format!("布局保存失败：{error}");
        }
        if mode == WorkspaceMode::Coding {
            self.editor.update(cx, |state, cx| state.focus(window, cx));
        } else {
            self.settings_focus.focus(window, cx);
        }
        cx.notify();
    }

    fn reading_toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        static LOGO: OnceLock<Arc<gpui_kit::Image>> = OnceLock::new();
        let logo = LOGO.get_or_init(|| {
            Arc::new(gpui_kit::Image::from_bytes(
                gpui_kit::ImageFormat::Png,
                include_bytes!("../assets/leetdesk-icon.png").to_vec(),
            ))
        });
        let coding = self.workspace_mode == WorkspaceMode::Coding;
        let return_to_code = leetdesk::learning::is_offline_view(&self.filter);
        let running = self.cancellation.is_some();
        div()
            .h(px(56.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .border_b_1()
            .border_color(theme::color(LINE, cx))
            .child(
                div()
                    .w(px(if self.reader_wide { 320. } else { 280. }))
                    .h_full()
                    .flex_shrink_0()
                    .px(px(28.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_r_1()
                    .border_color(theme::color(LINE, cx))
                    .window_control_area(WindowControlArea::Drag)
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
                    .child(img(logo.clone()).size(px(32.)))
                    .child(
                        div()
                            .text_size(px(23.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("LeetDesk"),
                    ),
            )
            .child(
                div().flex().h_full().px_6().gap_4().children(
                    [
                        (WorkspaceMode::Study, "学习", "Study", "mode-study"),
                        (WorkspaceMode::Coding, "编码", "Code", "mode-coding"),
                    ]
                    .into_iter()
                    .map(|(mode, zh, en, id)| {
                        Button::new(id)
                            .ghost()
                            .rounded_none()
                            .h_full()
                            .px_4()
                            .flex()
                            .items_center()
                            .cursor_pointer()
                            .border_b_3()
                            .border_color(theme::color(
                                if self.workspace_mode == mode {
                                    ACCENT
                                } else {
                                    BG
                                },
                                cx,
                            ))
                            .text_size(px(17.))
                            .text_color(theme::color(
                                if self.workspace_mode == mode {
                                    ACCENT
                                } else {
                                    MUTED
                                },
                                cx,
                            ))
                            .when(self.workspace_mode == mode, |el| {
                                el.font_weight(FontWeight::BOLD)
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.set_workspace_mode(mode, window, cx)
                            }))
                            .child(div().text_size(px(17.)).child(label(zh, en, cx)))
                    }),
                ),
            )
            .child(
                Button::new("curriculum-home")
                    .label("基础课程")
                    .ghost()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.set_workspace_mode(WorkspaceMode::Study, window, cx);
                        this.show_categories = true;
                        this.learning_navigation = true;
                        this.query.clear();
                        this.search
                            .update(cx, |state, cx| state.set_value("", window, cx));
                        this.select_learning("@course:start".into(), cx);
                    })),
            )
            .child(
                Button::new("offline-library")
                    .label(label("离线题解", "Walkthroughs", cx))
                    .ghost()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.set_workspace_mode(WorkspaceMode::Study, window, cx);
                        this.show_categories = true;
                        this.learning_navigation = true;
                        this.select_learning("@library".into(), cx);
                    })),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .window_control_area(WindowControlArea::Drag)
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move()),
            )
            .when(coding, |el| {
                el.child(
                    Button::new("favorite")
                        .icon(IconName::Star)
                        .small()
                        .ghost()
                        .tooltip(ui::text("☆ 收藏", cx))
                        .on_click(cx.listener(Self::toggle_favorite)),
                )
                .child(
                    Button::new("review")
                        .icon(IconName::BookOpen)
                        .small()
                        .ghost()
                        .tooltip(label("标记待复习", "Mark for review", cx))
                        .on_click(cx.listener(Self::mark_review)),
                )
                .child(
                    Button::new("focus-code")
                        .icon(IconName::Maximize)
                        .small()
                        .ghost()
                        .tooltip(label("专注编码", "Focus editor", cx))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.focus_mode = !this.focus_mode;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("run")
                        .label(label(
                            if running { "停止" } else { "测试" },
                            if running { "Stop" } else { "Run" },
                            cx,
                        ))
                        .icon(IconName::Play)
                        .small()
                        .ghost()
                        .on_click(cx.listener(|this, _, window, cx| {
                            if let Some(cancel) = &this.cancellation {
                                cancel.store(true, Ordering::Relaxed);
                            } else {
                                this.run_cases(&RunCases, window, cx);
                            }
                        })),
                )
                .child(
                    Button::new("submit")
                        .label(ui::text("提交", cx))
                        .primary()
                        .small()
                        .disabled(
                            running
                                || !self.online_enabled
                                || self.problem().online.is_none()
                                || !self.problem().supports(self.practice_language),
                        )
                        .on_click(
                            cx.listener(|this, _, window, cx| this.submit(&Submit, window, cx)),
                        ),
                )
            })
            .when(!coding, |el| {
                el.child(
                    Button::new("start-coding")
                        .accessibility_label(label(
                            if return_to_code {
                                "返回编码"
                            } else {
                                "开始编码"
                            },
                            if return_to_code {
                                "Back to code"
                            } else {
                                "Start coding"
                            },
                            cx,
                        ))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .text_size(px(16.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(label(
                                    if return_to_code {
                                        "返回编码"
                                    } else {
                                        "开始编码"
                                    },
                                    if return_to_code {
                                        "Back to code"
                                    } else {
                                        "Start coding"
                                    },
                                    cx,
                                ))
                                .child(icon(IconName::ArrowRight)),
                        )
                        .primary()
                        .w(px(148.))
                        .h(px(40.))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.set_workspace_mode(WorkspaceMode::Coding, window, cx)
                        })),
                )
            })
            .child(
                Button::new("settings")
                    .icon(IconName::Settings)
                    .ghost()
                    .mx_3()
                    .tooltip(ui::text("设置", cx))
                    .accessibility_label(ui::text("设置", cx))
                    .on_click(
                        cx.listener(|this, _, window, cx| this.settings(&Settings, window, cx)),
                    ),
            )
            .into_any_element()
    }

    fn reading_filters(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .py_2()
            .child(
                div().flex().gap_1().children(
                    [(true, "按解法"), (false, "按主题")]
                        .into_iter()
                        .map(|(learning, text)| {
                            Button::new(if learning {
                                "navigation-methods"
                            } else {
                                "navigation-topics"
                            })
                            .label(ui::text(text, cx))
                            .small()
                            .ghost()
                            .when(self.learning_navigation == learning, |b| {
                                b.text_color(theme::color(ACCENT, cx))
                            })
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.learning_navigation = learning;
                                    this.filter = "全部题目".into();
                                    this.document_view = None;
                                    cx.notify();
                                },
                            ))
                        }),
                ),
            )
            .child(
                div().flex().flex_wrap().gap_1().children(
                    ["全部", "未解答", "尝试过", "已通过", "收藏", "待复习"]
                        .into_iter()
                        .enumerate()
                        .map(|(index, text)| {
                            Button::new(("status-filter", index))
                                .label(ui::text(if index == 0 { "全部状态" } else { text }, cx))
                                .small()
                                .ghost()
                                .when(self.status_filter == text, |b| {
                                    b.text_color(theme::color(ACCENT, cx))
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.status_filter = text.into();
                                    cx.notify();
                                }))
                        }),
                ),
            )
            .child(
                div().flex().gap_1().children(
                    ["全部", "简单", "中等", "困难"]
                        .into_iter()
                        .enumerate()
                        .map(|(index, text)| {
                            Button::new(("difficulty", index))
                                .label(ui::text(text, cx))
                                .small()
                                .ghost()
                                .when(self.difficulty_filter == text, |b| {
                                    b.text_color(theme::color(ACCENT, cx))
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.difficulty_filter = text.into();
                                    cx.notify();
                                }))
                        }),
                ),
            )
            .child(
                Button::new("clear-filters")
                    .label(ui::text("重置", cx))
                    .small()
                    .ghost()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.filter = "全部题目".into();
                        this.study_plan_filter = "全部题目".into();
                        this.status_filter = "全部".into();
                        this.difficulty_filter = "全部".into();
                        this.search
                            .update(cx, |state, cx| state.set_value("", window, cx));
                        this.query.clear();
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    fn curriculum_tree(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut units: Vec<&str> = Vec::new();
        for course in leetdesk::curriculum::COURSES.iter() {
            if !units.contains(&course.unit.as_str()) {
                units.push(&course.unit);
            }
        }
        div()
            .mb_4()
            .child(
                div()
                    .px_2()
                    .py_2()
                    .font_weight(FontWeight::BOLD)
                    .child("从基础开始 · 原创课程"),
            )
            .children(units.into_iter().map(|unit| {
                let key = format!("course-unit:{unit}");
                let selected = leetdesk::curriculum::selected(&self.filter);
                let open = self.expanded_chapters.contains(&key)
                    || selected.is_some_and(|c| c.unit == unit);
                let toggle_key = key.clone();
                div()
                    .child(
                        Button::new(gpui_kit::SharedString::from(key))
                            .label(unit.to_owned())
                            .ghost()
                            .w_full()
                            .justify_start()
                            .icon(if open {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.expanded_chapters.remove(&toggle_key) {
                                    this.expanded_chapters.insert(toggle_key.clone());
                                }
                                cx.notify();
                            })),
                    )
                    .when(open, |el| {
                        el.children(
                            leetdesk::curriculum::COURSES
                                .iter()
                                .filter(|c| c.unit == unit)
                                .map(|course| {
                                    let route = format!("@course:{}", course.id);
                                    Button::new(gpui_kit::SharedString::from(route.clone()))
                                        .child(
                                            div().w_full().truncate().child(course.title.clone()),
                                        )
                                        .ghost()
                                        .w_full()
                                        .justify_start()
                                        .pl_5()
                                        .tooltip(course.goal.clone())
                                        .when(selected.is_some_and(|c| c.id == course.id), |b| {
                                            b.text_color(theme::color(ACCENT, cx))
                                        })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.select_learning(route.clone(), cx)
                                        }))
                                }),
                        )
                    })
            }))
            .into_any_element()
    }

    fn reading_tree(&self, cx: &mut Context<Self>) -> AnyElement {
        let visible = self.catalog_view.filter_in(
            "全部题目",
            &self.difficulty_filter,
            &self.query,
            &self.study_plan_filter,
            &self.status_filter,
        );
        let visible: HashSet<_> = visible.iter().copied().collect();
        let memberships = leetdesk::learning::memberships(&self.problem().slug);
        let active_chapter = leetdesk::learning::find(&self.filter)
            .or_else(|| {
                memberships
                    .first()
                    .and_then(|p| leetdesk::learning::find(&format!("@pattern:{p}")))
            })
            .map(|(chapter, _)| chapter.id.as_str());
        let mut chapters: Vec<_> = leetdesk::learning::CHAPTERS.iter().collect();
        chapters.sort_by_key(|chapter| Some(chapter.id.as_str()) != active_chapter);
        div()
            .child(self.curriculum_tree(cx))
            .child(
                div()
                    .px_2()
                    .py_2()
                    .font_weight(FontWeight::BOLD)
                    .child("掌握基础后 · 按解法练习"),
            )
            .children(chapters.into_iter().map(|chapter| {
                let expanded = self.expanded_chapters.contains(&chapter.id);
                div()
                    .mb_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .h(px(38.))
                            .child(
                                Button::new(gpui_kit::SharedString::from(format!(
                                    "chapter-toggle-{}",
                                    chapter.id
                                )))
                                .icon(if expanded {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                })
                                .small()
                                .ghost()
                                .accessibility_label(chapter.title.get(ui::language(cx)).to_owned())
                                .tooltip(chapter.overview.get(ui::language(cx)).to_owned())
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        if !this.expanded_chapters.remove(&chapter.id) {
                                            this.expanded_chapters.insert(chapter.id.clone());
                                        }
                                        cx.notify();
                                    },
                                )),
                            )
                            .child(
                                Button::new(gpui_kit::SharedString::from(format!(
                                    "chapter-{}",
                                    chapter.id
                                )))
                                .accessibility_label(chapter.title.get(ui::language(cx)).to_owned())
                                .child(
                                    div()
                                        .w_full()
                                        .text_size(px(16.))
                                        .truncate()
                                        .child(chapter.title.get(ui::language(cx)).to_owned()),
                                )
                                .ghost()
                                .flex_1()
                                .min_w_0()
                                .justify_start()
                                .text_size(px(16.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.select_learning(format!("@chapter:{}", chapter.id), cx)
                                    },
                                )),
                            ),
                    )
                    .when(expanded, |el| {
                        el.children(
                            chapter
                                .patterns
                                .iter()
                                .filter(|pattern| {
                                    self.expanded_patterns.contains(&pattern.id)
                                        || !self
                                            .catalog_view
                                            .learning_indices(&format!("@pattern:{}", pattern.id))
                                            .is_empty()
                                })
                                .map(|pattern| {
                                    let open = self.expanded_patterns.contains(&pattern.id);
                                    let route = format!("@pattern:{}", pattern.id);
                                    let indices: Vec<_> = self
                                        .catalog_view
                                        .learning_indices(&route)
                                        .iter()
                                        .filter(|i| visible.contains(i))
                                        .copied()
                                        .collect();
                                    let count = indices.len();
                                    div()
                                        .pl_6()
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .h(px(36.))
                                                .child(
                                                    Button::new(gpui_kit::SharedString::from(
                                                        format!("pattern-toggle-{}", pattern.id),
                                                    ))
                                                    .icon(if open {
                                                        IconName::ChevronDown
                                                    } else {
                                                        IconName::ChevronRight
                                                    })
                                                    .small()
                                                    .ghost()
                                                    .accessibility_label(
                                                        pattern
                                                            .title
                                                            .get(ui::language(cx))
                                                            .to_owned(),
                                                    )
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        if !this
                                                            .expanded_patterns
                                                            .remove(&pattern.id)
                                                        {
                                                            this.expanded_patterns
                                                                .insert(pattern.id.clone());
                                                        }
                                                        cx.notify();
                                                    })),
                                                )
                                                .child(
                                                    Button::new(gpui_kit::SharedString::from(
                                                        format!("pattern-{}", pattern.id),
                                                    ))
                                                    .accessibility_label(
                                                        pattern
                                                            .title
                                                            .get(ui::language(cx))
                                                            .to_owned(),
                                                    )
                                                    .child(
                                                        div()
                                                            .w_full()
                                                            .text_size(px(15.))
                                                            .truncate()
                                                            .child(
                                                                pattern
                                                                    .title
                                                                    .get(ui::language(cx))
                                                                    .to_owned(),
                                                            ),
                                                    )
                                                    .ghost()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .justify_start()
                                                    .text_size(px(15.))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .tooltip(format!(
                                                        "{} · {count}",
                                                        pattern.summary.get(ui::language(cx))
                                                    ))
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        this.select_learning(
                                                            format!("@pattern:{}", pattern.id),
                                                            cx,
                                                        )
                                                    })),
                                                ),
                                        )
                                        .when(open, |el| {
                                            el.child(div().pl_1().children(
                                                indices.into_iter().map(|index| {
                                                    self.reading_question(
                                                        index,
                                                        format!(
                                                            "route-{}-{}",
                                                            pattern.id, self.problems[index].id
                                                        ),
                                                        Some(pattern.id.clone()),
                                                        cx,
                                                    )
                                                }),
                                            ))
                                        })
                                        .when(open && count == 0, |el| {
                                            el.child(
                                                div()
                                                    .pl_8()
                                                    .py_2()
                                                    .text_xs()
                                                    .text_color(theme::color(MUTED, cx))
                                                    .child(label(
                                                        "当前筛选下没有题目",
                                                        "No matching problems",
                                                        cx,
                                                    )),
                                            )
                                        })
                                }),
                        )
                    })
            }))
            .child(
                Button::new("learning-unclassified")
                    .label(ui::text("未归类题目", cx))
                    .small()
                    .ghost()
                    .mt_2()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.filter = "@unclassified".into();
                        this.document_tab = DocumentTab::Description;
                        this.document_view = None;
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    pub(super) fn reading_question(
        &self,
        index: usize,
        id: String,
        pattern: Option<String>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let problem = &self.problems[index];
        let number = problem
            .online
            .as_ref()
            .map(|m| m.frontend_id.clone())
            .unwrap_or_else(|| problem.id.to_string());
        let active = index == self.current && !self.learning_overview;
        let title = format!("{number}. {}", problem.title);
        let tooltip = title.clone();
        div()
            .id(gpui_kit::SharedString::from(id))
            .test_support()
            .h(px(40.))
            .w_full()
            .flex()
            .items_center()
            .border_l_4()
            .border_color(theme::color(if active { ACCENT } else { BG }, cx))
            .pl(px(if pattern.is_some() { 36. } else { 12. }))
            .pr_3()
            .rounded_r_md()
            .cursor_pointer()
            .text_size(px(16.))
            .tooltip(move |window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
            })
            .when(active, |el| {
                el.bg(theme::color(0x322630, cx))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::color(ACCENT, cx))
            })
            .hover(|el| el.bg(theme::color(0x222027, cx)))
            .child(div().flex_1().min_w_0().truncate().child(title))
            .on_click(cx.listener(move |this, _, window, cx| {
                if let Some(pattern) = &pattern {
                    this.filter = format!("@pattern:{pattern}");
                }
                this.select_problem(index, window, cx);
            }))
            .into_any_element()
    }

    fn reading_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let flat = !self.query.trim().is_empty()
            || !self.learning_navigation
            || self.filter == "@unclassified";
        let indices = self.filtered_indices();
        let count = indices.len();
        let (solved, available) = self.catalog_view.hot_progress();
        div()
            .id("reading-sidebar")
            .test_support()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::color(PANEL, cx))
            .child(
                div()
                    .px(px(18.))
                    .pt_3()
                    .pb_2()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .h(px(32.))
                            .child(
                                div()
                                    .text_size(px(18.))
                                    .font_weight(FontWeight::BOLD)
                                    .child(
                                        if leetdesk::curriculum::selected(&self.filter).is_some()
                                            || leetdesk::learning::is_offline_view(&self.filter)
                                        {
                                            label("课程与题解", "Lessons and walkthroughs", cx)
                                        } else {
                                            label("题单", "Problems", cx)
                                        },
                                    ),
                            )
                            .child(
                                Button::new("show-filters")
                                    .icon(IconName::SlidersHorizontal)
                                    .small()
                                    .ghost()
                                    .tooltip(label("筛选与分组", "Filters and grouping", cx))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.filters_open = !this.filters_open;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .border_1()
                            .border_color(theme::color(LINE, cx))
                            .rounded_md()
                            .child(
                                Input::new(&self.search)
                                    .id("problem-search")
                                    .prefix(icon(IconName::Search))
                                    .bordered(false)
                                    .h(px(40.)),
                            ),
                    )
                    .child(
                        Button::new("plan-menu")
                            .accessibility_label(ui::text(self.study_plan_filter.clone(), cx))
                            .child(
                                div()
                                    .w_full()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .text_size(px(15.))
                                    .child(icon(if self.study_plan_filter == "热题 100" {
                                        IconName::Flame
                                    } else {
                                        IconName::List
                                    }))
                                    .child(ui::text(self.study_plan_filter.clone(), cx))
                                    .child(div().flex_1())
                                    .child(icon(IconName::ChevronDown)),
                            )
                            .w_full()
                            .h(px(40.))
                            .justify_start()
                            .outline()
                            .tooltip(format!("{solved}/{available}"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.plan_menu_open = !this.plan_menu_open;
                                cx.notify();
                            })),
                    )
                    .when(self.plan_menu_open || self.filters_open, |el| {
                        el.child(
                            div().flex().gap_1().children(
                                ["全部题目", "热题 100"].into_iter().enumerate().map(
                                    |(i, text)| {
                                        Button::new(("study-plan", i))
                                            .label(ui::text(text, cx))
                                            .small()
                                            .ghost()
                                            .when(self.study_plan_filter == text, |b| {
                                                b.text_color(theme::color(ACCENT, cx))
                                            })
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.study_plan_filter = text.into();
                                                this.plan_menu_open = false;
                                                cx.notify();
                                            }))
                                    },
                                ),
                            ),
                        )
                    })
                    .when(self.filters_open, |el| el.child(self.reading_filters(cx))),
            )
            .when(!flat, |el| {
                el.child(
                    div()
                        .id("category-scroll")
                        .test_support()
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .px_3()
                        .pt_2()
                        .pb_6()
                        .child(self.reading_tree(cx)),
                )
            })
            .when(flat, |el| {
                el.when(!self.learning_navigation, |el| {
                    el.child(
                        div()
                            .id("category-scroll")
                            .h(px(130.))
                            .overflow_y_scroll()
                            .px_3()
                            .child(self.topic_row("全部题目", false, cx))
                            .children(
                                self.catalog_view
                                    .topics()
                                    .iter()
                                    .map(|t| self.topic_row(t, false, cx)),
                            )
                            .children(
                                self.catalog_view
                                    .custom_tags()
                                    .iter()
                                    .map(|t| self.topic_row(t, true, cx)),
                            ),
                    )
                })
                .child(
                    div()
                        .px_5()
                        .py_2()
                        .text_xs()
                        .text_color(theme::color(MUTED, cx))
                        .child(format!("{count} {}", label("道题", "problems", cx))),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .when(count > 0, |el| {
                            el.child(
                                uniform_list(
                                    "problem-list",
                                    count,
                                    cx.processor(
                                        move |this, range: std::ops::Range<usize>, _, cx| {
                                            range
                                                .filter_map(|r| indices.get(r).copied())
                                                .map(|i| {
                                                    this.question_row(i, cx).into_any_element()
                                                })
                                                .collect::<Vec<_>>()
                                        },
                                    ),
                                )
                                .size_full(),
                            )
                        })
                        .when(count == 0, |el| {
                            el.child(div().p_5().text_sm().child(ui::text("没有匹配题目", cx)))
                        }),
                )
            })
            .when(self.problem().online.is_none(), |el| {
                el.child(
                    Button::new("import-catalog")
                        .label(ui::text("导入题目 JSON", cx))
                        .small()
                        .ghost()
                        .on_click(cx.listener(Self::import_catalog)),
                )
            })
            .into_any_element()
    }

    pub(super) fn reader_heading(&self, cx: &mut Context<Self>) -> AnyElement {
        if let Some(course) = leetdesk::curriculum::selected(&self.filter) {
            return div()
                .pt_4()
                .text_size(px(13.))
                .text_color(theme::color(MUTED, cx))
                .child(format!("{} · 先理解，再练习", course.unit))
                .into_any_element();
        }
        if leetdesk::learning::is_offline_view(&self.filter) {
            let title = leetdesk::learning::selected_article(&self.filter).map_or_else(
                || label("离线题解", "Offline walkthroughs", cx),
                |article| leetdesk::learning::article_title(article.slug, ui::language(cx)),
            );
            return div()
                .flex_shrink_0()
                .pt_4()
                .pb_1()
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(theme::color(MUTED, cx))
                        .child(label(
                            "方法 · 推演 · Go 代码",
                            "Method · Trace · Go code",
                            cx,
                        )),
                )
                .child(
                    div()
                        .mt_1()
                        .text_size(px(if self.reader_wide { 40. } else { 32. }))
                        .line_height(px(if self.reader_wide { 48. } else { 42. }))
                        .font_weight(FontWeight::BOLD)
                        .child(title),
                )
                .into_any_element();
        }
        let membership = leetdesk::learning::memberships(&self.problem().slug);
        let context = leetdesk::learning::find(&self.filter).or_else(|| {
            membership
                .first()
                .and_then(|p| leetdesk::learning::find(&format!("@pattern:{p}")))
        });
        let breadcrumb = context
            .map(|(chapter, pattern)| {
                format!(
                    "{}  /  {}",
                    chapter.title.get(ui::language(cx)),
                    pattern
                        .map(|p| p.title.get(ui::language(cx)))
                        .unwrap_or_else(|| {
                            if ui::language(cx) == leetdesk::i18n::UiLanguage::Chinese {
                                "方法总览"
                            } else {
                                "Overview"
                            }
                        })
                )
            })
            .unwrap_or_else(|| label("自主练习", "Independent practice", cx));
        let title = if self.learning_overview {
            leetdesk::learning::title(&self.filter, ui::language(cx))
                .unwrap_or(&breadcrumb)
                .to_owned()
        } else {
            let number = self
                .problem()
                .online
                .as_ref()
                .map(|m| m.frontend_id.clone())
                .unwrap_or_else(|| self.problem().id.to_string());
            format!("{number}. {}", self.problem().title)
        };
        div()
            .flex_shrink_0()
            .pt_4()
            .pb_1()
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(theme::color(MUTED, cx))
                    .child(breadcrumb),
            )
            .child(
                div()
                    .mt_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(if self.reader_wide { 40. } else { 32. }))
                            .line_height(px(if self.reader_wide { 48. } else { 42. }))
                            .font_weight(FontWeight::BOLD)
                            .child(title),
                    )
                    .child(
                        Button::new("favorite")
                            .icon(IconName::Star)
                            .small()
                            .ghost()
                            .tooltip(ui::text(
                                if self.progress.favorite {
                                    "★ 收藏"
                                } else {
                                    "☆ 收藏"
                                },
                                cx,
                            ))
                            .when(self.progress.favorite, |b| {
                                b.text_color(theme::color(ACCENT, cx))
                            })
                            .on_click(cx.listener(Self::toggle_favorite)),
                    )
                    .child(
                        Button::new("review")
                            .icon(IconName::BookOpen)
                            .small()
                            .ghost()
                            .tooltip(label("标记待复习", "Mark for review", cx))
                            .on_click(cx.listener(Self::mark_review)),
                    ),
            )
            .into_any_element()
    }

    fn reading_outline(&self, cx: &mut Context<Self>) -> AnyElement {
        let course = leetdesk::curriculum::selected(&self.filter).is_some();
        let show = self.document_tab == DocumentTab::Explanation
            && (course
                || !self.learning_overview
                || leetdesk::learning::selected_article(&self.filter).is_some());
        let sections: Vec<_> = if course {
            [
                "学习目标",
                "先修知识",
                "核心概念",
                "过程推演",
                "Go 实验",
                "复杂度与边界",
                "自己试一试",
                "继续学习",
            ]
            .into_iter()
            .map(|s| (s, s))
            .collect()
        } else {
            SECTIONS.to_vec()
        };
        div()
            .id("lesson-outline")
            .test_support()
            .h_full()
            .border_l_1()
            .border_color(theme::color(LINE, cx))
            .px(px(24.))
            .pt(px(44.))
            .text_size(px(14.))
            .child(div().font_weight(FontWeight::BOLD).mb_3().child(label(
                if course {
                    "本课目录"
                } else {
                    "本题讲解"
                },
                "On this page",
                cx,
            )))
            .when(show, |el| {
                el.children(sections.into_iter().enumerate().map(|(index, (zh, en))| {
                    let heading = label(zh, en, cx);
                    div()
                        .id(("lesson-section", index))
                        .test_support()
                        .min_h(px(36.))
                        .pl_4()
                        .py_2()
                        .border_l_2()
                        .border_color(theme::color(
                            if self.active_section == index {
                                ACCENT
                            } else {
                                LINE
                            },
                            cx,
                        ))
                        .text_color(theme::color(
                            if self.active_section == index {
                                ACCENT
                            } else {
                                MUTED
                            },
                            cx,
                        ))
                        .when(self.active_section == index, |el| {
                            el.font_weight(FontWeight::SEMIBOLD)
                        })
                        .cursor_pointer()
                        .hover(|el| el.text_color(theme::color(INK, cx)))
                        .child(heading.clone())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.pending_section = Some(heading.clone());
                            this.active_section = index;
                            this.reveal_section(cx);
                            cx.notify();
                        }))
                }))
            })
            .when(!show, |el| {
                el.child(div().text_color(theme::color(MUTED, cx)).child(label(
                    "选择学习讲解查看目录",
                    "Open the lesson to see its outline",
                    cx,
                )))
            })
            .into_any_element()
    }

    pub(super) fn reveal_section(&mut self, cx: &mut Context<Self>) {
        if let (Some(heading), Some((_, state, _))) = (&self.pending_section, &self.document_view)
            && let Some(start) = state.read(cx).rendered_text().as_str().find(heading)
        {
            state.update(cx, |state, cx| {
                let _ = state.reveal_range(start..start, cx);
            });
            self.pending_section = None;
        }
    }

    pub(super) fn reading_workspace(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.reader_wide = window.viewport_size().width >= px(1400.);
        let study = self.workspace_mode == WorkspaceMode::Study;
        let sidebar_width = if self.reader_wide { 320. } else { 280. };
        let body = if self.settings_open {
            div()
                .size_full()
                .child(self.settings_panel(cx))
                .into_any_element()
        } else if study {
            let document = self.document(cx).into_any_element();
            self.reveal_section(cx);
            div()
                .size_full()
                .flex()
                .when(self.show_categories, |el| {
                    el.child(
                        div()
                            .w(px(sidebar_width))
                            .h_full()
                            .flex_shrink_0()
                            .border_r_1()
                            .border_color(theme::color(LINE, cx))
                            .child(self.reading_sidebar(cx)),
                    )
                })
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .pl(px(if self.reader_wide { 80. } else { 32. }))
                        .pr_6()
                        .child(document),
                )
                .when(window.viewport_size().width >= px(1250.), |el| {
                    el.child(
                        div()
                            .w(px(if self.reader_wide { 248. } else { 200. }))
                            .flex_shrink_0()
                            .h_full()
                            .child(self.reading_outline(cx)),
                    )
                })
                .into_any_element()
        } else {
            let right = v_resizable("editor-results")
                .child(
                    resizable_panel()
                        .size(px(510.))
                        .size_range(px(180.)..px(1600.))
                        .child(self.editor_panel(cx)),
                )
                .child(
                    resizable_panel()
                        .size(px(280.))
                        .size_range(px(140.)..px(1200.))
                        .child(self.bottom_panel(cx)),
                );
            let mut panels = h_resizable("reading-code-v2");
            if self.show_categories && !self.focus_mode {
                panels = panels.child(
                    resizable_panel()
                        .size(px(280.))
                        .size_range(px(240.)..px(400.))
                        .child(self.reading_sidebar(cx)),
                );
            }
            if !self.focus_mode {
                panels = panels.child(
                    resizable_panel()
                        .size(px(430.))
                        .size_range(px(300.)..px(1500.))
                        .child(self.document(cx)),
                );
            }
            panels
                .child(
                    resizable_panel()
                        .size(px(700.))
                        .size_range(px(400.)..px(3000.))
                        .child(right),
                )
                .into_any_element()
        };
        let position = self.editor.read(cx).cursor_position();
        div()
            .key_context("LeetDesk")
            .when(!self.settings_open, |el| {
                el.track_focus(&self.settings_focus)
            })
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::color(BG, cx))
            .text_color(theme::color(INK, cx))
            .font_family(gpui_kit::component::Theme::global(cx).font_family.clone())
            .on_action(cx.listener(Self::save_action))
            .on_action(cx.listener(Self::run_cases))
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::format))
            .on_action(cx.listener(Self::import_notes))
            .on_action(cx.listener(Self::organize_imports))
            .on_action(cx.listener(Self::toggle_wrap))
            .on_action(cx.listener(Self::settings))
            .on_action(cx.listener(|this, _: &StudyMode, window, cx| {
                this.set_workspace_mode(WorkspaceMode::Study, window, cx)
            }))
            .on_action(cx.listener(|this, _: &CodingMode, window, cx| {
                this.set_workspace_mode(WorkspaceMode::Coding, window, cx)
            }))
            .on_action(cx.listener(Self::search_problems))
            .when(self.owns_window_chrome, |el| {
                el.child(crate::window_chrome::title_bar(
                    window,
                    cx,
                    cx.listener(|this, _, window, cx| {
                        if this.confirm_close(cx) {
                            window.remove_window();
                        }
                    }),
                ))
            })
            .child(self.reading_toolbar(cx))
            .child(div().flex_1().min_h_0().child(body))
            .child(
                div()
                    .h(px(36.))
                    .flex_shrink_0()
                    .px_5()
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_t_1()
                    .border_color(theme::color(LINE, cx))
                    .child(
                        Button::new("toggle-categories")
                            .icon(IconName::PanelLeft)
                            .ghost()
                            .small()
                            .tooltip(label("显示 / 隐藏题单", "Toggle problems", cx))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_categories = !this.show_categories;
                                cx.notify();
                            })),
                    )
                    .child(icon(IconName::FileText))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(theme::color(MUTED, cx))
                            .child(label(
                                if self.dirty || self.note_dirty {
                                    "正在保存…"
                                } else {
                                    "已保存 · 讲解可离线阅读"
                                },
                                if self.dirty || self.note_dirty {
                                    "Saving…"
                                } else {
                                    "Saved · Lessons available offline"
                                },
                                cx,
                            )),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(theme::color(MUTED, cx))
                            .child(
                                if study
                                    && !self.notice.contains("失败")
                                    && !self.notice.contains("错误")
                                {
                                    String::new()
                                } else {
                                    ui::text(self.notice.clone(), cx)
                                },
                            ),
                    )
                    .when(!study, |el| {
                        el.child(
                            Button::new("reconnect")
                                .label(ui::text(
                                    if self.language.is_some() {
                                        "Go 检查已连接"
                                    } else {
                                        "连接 Go 检查"
                                    },
                                    cx,
                                ))
                                .small()
                                .ghost()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.request_current(window, cx)
                                })),
                        )
                        .child(div().text_xs().child(format!(
                            "Ln {}, Col {}",
                            position.line + 1,
                            position.character + 1
                        )))
                    })
                    .child(
                        Button::new("platform")
                            .icon(IconName::ExternalLink)
                            .small()
                            .ghost()
                            .tooltip(ui::text("题目网页", cx))
                            .on_click({
                                let link = format!(
                                    "{}/problems/{}/",
                                    self.store.site().origin(),
                                    self.problem().slug
                                );
                                move |_, _, cx| cx.open_url(&link)
                            }),
                    )
                    .child(
                        Button::new("footer-language")
                            .label(self.practice_language.label())
                            .icon(IconName::ChevronDown)
                            .ghost()
                            .small()
                            .tooltip(label(
                                "语言与编辑器设置",
                                "Language and editor settings",
                                cx,
                            ))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.settings(&Settings, window, cx)
                            })),
                    ),
            )
            .into_any_element()
    }
}
