use crate::{theme, ui};
use anyhow::{Result, anyhow};
use gpui_kit::base::{Disableable, TestSupportExt, TextView, TextViewState, TextViewStyle};
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonVariants},
    h_resizable,
    input::{
        CompletionProvider, DefinitionProvider, Editor, EditorState, HoverProvider, Input,
        InputEvent, InputState, Rope, TabSize,
    },
    resizable_panel, v_resizable,
};
use gpui_kit::{
    AnyWindowHandle, App, Bounds, ClickEvent, ClipboardItem, Entity, Focusable, FontWeight,
    KeyBinding, MouseButton, PathPromptOptions, Subscription, Task, Window, WindowBounds,
    WindowControlArea, WindowOptions, div, prelude::*, px, rgb, size,
};
use gpui_kit::{EventEmitter, uniform_list};
use leetdesk::online;
use leetdesk::{
    appearance::Appearance,
    catalog::{self, Case, PracticeLanguage, Problem},
    catalog_view::CatalogView,
    language::{self, GoLanguage},
    note_bundle,
    preferences::{EditorPreferences, EditorTheme, InterfaceLanguage},
    runner, snippets,
    store::{Progress, Store},
    sync,
};
use lsp_types::{CompletionContext, CompletionResponse, Hover, LocationLink, Position};
use serde_json::json;
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[path = "reading.rs"]
mod reading;
use reading::WorkspaceMode;

gpui_kit::actions!(
    leetdesk,
    [
        Save,
        RunCases,
        Submit,
        Format,
        ImportNotes,
        OrganizeImports,
        ToggleWrap,
        StudyMode,
        CodingMode,
        SearchProblems,
        Settings,
        Quit
    ]
);

const BG: u32 = 0x1b1c23;
const PANEL: u32 = 0x23242d;
const LINE: u32 = 0x81808b;
const MUTED: u32 = 0xc8c4cb;
const INK: u32 = 0xf4f5f7;
const ACCENT: u32 = 0xff9da8;
const CODE_BG: u32 = 0x1f2029;

#[cfg(target_os = "macos")]
const UI_FONT: &str = "PingFang SC";
#[cfg(not(target_os = "macos"))]
const UI_FONT: &str = "Microsoft YaHei";
#[cfg(target_os = "macos")]
const CODE_FONT: &str = "Menlo";
#[cfg(not(target_os = "macos"))]
const CODE_FONT: &str = "Consolas";

pub(crate) fn ui_font() -> &'static str {
    UI_FONT
}

type DocumentCache = (
    (u32, usize, u8, String),
    Entity<TextViewState>,
    Option<PathBuf>,
);

#[derive(Clone, Copy, PartialEq)]
enum DocumentTab {
    Description,
    Explanation,
    Notes,
}
#[derive(Clone, Copy, PartialEq)]
enum BottomTab {
    Cases,
    Results,
    Diagnostics,
}

#[derive(Clone, Copy, PartialEq)]
enum NotesView {
    Edit,
    Split,
    Preview,
}

struct GoProvider {
    service: GoLanguage,
}

struct JavaProvider;
fn localize_completions(
    mut items: Vec<lsp_types::CompletionItem>,
    language: leetdesk::i18n::UiLanguage,
) -> Vec<lsp_types::CompletionItem> {
    for item in &mut items {
        item.label = language.text(&item.label).into_owned();
        if let Some(detail) = &mut item.detail {
            *detail = language.text(detail).into_owned();
        }
    }
    items
}
impl CompletionProvider for JavaProvider {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        _: CompletionContext,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<Result<CompletionResponse>> {
        let source = text.to_string();
        Task::ready(Ok(CompletionResponse::Array(localize_completions(
            snippets::java_completions(&source, offset),
            ui::language(cx),
        ))))
    }
    fn is_completion_trigger(&self, _: usize, text: &str, _: &mut App) -> bool {
        text.chars()
            .last()
            .is_some_and(|ch| ch == '.' || ch == '_' || ch.is_ascii_alphanumeric())
    }
}

impl CompletionProvider for GoProvider {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        mut trigger: CompletionContext,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<Result<CompletionResponse>> {
        let source = text.to_string();
        let locale = ui::language(cx);
        let local = snippets::completions(&source, offset);
        if let Err(error) = self.service.sync(&source) {
            if local.is_empty() {
                return Task::ready(Err(error));
            }
            return Task::ready(Ok(CompletionResponse::Array(localize_completions(
                local, locale,
            ))));
        }
        let mut params = self.service.at(&source, offset);
        // Identifier typing should invoke completion; only '.' is a gopls
        // trigger character. The input component passes its whole prefix here.
        if trigger.trigger_kind == lsp_types::CompletionTriggerKind::TRIGGER_CHARACTER
            && trigger.trigger_character.as_deref() != Some(".")
        {
            trigger.trigger_kind = lsp_types::CompletionTriggerKind::INVOKED;
            trigger.trigger_character = None;
        }
        params["context"] = serde_json::to_value(trigger).unwrap();
        let service = self.service.clone();
        cx.background_executor().spawn(async move {
            let value = match service.request("textDocument/completion", params) {
                Ok(value) => value,
                Err(_error) if !local.is_empty() => {
                    return Ok(CompletionResponse::Array(localize_completions(
                        local, locale,
                    )));
                }
                Err(error) => return Err(error),
            };
            let mut items = vec![];
            if !value.is_null() {
                match serde_json::from_value::<CompletionResponse>(value)? {
                    CompletionResponse::Array(found) => items.extend(found),
                    CompletionResponse::List(found) => items.extend(found.items),
                }
            }
            Ok(CompletionResponse::Array(localize_completions(
                snippets::merge_go_completions(&source, local, items),
                locale,
            )))
        })
    }
    fn is_completion_trigger(&self, _: usize, text: &str, _: &mut App) -> bool {
        text.chars()
            .last()
            .is_some_and(|ch| ch == '.' || ch == '_' || ch.is_ascii_alphanumeric())
    }
}

impl HoverProvider for GoProvider {
    fn hover(
        &self,
        text: &Rope,
        offset: usize,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<Result<Option<Hover>>> {
        let source = text.to_string();
        if let Err(error) = self.service.sync(&source) {
            return Task::ready(Err(error));
        }
        let params = self.service.at(&source, offset);
        let service = self.service.clone();
        cx.background_executor().spawn(async move {
            Ok(serde_json::from_value(
                service.request("textDocument/hover", params)?,
            )?)
        })
    }
}

impl DefinitionProvider for GoProvider {
    fn definitions(
        &self,
        text: &Rope,
        offset: usize,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<Result<Vec<LocationLink>>> {
        let source = text.to_string();
        if let Err(error) = self.service.sync(&source) {
            return Task::ready(Err(error));
        }
        let params = self.service.at(&source, offset);
        let service = self.service.clone();
        cx.background_executor().spawn(async move {
            let response = service.request("textDocument/definition", params)?;
            if response.is_null() {
                return Ok(vec![]);
            }
            let values = if response.is_array() {
                response.as_array().unwrap().clone()
            } else {
                vec![response]
            };
            values
                .into_iter()
                .map(|value| {
                    if value.get("targetUri").is_some() {
                        return Ok(serde_json::from_value(value)?);
                    }
                    let location: lsp_types::Location = serde_json::from_value(value)?;
                    Ok(LocationLink {
                        origin_selection_range: None,
                        target_uri: location.uri,
                        target_range: location.range,
                        target_selection_range: location.range,
                    })
                })
                .collect()
        })
    }
}

#[derive(Clone)]
pub enum WorkspaceEvent {
    LoadDetail(Problem),
    Run {
        problem: Problem,
        language: PracticeLanguage,
        source: String,
        input: String,
        generation: u64,
        cancel: Arc<AtomicBool>,
        submit: bool,
    },
}
impl EventEmitter<WorkspaceEvent> for Workspace {}

pub struct Workspace {
    store: Store,
    problems: Vec<Problem>,
    catalog_view: CatalogView,
    current: usize,
    editor: Entity<EditorState>,
    search: Entity<InputState>,
    cases_editor: Entity<EditorState>,
    run_input_editor: Entity<EditorState>,
    has_run_input: bool,
    note_editor: Entity<EditorState>,
    note_preview: Entity<TextViewState>,
    document_view: Option<DocumentCache>,
    saved_note: String,
    notes_view: NotesView,
    note_dirty: bool,
    tag_editor: Entity<InputState>,
    reference_editor: Entity<EditorState>,
    reference_path: Option<String>,
    document_tab: DocumentTab,
    bottom_tab: BottomTab,
    selected_note: usize,
    explanations: Vec<(String, String)>,
    filter: String,
    study_plan_filter: String,
    status_filter: String,
    query: String,
    difficulty_filter: String,
    show_categories: bool,
    learning_navigation: bool,
    expanded_chapters: HashSet<String>,
    expanded_patterns: HashSet<String>,
    workspace_mode: WorkspaceMode,
    filters_open: bool,
    plan_menu_open: bool,
    learning_overview: bool,
    active_section: usize,
    pending_section: Option<String>,
    reader_wide: bool,
    focus_mode: bool,
    online_enabled: bool,
    soft_wrap: bool,
    editor_font_size: u8,
    editor_preferences: EditorPreferences,
    editor_fonts: Vec<String>,
    settings_preview: Entity<EditorState>,
    settings_open: bool,
    settings_focus: gpui_kit::FocusHandle,
    window_handle: AnyWindowHandle,
    progress: Progress,
    output: String,
    notice: String,
    language_status: String,
    language: Option<GoLanguage>,
    practice_language: PracticeLanguage,
    language_epoch: u64,
    diagnostic_generation: u64,
    diagnostics: Vec<lsp_types::Diagnostic>,
    cancellation: Option<Arc<AtomicBool>>,
    run_generation: u64,
    dirty: bool,
    last_source: String,
    saved_source_revision: Option<(u32, u64)>,
    edit_generation: u64,
    edit_task: Option<Task<()>>,
    hint_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
    _poll: Task<()>,
}

impl Workspace {
    pub fn new(store: Store, window: &mut Window, cx: &mut Context<Self>) -> Result<Self> {
        let preferences = store.editor_preferences();
        ui::configure(store.site(), preferences.interface_language, cx);
        theme::select_editor_theme(preferences.editor_theme, cx);
        apply_appearance(theme::current(cx), cx);
        let problems = store.load_catalog()?;
        if problems.is_empty() {
            return Err(anyhow!("题库为空"));
        }
        let last_id = fs::read_to_string(store.root.join("session.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<u32>(&text).ok());
        let current = problems
            .iter()
            .position(|p| Some(p.id) == last_id)
            .unwrap_or(0);
        let problem = &problems[current];
        let practice_language = store.language();
        let installed_fonts = cx.text_system().all_font_names();
        let editor_fonts = [
            CODE_FONT,
            "Cascadia Code",
            "Cascadia Mono",
            "Consolas",
            "Courier New",
            "JetBrains Mono",
            "Fira Code",
            "Menlo",
            "Monaco",
            "SF Mono",
            "DejaVu Sans Mono",
            "Liberation Mono",
        ]
        .into_iter()
        .filter(|name| installed_fonts.iter().any(|installed| installed == name))
        .map(str::to_owned)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
        let settings_preview = cx.new(|cx| {
            let mut state = EditorState::new(window, cx).language("go").soft_wrap(true).line_number(true);
            state.set_value("// LeetCode · Preview\nfunc twoSum(nums []int, target int) []int {\n    message := \"Hello, 世界\"\n    return []int{0, 1}\n}", window, cx);
            state
        });
        store.prepare(problem)?;
        let last_source = store
            .source_for(problem.id, practice_language)
            .unwrap_or_else(|_| problem.starter_for(practice_language));
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(practice_language.highlighter())
                .soft_wrap(preferences.soft_wrap)
                .smart_indent(true)
                .auto_close(true)
                .line_number(true)
                .folding(true)
                .tab_size(TabSize {
                    tab_size: 4,
                    hard_tabs: practice_language == PracticeLanguage::Go,
                })
                .default_value(last_source.clone())
        });
        let cases_editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .soft_wrap(true)
                .language(if problem.online.is_some() {
                    "plaintext"
                } else {
                    "json"
                })
                .line_number(true)
                .default_value(if problem.online.is_some() {
                    store.online_input(problem)
                } else {
                    store.cases(problem.id).unwrap_or_default()
                })
        });
        let run_input_editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .soft_wrap(true)
                .line_number(true)
        });
        if let Ok(text) = fs::read_to_string(store.workspace(problem.id).join("cursor.json"))
            && let Ok(position) = serde_json::from_str::<Position>(&text)
        {
            editor.update(cx, |state, cx| {
                state.set_cursor_position(position, window, cx)
            });
        }
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(ui::text("搜索题号、标题、标签", cx))
        });
        let saved_note =
            fs::read_to_string(store.workspace(problem.id).join("notes.md")).unwrap_or_default();
        let note_preview = cx.new(|cx| TextViewState::markdown(&saved_note, cx));
        let note_editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("markdown")
                .soft_wrap(true)
                .line_number(true)
                .tab_size(TabSize {
                    tab_size: 4,
                    hard_tabs: false,
                })
                .placeholder(ui::text(
                    "# 我的思路\n\n记录做法、复杂度与易错点。支持 Markdown，保存后自动渲染。",
                    cx,
                ))
                .default_value(saved_note.clone())
        });
        let progress = store.progress(problem.id);
        let tag_editor = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(ui::text("自定义标签，用逗号分隔", cx))
                .default_value(progress.tags.join(", "))
        });
        let reference_editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("go")
                .soft_wrap(true)
                .line_number(true)
                .tab_size(TabSize {
                    tab_size: 4,
                    hard_tabs: true,
                })
        });
        let explanations = store.notes(problem)?;
        let output = store.latest_run(problem.id);
        let subscriptions = vec![
            cx.subscribe(&search, |this, state, event, cx| {
                if matches!(event, InputEvent::Change) {
                    this.query = state.read(cx).value().to_string();
                    cx.notify();
                }
            }),
            cx.subscribe(&editor, |this, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    this.changed(cx);
                }
            }),
            cx.subscribe(&note_editor, |this, state, event, cx| {
                if matches!(event, InputEvent::Change) {
                    this.note_dirty = state.read(cx).value().as_ref() != this.saved_note;
                    this.changed(cx);
                }
            }),
            cx.subscribe(&cases_editor, |this, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    this.changed(cx);
                }
            }),
            cx.subscribe(&tag_editor, |this, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    this.changed(cx);
                }
            }),
        ];
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                if this
                    .update(cx, |this, cx| this.poll_diagnostics(cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        let mut catalog_view = CatalogView::new(&problems, store.all_progress());
        let site = problem
            .online
            .as_ref()
            .map(|remote| remote.site)
            .unwrap_or_default();
        catalog_view.set_plan(store.study_plan(site));
        let membership = leetdesk::learning::memberships(&problem.slug);
        let expanded_chapters = membership
            .first()
            .and_then(|id| leetdesk::learning::find(&format!("@pattern:{id}")))
            .map(|(chapter, _)| HashSet::from([chapter.id.clone()]))
            .unwrap_or_default();
        let expanded_patterns = membership.first().cloned().into_iter().collect();
        let workspace_mode = reading::load_mode(&store.root);
        Ok(Self {
            store,
            problems,
            catalog_view,
            current,
            editor,
            search,
            cases_editor,
            run_input_editor,
            has_run_input: false,
            note_editor,
            note_preview,
            document_view: None,
            saved_note,
            notes_view: NotesView::Split,
            note_dirty: false,
            tag_editor,
            reference_editor,
            reference_path: None,
            document_tab: DocumentTab::Explanation,
            bottom_tab: BottomTab::Cases,
            selected_note: 0,
            explanations,
            filter: "全部题目".into(),
            study_plan_filter: "全部题目".into(),
            status_filter: "全部".into(),
            query: String::new(),
            difficulty_filter: "全部".into(),
            show_categories: true,
            learning_navigation: true,
            expanded_chapters,
            expanded_patterns,
            workspace_mode,
            filters_open: false,
            plan_menu_open: false,
            learning_overview: false,
            active_section: 0,
            pending_section: None,
            reader_wide: true,
            focus_mode: false,
            online_enabled: false,
            soft_wrap: preferences.soft_wrap,
            editor_font_size: preferences.font_size,
            editor_preferences: preferences,
            editor_fonts,
            settings_preview,
            settings_open: false,
            settings_focus: cx.focus_handle(),
            window_handle: window.window_handle(),
            progress,
            output,
            notice: "Go 工作区已就绪 · 可导入你的 Markdown 讲解".into(),
            language_status: "正在连接 gopls…".into(),
            language: None,
            practice_language,
            language_epoch: 0,
            diagnostic_generation: 0,
            diagnostics: vec![],
            cancellation: None,
            run_generation: 0,
            dirty: false,
            last_source,
            saved_source_revision: None,
            edit_generation: 0,
            edit_task: None,
            hint_task: None,
            _subscriptions: subscriptions,
            _poll: poll,
        })
    }

    pub fn problem(&self) -> &Problem {
        &self.problems[self.current]
    }

    fn set_appearance(&mut self, appearance: Appearance, cx: &mut Context<Self>) {
        if let Err(error) = self.store.save_appearance(appearance) {
            self.notice = format!("主题保存失败：{error}");
        } else {
            self.notice = format!("已切换为{}主题", appearance.label());
        }
        apply_appearance(appearance, cx);
        self.document_view = None;
        cx.notify();
    }

    fn settings(&mut self, _: &Settings, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = !self.settings_open;
        if !self.settings_open {
            self.editor.update(cx, |state, cx| state.focus(window, cx));
        } else {
            self.settings_focus.focus(window, cx);
        }
        cx.notify();
    }

    fn save_editor_preferences(&mut self, cx: &mut Context<Self>) {
        self.editor_preferences.font_size = self.editor_font_size;
        self.editor_preferences.soft_wrap = self.soft_wrap;
        if let Err(error) = self.store.save_editor_preferences(EditorPreferences {
            font_size: self.editor_font_size,
            soft_wrap: self.soft_wrap,
            ..self.editor_preferences.clone()
        }) {
            self.notice = format!("设置保存失败：{error}");
        }
        cx.notify();
    }

    fn code_font(&self) -> &str {
        if self
            .editor_fonts
            .contains(&self.editor_preferences.font_family)
        {
            &self.editor_preferences.font_family
        } else {
            CODE_FONT
        }
    }

    fn set_interface_language(
        &mut self,
        preference: InterfaceLanguage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editor_preferences.interface_language = preference;
        ui::configure(self.store.site(), preference, cx);
        self.search.update(cx, |state, cx| {
            state.set_placeholder(ui::text("搜索题号、标题、标签", cx), window, cx)
        });
        self.tag_editor.update(cx, |state, cx| {
            state.set_placeholder(ui::text("自定义标签，用逗号分隔", cx), window, cx)
        });
        self.note_editor.update(cx, |state, cx| {
            state.set_placeholder(
                ui::text(
                    "# 我的思路\n\n记录做法、复杂度与易错点。支持 Markdown，保存后自动渲染。",
                    cx,
                ),
                window,
                cx,
            )
        });
        self.document_view = None;
        self.save_editor_preferences(cx);
    }

    fn language_settings(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        div()
            .mt_4()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(ui::text("界面语言", cx)),
            )
            .child(div().mt_2().flex().gap_1().flex_wrap().children(
                InterfaceLanguage::ALL.into_iter().map(|preference| {
                    Button::new(gpui_kit::SharedString::from(format!(
                        "interface-language-{}",
                        preference.key()
                    )))
                    .label(format!(
                        "{}{}",
                        if ui::preference(cx) == preference {
                            "✓ "
                        } else {
                            ""
                        },
                        ui::text(preference.label(), cx)
                    ))
                    .small()
                    .ghost()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_interface_language(preference, window, cx)
                    }))
                }),
            ))
            .child(
                div()
                    .mt_1()
                    .text_xs()
                    .text_color(theme::color(MUTED, cx))
                    .child(ui::text("跟随站点时：国际站英文，中国站中文", cx)),
            )
            .into_any_element()
    }

    fn font_settings(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        div()
            .mt_3()
            .child(div().text_sm().child(ui::text("代码字体", cx)))
            .child(
                div()
                    .mt_2()
                    .flex()
                    .gap_1()
                    .flex_wrap()
                    .child(
                        Button::new("editor-font-default")
                            .label(ui::text(
                                if self.editor_preferences.font_family.is_empty() {
                                    "✓ 默认等宽字体"
                                } else {
                                    "默认等宽字体"
                                },
                                cx,
                            ))
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.editor_preferences.font_family.clear();
                                this.save_editor_preferences(cx);
                            })),
                    )
                    .children(self.editor_fonts.iter().map(|font| {
                        let font = font.clone();
                        Button::new(gpui_kit::SharedString::from(format!("editor-font-{font}")))
                            .label(format!(
                                "{}{font}",
                                if self.editor_preferences.font_family == font {
                                    "✓ "
                                } else {
                                    ""
                                }
                            ))
                            .small()
                            .ghost()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.editor_preferences.font_family = font.clone();
                                this.save_editor_preferences(cx);
                            }))
                    })),
            )
            .when(
                !self.editor_preferences.font_family.is_empty()
                    && !self
                        .editor_fonts
                        .contains(&self.editor_preferences.font_family),
                |row| {
                    row.child(
                        div()
                            .mt_1()
                            .text_xs()
                            .text_color(theme::color(MUTED, cx))
                            .child(ui::text("字体不可用，已使用默认等宽字体", cx)),
                    )
                },
            )
            .into_any_element()
    }

    fn editor_theme_settings(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        div()
            .mt_4()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(ui::text("编辑器颜色主题", cx)),
            )
            .child(div().mt_2().flex().gap_1().flex_wrap().children(
                EditorTheme::ALL.into_iter().map(|value| {
                    Button::new(gpui_kit::SharedString::from(format!(
                        "editor-theme-{}",
                        value.key()
                    )))
                    .label(format!(
                        "{}{}",
                        if self.editor_preferences.editor_theme == value {
                            "✓ "
                        } else {
                            ""
                        },
                        ui::text(value.label(), cx)
                    ))
                    .small()
                    .ghost()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.editor_preferences.editor_theme = value;
                        theme::select_editor_theme(value, cx);
                        apply_appearance(theme::current(cx), cx);
                        this.document_view = None;
                        this.save_editor_preferences(cx);
                    }))
                }),
            ))
            .child(
                div()
                    .mt_2()
                    .text_xs()
                    .text_color(theme::color(MUTED, cx))
                    .child(ui::text(
                        "预览 · 配色跟随界面明暗，关键字加粗、注释斜体",
                        cx,
                    )),
            )
            .child(
                div()
                    .mt_2()
                    .h(px(145.))
                    .border_1()
                    .border_color(theme::color(LINE, cx))
                    .child(
                        Editor::new(&self.settings_preview)
                            .readonly(true)
                            .size_full()
                            .font_family(self.code_font().to_owned())
                            .text_size(px(self.editor_font_size as f32)),
                    ),
            )
            .into_any_element()
    }

    pub fn connect_language(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.language_epoch += 1;
        let epoch = self.language_epoch;
        self.language = None;
        self.hint_task = None;
        self.diagnostic_generation = 0;
        self.diagnostics.clear();
        self.language_status = "正在连接 gopls…".into();
        self.editor.update(cx, |state, cx| {
            state.set_inlay_hints(Vec::new(), cx);
            *state.lsp_mut() = Default::default();
            if let Some(diagnostics) = state.diagnostics_mut() {
                diagnostics.clear();
            }
            cx.notify();
        });
        if self.practice_language == PracticeLanguage::Java {
            self.language_status = "Java 语法高亮与模板补全".into();
            self.editor.update(cx, |state, cx| {
                state.lsp_mut().completion_provider = Some(Rc::new(JavaProvider));
                state.refresh(cx);
            });
            cx.notify();
            return;
        }
        let path = self.store.workspace(self.problem().id);
        let source = self.editor.read(cx).value().to_string();
        let task = cx
            .background_executor()
            .spawn(async move { GoLanguage::start(&path, &source) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            _ = this.update_in(cx, |this, _window, cx| {
                if this.language_epoch != epoch {
                    return;
                }
                match result {
                    Ok(service) => {
                        let _ = service.sync(&this.editor.read(cx).value());
                        let provider = Rc::new(GoProvider {
                            service: service.clone(),
                        });
                        let owner = cx.entity().downgrade();
                        let current_uri = service.uri().to_string();
                        this.editor.update(cx, |state, cx| {
                            state.lsp_mut().completion_provider = Some(provider.clone());
                            state.lsp_mut().hover_provider = Some(provider.clone());
                            state.lsp_mut().definition_provider = Some(provider);
                            state.lsp_mut().show_document =
                                Some(Rc::new(move |params, window, cx| {
                                    if params.uri.as_str() == current_uri {
                                        return false;
                                    }
                                    if let Ok(url) = url::Url::parse(params.uri.as_str())
                                        && let Ok(path) = url.to_file_path()
                                        && let Ok(text) = fs::read_to_string(&path)
                                    {
                                        let _ = owner.update(cx, |this, cx| {
                                            this.reference_path = Some(path.display().to_string());
                                            this.reference_editor.update(cx, |state, cx| {
                                                state.set_value(text, window, cx);
                                                if let Some(range) = params.selection {
                                                    state.set_cursor_position(
                                                        range.start,
                                                        window,
                                                        cx,
                                                    );
                                                }
                                            });
                                            cx.notify();
                                        });
                                        return true;
                                    }
                                    true
                                }));
                            state.refresh(cx);
                        });
                        this.language_status = "gopls 已连接".into();
                        this.language = Some(service);
                        this.schedule_type_hints(cx);
                    }
                    Err(error) => {
                        this.language_status = "gopls 连接失败".into();
                        this.notice = format!("{error:#}");
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        self.dirty = true;
        self.edit_generation += 1;
        let source = self.editor.read(cx).value().to_string();
        if source != self.last_source {
            self.last_source = source.clone();
            if let Some(service) = &self.language
                && let Err(error) = service.sync(&source)
            {
                self.notice = error.to_string();
            }
            // Only source edits invalidate diagnostics. Editing notes does not.
            self.diagnostics.clear();
            self.diagnostic_generation = 0;
            self.editor.update(cx, |editor, cx| {
                editor.set_inlay_hints(Vec::new(), cx);
                if let Some(set) = editor.diagnostics_mut() {
                    set.clear();
                }
                cx.notify();
            });
            self.schedule_type_hints(cx);
        }
        let handle = self.window_handle;
        self.edit_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(450))
                .await;
            _ = handle.update(cx, |_, _window, cx| {
                _ = this.update(cx, |this, cx| {
                    if let Err(error) = this.save(cx) {
                        this.notice = error.to_string();
                    }
                    cx.notify();
                });
            });
        }));
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) -> Result<()> {
        let id = self.problem().id;
        let source_revision = (id, self.editor.read(cx).document_revision());
        if self.saved_source_revision != Some(source_revision) {
            self.store.save_source_for(
                id,
                self.practice_language,
                &self.editor.read(cx).value(),
            )?;
            self.saved_source_revision = Some(source_revision);
            if let Some(service) = &self.language {
                let _ = service.saved();
            }
        }
        if self.problem().online.is_some() {
            self.store
                .save_online_input(id, &self.cases_editor.read(cx).value())?;
        } else {
            self.store
                .save_cases(id, &self.cases_editor.read(cx).value())?;
        }
        self.save_notes_document(cx)?;
        leetdesk::store::atomic_write(
            &self.store.workspace(id).join("cursor.json"),
            &serde_json::to_vec(&self.editor.read(cx).cursor_position())?,
        )?;
        self.progress.tags = self
            .tag_editor
            .read(cx)
            .value()
            .split([',', '，'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        self.progress.tags.sort();
        self.progress.tags.dedup();
        self.store.save_progress(id, &self.progress)?;
        self.catalog_view.update_progress(id, &self.progress);
        leetdesk::store::atomic_write(
            &self.store.root.join("session.json"),
            serde_json::to_vec(&id)?.as_slice(),
        )?;
        self.dirty = false;
        Ok(())
    }

    pub fn flush(&mut self, cx: &mut Context<Self>) -> Result<()> {
        self.save(cx)
    }

    fn save_notes_document(&mut self, cx: &mut Context<Self>) -> Result<()> {
        let markdown = self.note_editor.read(cx).value().to_string();
        leetdesk::store::atomic_write(
            &self.store.workspace(self.problem().id).join("notes.md"),
            markdown.as_bytes(),
        )?;
        if markdown != self.saved_note {
            self.note_preview
                .update(cx, |state, cx| state.set_text(&markdown, cx));
            self.saved_note = markdown;
        }
        self.note_dirty = false;
        Ok(())
    }

    fn save_notes_click(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.notice = match self.save_notes_document(cx) {
            Ok(()) => "笔记已保存为 notes.md · Markdown 预览已更新".into(),
            Err(error) => format!("笔记保存失败：{error:#}"),
        };
        cx.notify();
    }

    fn export_note_bundle(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(error) = self.save_notes_document(cx) {
            self.notice = format!("笔记保存失败：{error:#}");
            cx.notify();
            return;
        }
        let directory = default_note_export_directory(&self.store.root);
        let prompt = cx.prompt_for_new_path(&directory, Some("leetdesk-notes.zip"));
        let root = self.store.root.clone();
        cx.spawn_in(window, async move |this, cx| {
            let selected = prompt.await;
            let result = match selected {
                Ok(Ok(Some(path))) => {
                    let path = path.with_extension("zip");
                    let task = cx.background_executor().spawn(async move {
                        let store = Store::open(root)?;
                        let summary = note_bundle::export(&store, &path)?;
                        Ok::<_, anyhow::Error>((path, summary))
                    });
                    Some(task.await)
                }
                Ok(Ok(None)) => None,
                Ok(Err(error)) => Some(Err(error)),
                Err(error) => Some(Err(error.into())),
            };
            if let Some(result) = result {
                _ = this.update_in(cx, |this, _, cx| {
                    this.notice = match result {
                        Ok((path, summary)) => format!(
                            "已导出 {} 篇个人笔记、{} 篇讲解和 {} 张图片到 {}{}",
                            summary.personal,
                            summary.explanations,
                            summary.images,
                            path.display(),
                            if summary.missing_images > 0 {
                                format!("；{} 张引用图片未找到", summary.missing_images)
                            } else {
                                String::new()
                            }
                        ),
                        Err(error) => format!("导出笔记失败：{error:#}"),
                    };
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn import_note_bundle(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(error) = self.save_notes_document(cx) {
            self.notice = format!("笔记保存失败：{error:#}");
            cx.notify();
            return;
        }
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(ui::text("选择 LeetDesk 笔记包（.zip）", cx).into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let selected = prompt.await;
            _ = this.update_in(cx, |this, window, cx| {
                let result = match selected {
                    Ok(Ok(Some(paths))) => paths
                        .first()
                        .map(|path| note_bundle::import(&this.store, path)),
                    Ok(Ok(None)) => None,
                    Ok(Err(error)) => Some(Err(error)),
                    Err(error) => Some(Err(error.into())),
                };
                if let Some(result) = result {
                    this.notice = match result {
                        Ok(summary) => {
                            this.saved_note = fs::read_to_string(
                                this.store.workspace(this.problem().id).join("notes.md"),
                            )
                            .unwrap_or_default();
                            this.note_editor.update(cx, |state, cx| {
                                state.set_value(this.saved_note.clone(), window, cx)
                            });
                            this.note_preview
                                .update(cx, |state, cx| state.set_text(&this.saved_note, cx));
                            this.note_dirty = false;
                            this.explanations =
                                this.store.notes(this.problem()).unwrap_or_default();
                            this.document_view = None;
                            format!(
                                "已导入 {} 篇个人笔记、{} 篇讲解；{} 篇与现有笔记合并",
                                summary.personal, summary.explanations, summary.merged
                            )
                        }
                        Err(error) => format!("导入笔记包失败：{error:#}"),
                    };
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn upload_backup(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(error) = self.save(cx) {
            self.notice = format!("保存后才能上传备份：{error:#}");
            cx.notify();
            return;
        }
        self.notice = "正在上传笔记、解法和提交快照…".into();
        let root = self.store.root.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = smol::unblock(move || {
                let store = Store::open(root)?;
                sync::upload(&store)
            })
            .await;
            _ = this.update_in(cx, |this, _, cx| {
                this.notice = match result {
                    Ok(summary) => format!(
                        "备份已上传：{} 篇笔记、{} 份解法、{} 次提交快照",
                        summary.notes, summary.solutions, summary.submissions
                    ),
                    Err(error) => format!("备份上传失败：{error:#}"),
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn download_backup(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(error) = self.save(cx) {
            self.notice = format!("保存后才能取回备份：{error:#}");
            cx.notify();
            return;
        }
        self.notice = "正在取回服务器上的最新备份…".into();
        let root = self.store.root.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = smol::unblock(move || {
                let store = Store::open(root)?;
                sync::download(&store)
            })
            .await;
            _ = this.update_in(cx, |this, window, cx| {
                this.notice = match result {
                    Ok(summary) => {
                        this.saved_note = fs::read_to_string(
                            this.store.workspace(this.problem().id).join("notes.md"),
                        )
                        .unwrap_or_default();
                        this.note_editor.update(cx, |state, cx| {
                            state.set_value(this.saved_note.clone(), window, cx)
                        });
                        this.note_preview
                            .update(cx, |state, cx| state.set_text(&this.saved_note, cx));
                        this.note_dirty = false;
                        this.explanations = this.store.notes(this.problem()).unwrap_or_default();
                        this.document_view = None;
                        format!(
                            "已取回：{} 篇笔记、{} 份解法、{} 次提交快照；{} 处冲突保留了副本",
                            summary.notes,
                            summary.solutions,
                            summary.submissions,
                            summary.conflicts
                        )
                    }
                    Err(error) => format!("取回备份失败：{error:#}"),
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub fn set_online_enabled(&mut self, enabled: bool) {
        self.online_enabled = enabled;
    }
    pub fn update_study_plan(&mut self, plan: online::StudyPlan, cx: &mut Context<Self>) {
        self.catalog_view.set_plan(plan);
        self.notice = "热题 100 题单已更新并缓存".into();
        cx.notify();
    }
    pub fn request_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .problem()
            .online
            .as_ref()
            .is_some_and(|remote| !remote.loaded)
        {
            self.language_epoch += 1;
            self.language = None;
            self.language_status = "正在加载题目…".into();
            self.editor.update(cx, |state, cx| {
                *state.lsp_mut() = Default::default();
                if let Some(set) = state.diagnostics_mut() {
                    set.clear();
                }
                cx.notify();
            });
            cx.emit(WorkspaceEvent::LoadDetail(self.problem().clone()));
        } else {
            self.connect_language(window, cx);
        }
    }
    pub fn install_detail(
        &mut self,
        detail: Problem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let id = detail.id;
        self.store.save_online_detail(&detail)?;
        if let Some(index) = self.problems.iter().position(|p| p.id == id) {
            self.problems[index] = detail.clone();
            self.document_view = None;
            self.catalog_view.replace(&self.problems);
        }
        if self.problem().id != id {
            cx.notify();
            return Ok(());
        }
        self.store.prepare(&detail)?;
        let source = self
            .store
            .source_for(id, self.practice_language)
            .unwrap_or_else(|_| detail.starter_for(self.practice_language));
        self.last_source = source.clone();
        self.editor
            .update(cx, |state, cx| state.set_value(source, window, cx));
        self.cases_editor.update(cx, |state, cx| {
            state.set_highlighter("plaintext", cx);
            state.set_value(self.store.online_input(&detail), window, cx);
        });
        self.explanations = self.store.notes(&detail)?;
        self.selected_note = 0;
        self.notice = format!(
            "已加载 {} · {}",
            detail
                .online
                .as_ref()
                .map(|m| m.frontend_id.as_str())
                .unwrap_or(""),
            detail.title
        );
        self.connect_language(window, cx);
        cx.notify();
        Ok(())
    }
    pub fn merge_online_index(
        &mut self,
        mut problems: Vec<Problem>,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.save(cx)?;
        let id = self.problem().id;
        let loaded: HashMap<_, _> = self
            .problems
            .iter()
            .filter(|p| p.online.as_ref().is_some_and(|m| m.loaded))
            .map(|p| (p.slug.as_str(), p))
            .collect();
        for problem in &mut problems {
            if let Some(existing) = loaded.get(problem.slug.as_str())
                && existing.online.as_ref().map(|remote| remote.site)
                    == problem.online.as_ref().map(|remote| remote.site)
            {
                let status = problem.online.as_ref().and_then(|m| m.status.clone());
                *problem = (*existing).clone();
                problem.online.as_mut().unwrap().status = status;
            }
        }
        self.current = problems.iter().position(|p| p.id == id).unwrap_or(0);
        self.problems = problems;
        self.document_view = None;
        self.catalog_view.replace(&self.problems);
        let site = self
            .problem()
            .online
            .as_ref()
            .map(|remote| remote.site)
            .unwrap_or_default();
        self.catalog_view.set_plan(self.store.study_plan(site));
        cx.notify();
        Ok(())
    }
    pub fn online_result(
        &mut self,
        id: u32,
        generation: u64,
        source: &str,
        output: String,
        cx: &mut Context<Self>,
    ) {
        if generation != self.run_generation {
            return;
        }
        self.cancellation = None;
        if let Err(error) = self.store.save_run(id, source, 0, 0, &output) {
            self.notice = format!("运行完成，保存记录失败：{error}");
        } else {
            self.notice = format!("题目 {id} 的在线运行已完成");
        }
        if self.problem().id == id {
            self.output = output;
        }
        cx.notify();
    }
    pub fn online_error(&mut self, message: String, cx: &mut Context<Self>) {
        self.notice = message;
        cx.notify();
    }

    fn poll_diagnostics(&mut self, cx: &mut Context<Self>) {
        let Some(service) = &self.language else {
            return;
        };
        if let Some(error) = service.error() {
            if self.language_status != "gopls 已断开" {
                self.language_status = "gopls 已断开".into();
                self.notice = error;
                cx.notify();
            }
            return;
        }
        let Some(snapshot) = service.diagnostics_since(self.diagnostic_generation) else {
            return;
        };
        if snapshot.version.is_some_and(|v| v != service.version()) {
            return;
        }
        self.diagnostic_generation = snapshot.generation;
        self.diagnostics = snapshot.items.clone();
        self.editor.update(cx, |state, cx| {
            let text = state.text().clone();
            if let Some(set) = state.diagnostics_mut() {
                set.reset(&text);
                set.extend(snapshot.items);
            }
            cx.notify();
        });
        cx.notify();
    }

    fn select_problem(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.learning_overview = false;
        self.active_section = 0;
        self.pending_section = None;
        self.document_view = None;
        if index == self.current {
            cx.notify();
            if self.problem().online.as_ref().is_some_and(|m| !m.loaded) {
                cx.emit(WorkspaceEvent::LoadDetail(self.problem().clone()));
            }
            return;
        }
        if let Err(error) = self.save(cx) {
            self.notice = format!("切换前保存失败：{error}");
            cx.notify();
            return;
        }
        self.edit_task = None;
        let problem = self.problems[index].clone();
        if let Err(error) = self.store.prepare(&problem) {
            self.notice = error.to_string();
            cx.notify();
            return;
        }
        self.current = index;
        self.progress = self.store.progress(problem.id);
        self.selected_note = 0;
        self.reference_path = None;
        self.explanations = self.store.notes(&problem).unwrap_or_default();
        self.output = self.store.latest_run(problem.id);
        self.has_run_input = false;
        let source = self
            .store
            .source_for(problem.id, self.practice_language)
            .unwrap_or_else(|_| problem.starter_for(self.practice_language));
        self.last_source = source.clone();
        self.saved_source_revision = None;
        self.editor.update(cx, |state, cx| {
            state.set_highlighter(self.practice_language.highlighter(), cx);
            state.set_value(source, window, cx);
            if let Ok(text) =
                fs::read_to_string(self.store.workspace(problem.id).join("cursor.json"))
                && let Ok(position) = serde_json::from_str::<Position>(&text)
            {
                state.set_cursor_position(position, window, cx);
            }
        });
        self.cases_editor.update(cx, |state, cx| {
            state.set_highlighter(
                if problem.online.is_some() {
                    "plaintext"
                } else {
                    "json"
                },
                cx,
            );
            state.set_value(
                if problem.online.is_some() {
                    self.store.online_input(&problem)
                } else {
                    self.store.cases(problem.id).unwrap_or_default()
                },
                window,
                cx,
            )
        });
        self.saved_note = fs::read_to_string(self.store.workspace(problem.id).join("notes.md"))
            .unwrap_or_default();
        self.note_editor.update(cx, |state, cx| {
            state.set_value(self.saved_note.clone(), window, cx)
        });
        self.note_preview
            .update(cx, |state, cx| state.set_text(&self.saved_note, cx));
        self.note_dirty = false;
        self.tag_editor.update(cx, |state, cx| {
            state.set_value(self.progress.tags.join(", "), window, cx)
        });
        self.notice = format!("已打开 {} · {}", problem.id, problem.title);
        self.request_current(window, cx);
        cx.notify();
    }

    fn switch_language(
        &mut self,
        language: PracticeLanguage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if language == self.practice_language {
            return;
        }
        if let Err(error) = self.save(cx) {
            self.notice = format!("切换语言前保存失败：{error:#}");
            cx.notify();
            return;
        }
        if let Err(error) = self.store.save_language(language) {
            self.notice = format!("保存语言偏好失败：{error:#}");
            cx.notify();
            return;
        }
        self.practice_language = language;
        self.saved_source_revision = None;
        let problem = self.problem().clone();
        if let Err(error) = self.store.prepare(&problem) {
            self.notice = error.to_string();
            cx.notify();
            return;
        }
        let source = self
            .store
            .source_for(problem.id, language)
            .unwrap_or_else(|_| problem.starter_for(language));
        self.last_source = source.clone();
        self.editor.update(cx, |state, cx| {
            state.set_highlighter(language.highlighter(), cx);
            state.set_tab_size(
                TabSize {
                    tab_size: 4,
                    hard_tabs: language == PracticeLanguage::Go,
                },
                cx,
            );
            state.set_value(source, window, cx);
        });
        self.connect_language(window, cx);
        self.notice = format!("已切换到 {}；代码分别保存在独立文件中", language.label());
        if language == PracticeLanguage::Java
            && problem
                .online
                .as_ref()
                .is_some_and(|m| m.loaded && problem.java_starter.is_empty())
        {
            cx.emit(WorkspaceEvent::LoadDetail(problem));
        }
        cx.notify();
    }

    fn capture_run_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.cases_editor.read(cx).value().to_string();
        let language = if self.problem().online.is_some() {
            "plaintext"
        } else {
            "json"
        };
        self.run_input_editor.update(cx, |state, cx| {
            state.set_highlighter(language, cx);
            state.set_value(input, window, cx);
        });
        self.has_run_input = true;
    }

    fn run_cases(&mut self, _: &RunCases, window: &mut Window, cx: &mut Context<Self>) {
        if self.cancellation.is_some() {
            return;
        }
        if let Err(error) = self.save(cx) {
            self.notice = error.to_string();
            cx.notify();
            return;
        }
        if self.problem().online.is_some() {
            if !self.online_enabled {
                self.notice = "当前是离线缓存，请连接账号后在线运行".into();
                cx.notify();
                return;
            }
            let source = self.editor.read(cx).value().to_string();
            let input = self.cases_editor.read(cx).value().to_string();
            if let Err(error) =
                online::run_request(self.problem(), self.practice_language, &source, &input)
            {
                self.notice = error.to_string();
                cx.notify();
                return;
            }
            self.run_generation += 1;
            let cancel = Arc::new(AtomicBool::new(false));
            self.cancellation = Some(cancel.clone());
            self.capture_run_input(window, cx);
            self.output = "正在使用当前账号在 LeetCode 运行…".into();
            self.bottom_tab = BottomTab::Results;
            cx.emit(WorkspaceEvent::Run {
                problem: self.problem().clone(),
                language: self.practice_language,
                source,
                input,
                generation: self.run_generation,
                cancel,
                submit: false,
            });
            cx.notify();
            return;
        }
        let problem = self.problem().clone();
        let source = self.editor.read(cx).value().to_string();
        let cases: Vec<Case> = match serde_json::from_str(&self.cases_editor.read(cx).value()) {
            Ok(cases) => cases,
            Err(error) => {
                self.notice = format!("测试用例 JSON 无效：{error}");
                self.bottom_tab = BottomTab::Cases;
                cx.notify();
                return;
            }
        };
        if let Err(error) = catalog::validate_cases(&cases, &problem.parameters) {
            self.notice = error.to_string();
            cx.notify();
            return;
        }
        self.run_generation += 1;
        let generation = self.run_generation;
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancellation = Some(cancel.clone());
        self.capture_run_input(window, cx);
        let path = self.store.workspace(problem.id);
        let practice_language = self.practice_language;
        self.output = format!("正在编译并运行 {} 用例…", practice_language.label());
        self.bottom_tab = BottomTab::Results;
        let task = cx.background_executor().spawn(async move {
            let result = match practice_language {
                PracticeLanguage::Go => runner::run(&problem, &source, &cases, &path, cancel),
                PracticeLanguage::Java => {
                    runner::run_java(&problem, &source, &cases, &path, cancel)
                }
            };
            (problem.id, source, result)
        });
        cx.spawn(async move |this, cx| {
            let (id, source, result) = task.await;
            _ = this.update(cx, |this, cx| {
                if this.run_generation != generation {
                    return;
                }
                this.cancellation = None;
                match result {
                    Ok(result) => {
                        let _ = this.store.save_run(
                            id,
                            &source,
                            result.passed,
                            result.total,
                            &result.output,
                        );
                        let mut progress = this.store.progress(id);
                        progress.state = if result.passed == result.total {
                            "本地通过"
                        } else {
                            "尝试中"
                        }
                        .into();
                        let _ = this.store.save_progress(id, &progress);
                        this.catalog_view.update_progress(id, &progress);
                        if this.problem().id == id {
                            this.output = result.output;
                            this.progress.state = progress.state;
                        }
                        this.notice =
                            format!("题目 {id}：本地 {}/{} 通过", result.passed, result.total);
                    }
                    Err(error) => {
                        this.notice = error.to_string();
                        if this.problem().id == id {
                            this.output = format!("{error:#}");
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn submit(&mut self, _: &Submit, _: &mut Window, cx: &mut Context<Self>) {
        if self.cancellation.is_some() {
            return;
        }
        let source = self.editor.read(cx).value().to_string();
        if !self.online_enabled || self.problem().online.is_none() {
            self.notice = "提交需要登录 LeetCode 并打开在线题目".into();
            cx.notify();
            return;
        }
        if let Err(error) = online::submit_request(self.problem(), self.practice_language, &source)
            .and_then(|_| self.save(cx))
            .and_then(|_| {
                self.store
                    .save_submission_snapshot(self.problem().id, self.practice_language, &source)
                    .map(|_| ())
            })
        {
            self.notice = error.to_string();
            cx.notify();
            return;
        }
        self.run_generation += 1;
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancellation = Some(cancel.clone());
        self.output = "正在提交当前代码快照，等待 LeetCode 判题…".into();
        self.has_run_input = false;
        self.bottom_tab = BottomTab::Results;
        cx.emit(WorkspaceEvent::Run {
            problem: self.problem().clone(),
            language: self.practice_language,
            source,
            input: String::new(),
            generation: self.run_generation,
            cancel,
            submit: true,
        });
        cx.notify();
    }

    pub fn record_submission(
        &mut self,
        id: u32,
        owner: &str,
        accepted: bool,
        cx: &mut Context<Self>,
    ) -> Result<usize> {
        let problem = self
            .problems
            .iter()
            .find(|p| p.id == id)
            .ok_or_else(|| anyhow!("提交题目已不在题库"))?;
        let site = problem
            .online
            .as_ref()
            .ok_or_else(|| anyhow!("在线题目信息缺失"))?
            .site;
        let mut cache = self
            .store
            .online_cache(site)?
            .ok_or_else(|| anyhow!("题库缓存缺失"))?;
        if cache.username != owner {
            return Err(anyhow!("账号已变化，提交状态未写入其他账号缓存"));
        }
        let remote = self
            .problems
            .iter_mut()
            .find(|p| p.id == id)
            .unwrap()
            .online
            .as_mut()
            .unwrap();
        if accepted {
            remote.status = Some("ac".into());
        } else if remote.status.as_deref() != Some("ac") {
            remote.status = Some("notac".into());
        }
        if let Some(cached) = cache.problems.iter_mut().find(|p| p.id == id) {
            cached.online.as_mut().unwrap().status = remote.status.clone();
        }
        self.store.save_online_cache(&mut cache)?;
        self.catalog_view.replace(&self.problems);
        cx.notify();
        Ok(self
            .problems
            .iter()
            .filter(|p| online::status_label(p) == "已通过")
            .count())
    }

    fn format(&mut self, _: &Format, window: &mut Window, cx: &mut Context<Self>) {
        self.format_on_save(window, cx);
    }

    fn initialize_code(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .problem()
            .online
            .as_ref()
            .is_some_and(|remote| !remote.loaded)
        {
            self.notice = "题目尚未加载完成，请稍后再初始化代码".into();
            cx.notify();
            return;
        }
        let starter = self.problem().starter_for(self.practice_language);
        let source = self.editor.read(cx).value().to_string();
        if source == starter {
            self.notice = "当前代码已经是题目初始模板".into();
            cx.notify();
            return;
        }
        let directory = self.store.workspace(self.problem().id);
        let backup = (1..)
            .map(|number| {
                directory.join(format!(
                    "solution-before-reset-{number}.{}",
                    if self.practice_language == PracticeLanguage::Go {
                        "go"
                    } else {
                        "java"
                    }
                ))
            })
            .find(|path| !path.exists())
            .unwrap();
        if let Err(error) = leetdesk::store::atomic_write(&backup, source.as_bytes()) {
            self.notice = format!("备份当前代码失败，初始化已取消：{error:#}");
            cx.notify();
            return;
        }
        let edits = leetdesk::editing::diff(&source, &starter);
        if !self.editor.update(cx, |state, cx| {
            state.apply_programmatic_edits(&edits, window, cx)
        }) {
            self.notice = "正在输入，请结束输入后再初始化代码".into();
            cx.notify();
            return;
        }
        self.changed(cx);
        self.notice = match self.save(cx) {
            Ok(()) => format!(
                "已恢复题目初始代码；原代码备份在 {} · 可撤销",
                backup.display()
            ),
            Err(error) => format!("代码已初始化，保存失败：{error:#}"),
        };
        cx.notify();
    }

    fn organize_imports(
        &mut self,
        _: &OrganizeImports,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.practice_language == PracticeLanguage::Java {
            self.notice = "Java 不需要整理导入；请直接编辑 import 语句".into();
            cx.notify();
            return;
        }
        let source = self.editor.read(cx).value().to_string();
        let id = self.problem().id;
        let revision = self.editor.read(cx).document_revision();
        let workspace = self.store.workspace(id);
        self.notice = "正在整理 Go 导入…".into();
        let input = source.clone();
        let task = cx.background_executor().spawn(async move {
            let text = runner::organize_go_imports(&input, &workspace)?;
            let patch = leetdesk::editing::diff(&input, &text);
            Ok::<_, anyhow::Error>((text, patch))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result: Result<(String, leetdesk::editing::Edits)> = task.await;
            _ = this.update_in(cx, |this, window, cx| {
                if this.problem().id != id
                    || this.editor.read(cx).document_revision() != revision
                    || this.editor.read(cx).value().as_ref() != source
                {
                    this.notice = "代码已发生变化，请再次整理导入".into();
                    cx.notify();
                    return;
                }
                match result {
                    Ok((text, edits)) => {
                        if text == source {
                            this.notice = "导入已是最新，无需调整".into();
                            cx.notify();
                            return;
                        }
                        if !this.editor.update(cx, |state, cx| {
                            state.apply_programmatic_edits(&edits, window, cx)
                        }) {
                            this.notice = "正在输入，请输入完成后再整理导入".into();
                            cx.notify();
                            return;
                        }
                        this.changed(cx);
                        this.notice = match this.save(cx) {
                            Ok(()) => "已整理 Go 导入并保存 · 可撤销".into(),
                            Err(error) => format!("导入已整理，保存失败：{error}"),
                        };
                    }
                    Err(error) => this.notice = format!("整理导入失败：{error:#}"),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn import_notes(&mut self, _: &ImportNotes, window: &mut Window, cx: &mut Context<Self>) {
        self.import_notes_from(false, window, cx);
    }

    fn import_notes_from(
        &mut self,
        directories: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = self.save(cx) {
            self.notice = error.to_string();
            cx.notify();
            return;
        }
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: !directories,
            directories,
            multiple: true,
            prompt: Some(if directories {
                ui::text("选择包含 Markdown 的文件夹", cx).into()
            } else {
                ui::text("选择 Markdown 文件", cx).into()
            }),
        });
        cx.spawn_in(window, async move |this, cx| {
            let selected = prompt.await;
            _ = this.update_in(cx, |this, window, cx| {
                match selected {
                    Ok(Ok(Some(paths))) => {
                        let mut files = vec![];
                        for path in paths {
                            collect_markdown(&path, &mut files);
                        }
                        if files.is_empty() {
                            this.notice = "所选位置没有 .md 或 .markdown 文件".into();
                            cx.notify();
                            return;
                        }
                        let mut success = 0;
                        let mut errors = vec![];
                        for path in files {
                            match this
                                .store
                                .import_note(&path, this.problem().id, &this.problems)
                            {
                                Ok(_) => success += 1,
                                Err(error) => errors.push(format!("{}：{error}", path.display())),
                            }
                        }
                        this.explanations = this.store.notes(this.problem()).unwrap_or_default();
                        this.document_view = None;
                        this.catalog_view
                            .replace_progress(this.store.all_progress());
                        let imported_progress = this.store.progress(this.problem().id);
                        this.progress.tags = imported_progress.tags;
                        this.tag_editor.update(cx, |state, cx| {
                            state.set_value(this.progress.tags.join(", "), window, cx)
                        });
                        this.document_tab = DocumentTab::Explanation;
                        if !this.explanations.is_empty() {
                            this.selected_note = this.explanations.len() - 1;
                        }
                        this.notice = format!(
                            "已导入 {success} 篇讲解{}",
                            if errors.is_empty() {
                                String::new()
                            } else {
                                format!("；{}", errors.join("；"))
                            }
                        );
                    }
                    Ok(Err(error)) => this.notice = error.to_string(),
                    Err(error) => this.notice = error.to_string(),
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn import_catalog(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(ui::text("导入题目 JSON（格式参见 assets/problems.json）", cx).into()),
        });
        cx.spawn(async move |this, cx| {
            let result = prompt.await;
            _ = this.update(cx, |this, cx| {
                if let Ok(Ok(Some(paths))) = result
                    && let Some(path) = paths.first()
                {
                    let current_id = this.problem().id;
                    match this.store.import_catalog(path) {
                        Ok(count) => {
                            this.problems = this.store.load_catalog().unwrap();
                            this.document_view = None;
                            this.catalog_view.replace(&this.problems);
                            this.current = this
                                .problems
                                .iter()
                                .position(|p| p.id == current_id)
                                .unwrap_or(0);
                            this.notice = format!("已导入 {count} 道题目");
                        }
                        Err(error) => this.notice = format!("导入失败：{error:#}"),
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn toggle_favorite(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.progress.favorite = !self.progress.favorite;
        let _ = self.store.save_progress(self.problem().id, &self.progress);
        self.catalog_view
            .update_progress(self.problem().id, &self.progress);
        cx.notify();
    }

    fn mark_review(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.progress.state = if self.progress.state == "待复习" {
            "尝试中"
        } else {
            "待复习"
        }
        .into();
        let _ = self.store.save_progress(self.problem().id, &self.progress);
        self.catalog_view
            .update_progress(self.problem().id, &self.progress);
        cx.notify();
    }

    fn copy_submission(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let source = self.editor.read(cx).value().to_string();
        let source = online::submission_source(self.practice_language, &source);
        cx.write_to_clipboard(ClipboardItem::new_string(source));
        self.notice = if self.practice_language == PracticeLanguage::Go {
            "已复制解法（移除了本地 package 声明）"
        } else {
            "已复制 Java 解法"
        }
        .into();
        cx.notify();
    }

    fn save_action(&mut self, _: &Save, window: &mut Window, cx: &mut Context<Self>) {
        if self.document_tab == DocumentTab::Notes
            && (self
                .note_editor
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
                || self.note_preview.read(cx).focus_handle().is_focused(window))
        {
            self.notice = match self.save(cx) {
                Ok(()) => "笔记已保存为 notes.md · Markdown 预览已更新".into(),
                Err(error) => format!("笔记保存失败：{error:#}"),
            };
            cx.notify();
            return;
        }
        self.format_on_save(window, cx);
    }

    fn format_on_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Persist the draft first, including incomplete Go. A failed formatter
        // must never prevent code, cases or notes from being saved.
        if let Err(error) = self.save(cx) {
            self.notice = format!("保存失败：{error}");
            cx.notify();
            return;
        }
        let id = self.problem().id;
        let source = self.editor.read(cx).value().to_string();
        let revision = self.editor.read(cx).document_revision();
        let practice_language = self.practice_language;
        let formatter = if practice_language == PracticeLanguage::Go {
            "gofmt"
        } else {
            "google-java-format"
        };
        self.notice = format!("已保存，正在运行 {formatter}…");
        let input = source.clone();
        let task = cx.background_executor().spawn(async move {
            (match practice_language {
                PracticeLanguage::Go => runner::format_go(&input),
                PracticeLanguage::Java => runner::format_java(&input),
            })
            .map(|text| {
                let edits = leetdesk::editing::diff(&input, &text);
                (text, edits)
            })
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            _ = this.update_in(cx, |this, window, cx| {
                // A formatter for an older snapshot may not replace newer edits.
                if this.problem().id != id || this.practice_language != practice_language {
                    return;
                }
                if this.editor.read(cx).document_revision() != revision
                    || this.editor.read(cx).value().as_ref() != source
                {
                    this.notice = "代码已变化，已忽略旧格式化结果".into();
                    cx.notify();
                    return;
                }
                match result {
                    Ok((text, edits)) => {
                        if text != source {
                            if !this.editor.update(cx, |state, cx| {
                                state.apply_programmatic_edits(&edits, window, cx)
                            }) {
                                this.notice = "草稿已保存，请输入完成后再保存格式化".into();
                                cx.notify();
                                return;
                            }
                            this.changed(cx);
                        }
                        this.notice = match this.save(cx) {
                            Ok(()) if text == source => {
                                format!("已保存 · 代码已符合 {formatter} 格式")
                            }
                            Ok(()) => format!("已保存并使用 {formatter} 格式化 · 可撤销"),
                            Err(error) => format!("格式化完成，保存失败：{error}"),
                        };
                    }
                    Err(error) => {
                        this.notice = format!("已保存原代码，{formatter} 未完成：{error}")
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn toggle_wrap(&mut self, _: &ToggleWrap, window: &mut Window, cx: &mut Context<Self>) {
        self.soft_wrap = !self.soft_wrap;
        for editor in [&self.editor, &self.reference_editor, &self.cases_editor] {
            editor.update(cx, |state, cx| {
                state.set_soft_wrap(self.soft_wrap, window, cx)
            });
        }
        self.save_editor_preferences(cx);
        cx.notify();
    }

    fn schedule_type_hints(&mut self, cx: &mut Context<Self>) {
        let Some(service) = self.language.clone() else {
            return;
        };
        let source = self.editor.read(cx).value().to_string();
        let revision = self.editor.read(cx).document_revision();
        let epoch = self.language_epoch;
        self.hint_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(350))
                .await;
            let snapshot = source.clone();
            let task = cx
                .background_executor()
                .spawn(async move { service.type_hints(&snapshot) });
            if let Ok(hints) = task.await {
                _ = this.update(cx, |this, cx| {
                    if this.language_epoch != epoch
                        || this.editor.read(cx).document_revision() != revision
                        || this.editor.read(cx).value().as_ref() != source
                    {
                        return;
                    }
                    this.editor.update(cx, |state, cx| {
                        state.set_inlay_hints(
                            hints
                                .into_iter()
                                .map(|(offset, label)| (offset, label.into()))
                                .collect(),
                            cx,
                        )
                    });
                });
            }
        }));
    }

    fn filtered_indices(&self) -> Rc<[usize]> {
        self.catalog_view.filter_in(
            &self.filter,
            &self.difficulty_filter,
            &self.query,
            &self.study_plan_filter,
            &self.status_filter,
        )
    }
    fn question_row(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id(("problem", self.problems[index].id as usize))
            .test_support()
            .child(self.reading_question(
                index,
                format!("flat-{}", self.problems[index].id),
                None,
                cx,
            ))
    }
    fn select_learning(&mut self, filter: String, cx: &mut Context<Self>) {
        if let Some((chapter, pattern)) = leetdesk::learning::find(&filter) {
            self.expanded_chapters.insert(chapter.id.clone());
            if let Some(pattern) = pattern {
                self.expanded_patterns.insert(pattern.id.clone());
            }
        }
        self.learning_overview = leetdesk::learning::find(&filter)
            .is_none_or(|(_, pattern)| pattern.is_none())
            || !leetdesk::learning::matches(
                &filter,
                &leetdesk::learning::memberships(&self.problem().slug),
            );
        self.active_section = 0;
        self.pending_section = None;
        self.filter = filter;
        self.document_tab = DocumentTab::Explanation;
        self.document_view = None;
        cx.notify();
    }

    fn search_problems(&mut self, _: &SearchProblems, window: &mut Window, cx: &mut Context<Self>) {
        self.show_categories = true;
        self.focus_mode = false;
        self.settings_open = false;
        self.search.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    fn topic_row(&self, label: &str, custom: bool, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let value = if custom {
            format!("@custom:{label}")
        } else {
            label.to_string()
        };
        let selected = self.filter == value;
        let text = if label == "全部题目" {
            "全部主题"
        } else {
            label
        }
        .to_string();
        Button::new(gpui_kit::SharedString::from(format!(
            "{}-{label}",
            if custom { "custom-tag" } else { "topic" }
        )))
        .label(if custom { text } else { ui::text(text, cx) })
        .small()
        .ghost()
        .w_full()
        .justify_start()
        .rounded_md()
        .text_sm()
        .cursor_pointer()
        .text_color(theme::color(if selected { ACCENT } else { MUTED }, cx))
        .when(selected, |el| el.bg(theme::color(0x322630, cx)))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.filter = value.clone();
            cx.notify();
        }))
        .into_any_element()
    }
    fn notes_document(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let edit = div()
            .id("notes-editor")
            .size_full()
            .min_w_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .text_color(theme::color(MUTED, cx))
                    .child(ui::text("Markdown 源码", cx)),
            )
            .child(
                Editor::new(&self.note_editor)
                    .size_full()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .font_family(self.code_font().to_owned())
                    .text_size(px(14.)),
            )
            .into_any_element();
        let origin = self.store.workspace(self.problem().id);
        let image_origin = origin.clone();
        let preview = div()
            .id("notes-preview-content")
            .size_full()
            .min_w_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .text_color(theme::color(MUTED, cx))
                    .child(ui::text("保存后的预览", cx)),
            )
            .child(
                div()
                    .id("notes-preview-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_4()
                    .when(self.saved_note.is_empty(), |el| {
                        el.child(
                            div()
                                .text_sm()
                                .text_color(theme::color(MUTED, cx))
                                .child(ui::text("写下思路并保存，Markdown 将在这里渲染。", cx)),
                        )
                    })
                    .when(!self.saved_note.is_empty(), |el| {
                        el.child(
                            TextView::new(&self.note_preview)
                                .markdown_extensions(crate::markdown::extensions())
                                .selectable(true)
                                .style(document_style(cx))
                                .text_size(px(15.))
                                .image_source(move |uri| {
                                    local_document_path(uri.as_ref(), Some(&image_origin))
                                        .map_or_else(|| uri.clone().into(), |path| path.into())
                                })
                                .on_link_click(move |url, _, _, cx| {
                                    if let Some(path) =
                                        local_document_path(url.as_ref(), Some(&origin))
                                    {
                                        if let Ok(uri) = language::file_uri(&path) {
                                            cx.open_url(&uri);
                                        }
                                    } else {
                                        cx.open_url(url.as_ref());
                                    }
                                }),
                        )
                    }),
            )
            .into_any_element();
        let body = match self.notes_view {
            NotesView::Edit => edit,
            NotesView::Preview => preview,
            NotesView::Split => v_resizable(("notes-split-pane", self.problem().id as usize))
                .child(
                    resizable_panel()
                        .size(px(270.))
                        .min_size(px(120.))
                        .child(edit),
                )
                .child(resizable_panel().min_size(px(100.)).child(preview))
                .into_any_element(),
        };
        div()
            .size_full()
            .min_w_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .px_3()
                    .py_2()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .border_b_1()
                    .border_color(theme::color(LINE, cx))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .justify_between()
                            .items_center()
                            .child(div().text_xs().text_color(theme::color(MUTED, cx)).child(
                                ui::text(
                                    format!(
                                        "notes.md · {}",
                                        if self.note_dirty {
                                            "未保存"
                                        } else {
                                            "已保存"
                                        }
                                    ),
                                    cx,
                                ),
                            ))
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_1()
                                    .child(
                                        Button::new("import-note-bundle")
                                            .small()
                                            .ghost()
                                            .label(ui::text("导入笔记包", cx))
                                            .on_click(cx.listener(Self::import_note_bundle)),
                                    )
                                    .child(
                                        Button::new("export-note-bundle")
                                            .small()
                                            .ghost()
                                            .label(ui::text("导出全部笔记", cx))
                                            .on_click(cx.listener(Self::export_note_bundle)),
                                    )
                                    .child(
                                        Button::new("backup-upload")
                                            .small()
                                            .ghost()
                                            .label(ui::text("上传备份", cx))
                                            .on_click(cx.listener(Self::upload_backup)),
                                    )
                                    .child(
                                        Button::new("backup-download")
                                            .small()
                                            .ghost()
                                            .label(ui::text("取回备份", cx))
                                            .on_click(cx.listener(Self::download_backup)),
                                    )
                                    .child(
                                        Button::new("open-notes-file")
                                            .small()
                                            .ghost()
                                            .label(ui::text("打开 MD", cx))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                let path = this
                                                    .store
                                                    .workspace(this.problem().id)
                                                    .join("notes.md");
                                                if let Err(error) = this.save_notes_document(cx) {
                                                    this.notice =
                                                        format!("笔记保存失败：{error:#}");
                                                } else if let Ok(uri) = language::file_uri(&path) {
                                                    cx.open_url(&uri);
                                                }
                                                cx.notify();
                                            })),
                                    ),
                            ),
                    )
                    .child(
                        div().flex().justify_between().items_center().child(
                            div()
                                .flex()
                                .gap_1()
                                .children(
                                    [
                                        (NotesView::Edit, "编辑", "notes-edit"),
                                        (NotesView::Split, "分栏", "notes-split"),
                                        (NotesView::Preview, "预览", "notes-preview"),
                                    ]
                                    .into_iter()
                                    .map(
                                        |(mode, label, id)| {
                                            Button::new(id)
                                                .label(ui::text(label, cx))
                                                .small()
                                                .ghost()
                                                .when(self.notes_view == mode, |button| {
                                                    button.text_color(theme::color(ACCENT, cx))
                                                })
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    if mode == NotesView::Preview
                                                        && let Err(error) =
                                                            this.save_notes_document(cx)
                                                    {
                                                        this.notice =
                                                            format!("笔记保存失败：{error:#}");
                                                    } else {
                                                        this.notes_view = mode;
                                                    }
                                                    cx.notify();
                                                }))
                                        },
                                    ),
                                )
                                .child(
                                    Button::new("save-notes")
                                        .small()
                                        .primary()
                                        .label(ui::text("保存笔记", cx))
                                        .on_click(cx.listener(Self::save_notes_click)),
                                ),
                        ),
                    ),
            )
            .child(div().flex_1().min_h_0().min_w_0().child(body))
    }

    fn document(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs = [
            (DocumentTab::Description, "题目描述"),
            (DocumentTab::Explanation, "学习讲解"),
            (DocumentTab::Notes, "我的笔记"),
        ];
        let mut content = div()
            .id("document-content")
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .size_full();
        if self.document_tab == DocumentTab::Notes {
            content = content.child(self.notes_document(cx));
        } else {
            let key = (
                self.problem().id,
                self.selected_note,
                self.document_tab as u8,
                if self.document_tab == DocumentTab::Explanation {
                    self.filter.clone()
                } else {
                    String::new()
                },
            );
            if self
                .document_view
                .as_ref()
                .is_none_or(|cached| cached.0 != key)
            {
                let problem = self.problem();
                let origin =
                    if self.document_tab == DocumentTab::Explanation && self.selected_note > 0 {
                        self.explanations
                            .get(self.selected_note)
                            .and_then(|(name, _)| {
                                let path = self
                                    .store
                                    .root
                                    .join("notes")
                                    .join(problem.id.to_string())
                                    .join(name)
                                    .with_extension("origin");
                                fs::read_to_string(path).ok().map(PathBuf::from)
                            })
                            .and_then(|path| path.parent().map(std::path::Path::to_path_buf))
                    } else {
                        None
                    };
                let markdown = if self.document_tab == DocumentTab::Description {
                    problem.description.clone()
                } else {
                    let compose = if self.workspace_mode == WorkspaceMode::Study {
                        leetdesk::learning::reading_lesson
                    } else {
                        |_,
                         filter: &str,
                         problem: &Problem,
                         language,
                         site,
                         problems: &[Problem],
                         supplement: Option<(&str, &str)>| {
                            leetdesk::learning::lesson(
                                filter, problem, language, site, problems, supplement,
                            )
                        }
                    };
                    compose(
                        self.learning_overview,
                        &self.filter,
                        problem,
                        ui::language(cx),
                        self.store.site(),
                        &self.problems,
                        self.explanations
                            .get(self.selected_note)
                            .map(|(name, body)| {
                                (ui::text(name.clone(), cx), catalog::markdown_body(body))
                            })
                            .as_ref()
                            .map(|(name, body)| (name.as_str(), *body)),
                    )
                };
                let state =
                    if self.document_tab == DocumentTab::Description && problem.online.is_some() {
                        cx.new(|cx| {
                            let placeholder =
                                ui::text("<p>正在从 LeetCode 加载题面和 Go 模板…</p>", cx);
                            TextViewState::html(
                                if markdown.is_empty() {
                                    &placeholder
                                } else {
                                    &markdown
                                },
                                cx,
                            )
                            .scrollable(true)
                        })
                    } else {
                        cx.new(|cx| TextViewState::markdown(&markdown, cx).scrollable(true))
                    };
                self.document_view = Some((key, state, origin));
            }
            let (_, state, origin) = self.document_view.as_ref().unwrap();
            let origin = origin.clone();
            let image_origin = origin.clone();
            content = content.child(
                TextView::new(state)
                    .scrollable(true)
                    .font_family(gpui_kit::component::Theme::global(cx).font_family.clone())
                    .line_height(px(if self.workspace_mode == WorkspaceMode::Study {
                        27.
                    } else {
                        23.
                    }))
                    .code_block_actions(|block, _, cx| {
                        let code = block.code();
                        Button::new("copy-lesson-code")
                            .icon(gpui_kit::assets::IconName::Copy)
                            .small()
                            .ghost()
                            .tooltip(ui::text("复制代码", cx))
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(code.to_string()))
                            })
                    })
                    .markdown_extensions(crate::markdown::extensions())
                    .image_source(move |uri| {
                        if let Some(path) =
                            local_document_path(uri.as_ref(), image_origin.as_deref())
                        {
                            return path.into();
                        }
                        uri.clone().into()
                    })
                    .on_link_click(move |url, _, _, cx| {
                        if let Some(path) = local_document_path(url.as_ref(), origin.as_deref()) {
                            if let Ok(uri) = language::file_uri(&path) {
                                cx.open_url(&uri);
                            }
                        } else {
                            cx.open_url(url.as_ref());
                        }
                    })
                    .selectable(true)
                    .style(document_style(cx))
                    .size_full()
                    .text_size(px(if self.workspace_mode == WorkspaceMode::Study {
                        18.
                    } else {
                        15.
                    })),
            );
        }
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme::color(BG, cx))
            .when(self.workspace_mode == WorkspaceMode::Study, |el| {
                el.child(self.reader_heading(cx))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .h_12()
                    .px_3()
                    .gap_1()
                    .border_b_1()
                    .border_color(theme::color(LINE, cx))
                    .children(tabs.into_iter().enumerate().map(|(index, (tab, label))| {
                        Button::new(("document-tab", index))
                            .accessibility_label(ui::text(label, cx))
                            .child(div().text_size(px(16.)).child(ui::text(label, cx)))
                            .small()
                            .ghost()
                            .h_full()
                            .rounded_none()
                            .border_b_3()
                            .border_color(theme::color(
                                if self.document_tab == tab { ACCENT } else { BG },
                                cx,
                            ))
                            .when(self.document_tab == tab, |button| {
                                button
                                    .text_color(theme::color(ACCENT, cx))
                                    .font_weight(FontWeight::BOLD)
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.document_tab = tab;
                                cx.notify();
                            }))
                    })),
            )
            .when(
                self.document_tab == DocumentTab::Explanation
                    && self
                        .explanations
                        .iter()
                        .any(|(_, body)| !body.trim().is_empty()),
                |el| {
                    el.child(
                        div().px_3().py_2().flex().flex_wrap().gap_1().children(
                            self.explanations
                                .iter()
                                .enumerate()
                                .filter(|(_, (_, body))| !body.trim().is_empty())
                                .map(|(index, (name, _))| {
                                    Button::new(("explanation", index))
                                        .label(ui::text(name.clone(), cx))
                                        .small()
                                        .ghost()
                                        .when(index == self.selected_note, |button| {
                                            button.text_color(theme::color(ACCENT, cx))
                                        })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.selected_note = index;
                                            cx.notify();
                                        }))
                                }),
                        ),
                    )
                },
            )
            .child(content)
            .when(self.filters_open, |el| {
                el.child(
                    div()
                        .px_3()
                        .py_2()
                        .border_t_1()
                        .border_color(theme::color(LINE, cx))
                        .child(Input::new(&self.tag_editor).id("custom-tags").small()),
                )
            })
    }

    fn settings_panel(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        div()
            .id("settings-panel")
            .track_focus(&self.settings_focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::color(BG, cx))
            .child(
                div()
                    .h_9()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .bg(theme::color(PANEL, cx))
                    .border_b_1()
                    .border_color(theme::color(LINE, cx))
                    .child(
                        Button::new("solution-tab")
                            .label(self.practice_language.filename())
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.settings_open = false;
                                this.editor.update(cx, |state, cx| state.focus(window, cx));
                                cx.notify();
                            })),
                    )
                    .child(div().flex_1().text_sm().child(ui::text("设置", cx)))
                    .child(
                        Button::new("close-settings")
                            .icon(gpui_kit::assets::IconName::X)
                            .accessibility_label(ui::text("关闭", cx))
                            .small()
                            .ghost()
                            .tooltip(ui::text("关闭", cx))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.settings_open = false;
                                this.editor.update(cx, |state, cx| state.focus(window, cx));
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id("settings-scroll")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_5()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(ui::text("设置", cx)),
                    )
                    .child(self.language_settings(cx))
                    .child(
                        div()
                            .mt_5()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(ui::text("外观", cx)),
                    )
                    .child(div().mt_2().flex().gap_2().flex_wrap().children(
                        Appearance::ALL.into_iter().map(|appearance| {
                            Button::new(gpui_kit::SharedString::from(format!(
                                "appearance-{}",
                                appearance.key()
                            )))
                            .label(format!(
                                "{}{}",
                                if theme::current(cx) == appearance {
                                    "✓ "
                                } else {
                                    ""
                                },
                                ui::text(appearance.label(), cx)
                            ))
                            .small()
                            .ghost()
                            .border_1()
                            .border_color(theme::color(
                                if theme::current(cx) == appearance {
                                    ACCENT
                                } else {
                                    LINE
                                },
                                cx,
                            ))
                            .on_click(cx.listener(
                                move |this, _, _, cx| this.set_appearance(appearance, cx),
                            ))
                        }),
                    ))
                    .child(
                        div()
                            .mt_5()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(ui::text("编辑器", cx)),
                    )
                    .child(self.font_settings(cx))
                    .child(
                        div()
                            .mt_3()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(div().flex_1().text_sm().child(ui::text("代码字号", cx)))
                            .child(
                                Button::new("font-smaller")
                                    .label("−")
                                    .small()
                                    .ghost()
                                    .tooltip(ui::text("减小字号", cx))
                                    .disabled(self.editor_font_size <= 12)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.editor_font_size =
                                            this.editor_font_size.saturating_sub(1).max(12);
                                        this.save_editor_preferences(cx);
                                    })),
                            )
                            .child(div().text_sm().child(self.editor_font_size.to_string()))
                            .child(
                                Button::new("font-larger")
                                    .label("+")
                                    .small()
                                    .ghost()
                                    .tooltip(ui::text("增大字号", cx))
                                    .disabled(self.editor_font_size >= 24)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.editor_font_size = (this.editor_font_size + 1).min(24);
                                        this.save_editor_preferences(cx);
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .mt_3()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(div().text_sm().child(ui::text("自动折行", cx)))
                            .child(
                                Button::new("settings-wrap")
                                    .label(ui::text(
                                        if self.soft_wrap {
                                            "开启"
                                        } else {
                                            "关闭折行"
                                        },
                                        cx,
                                    ))
                                    .small()
                                    .ghost()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.toggle_wrap(&ToggleWrap, window, cx)
                                    })),
                            ),
                    )
                    .child(self.editor_theme_settings(cx))
                    .child(
                        div()
                            .mt_5()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(ui::text("工具与导入", cx)),
                    )
                    .child(
                        div()
                            .mt_2()
                            .flex()
                            .gap_2()
                            .flex_wrap()
                            .child(
                                Button::new("import-md")
                                    .label(ui::text("导入 MD 文件", cx))
                                    .small()
                                    .ghost()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.import_notes(&ImportNotes, window, cx)
                                    })),
                            )
                            .child(
                                Button::new("import-md-folder")
                                    .label(ui::text("导入文件夹", cx))
                                    .small()
                                    .ghost()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.import_notes_from(true, window, cx)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .mt_5()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(ui::text("快捷键", cx)),
                    )
                    .child(
                        div()
                            .mt_2()
                            .text_xs()
                            .text_color(theme::color(MUTED, cx))
                            .child(ui::text(
                                if cfg!(target_os = "macos") {
                                    "⌘, 设置 · ⌘S 保存 · ⌘↵ 运行"
                                } else {
                                    "Ctrl+, 设置 · Ctrl+S 保存 · Ctrl+Enter 运行"
                                },
                                cx,
                            )),
                    ),
            )
            .into_any_element()
    }

    fn editor_panel(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        if self.settings_open {
            return self.settings_panel(cx);
        }
        if let Some(path) = &self.reference_path {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .child(
                    div()
                        .h_9()
                        .px_3()
                        .flex()
                        .items_center()
                        .justify_between()
                        .bg(theme::color(PANEL, cx))
                        .child(div().text_xs().child(path.clone()))
                        .child(
                            Button::new("close-definition")
                                .label(ui::text("返回解法", cx))
                                .small()
                                .ghost()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.reference_path = None;
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    Editor::new(&self.reference_editor)
                        .readonly(true)
                        .size_full()
                        .flex_1()
                        .min_h_0()
                        .font_family(self.code_font().to_owned())
                        .text_size(px(self.editor_font_size as f32)),
                )
                .into_any_element();
        }
        div()
            .size_full()
            .min_w_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .h_9()
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_1()
                    .bg(theme::color(PANEL, cx))
                    .border_b_1()
                    .border_color(theme::color(LINE, cx))
                    .child(
                        div()
                            .h_full()
                            .px_3()
                            .flex()
                            .gap_2()
                            .items_center()
                            .bg(theme::color(BG, cx))
                            .border_t_2()
                            .border_color(theme::color(ACCENT, cx))
                            .child(
                                gpui_kit::component::Icon::new(
                                    gpui_kit::assets::IconName::FileCode,
                                )
                                .small(),
                            )
                            .child(div().text_sm().child(self.practice_language.filename()))
                            .when(self.dirty, |tab| {
                                tab.child(
                                    div()
                                        .text_xs()
                                        .text_color(theme::color(ACCENT, cx))
                                        .child("●"),
                                )
                            }),
                    )
                    .child(div().flex_1())
                    .children(
                        [PracticeLanguage::Go, PracticeLanguage::Java]
                            .into_iter()
                            .map(|language| {
                                Button::new(if language == PracticeLanguage::Go {
                                    "language-go"
                                } else {
                                    "language-java"
                                })
                                .label(language.label())
                                .small()
                                .ghost()
                                .when(self.practice_language == language, |button| {
                                    button.text_color(theme::color(ACCENT, cx))
                                })
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        this.switch_language(language, window, cx)
                                    },
                                ))
                            }),
                    )
                    .child(
                        Button::new("toggle-wrap")
                            .label(ui::text("折行", cx))
                            .small()
                            .ghost()
                            .tooltip(ui::text("折行", cx))
                            .when(self.soft_wrap, |button| {
                                button.text_color(theme::color(ACCENT, cx))
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_wrap(&ToggleWrap, window, cx)
                            })),
                    ),
            )
            .child(
                div()
                    .min_h(px(30.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_1()
                    .flex_wrap()
                    .bg(theme::color(BG, cx))
                    .border_b_1()
                    .border_color(theme::color(LINE, cx))
                    .child(
                        Button::new("initialize-code")
                            .label(ui::text("恢复初始模板", cx))
                            .small()
                            .ghost()
                            .on_click(cx.listener(Self::initialize_code)),
                    )
                    .when(self.practice_language == PracticeLanguage::Go, |row| {
                        row.child(
                            Button::new("organize-imports")
                                .label(ui::text("整理导入", cx))
                                .small()
                                .ghost()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.organize_imports(&OrganizeImports, window, cx)
                                })),
                        )
                    })
                    .child(
                        Button::new("format")
                            .label(ui::text("格式化", cx))
                            .small()
                            .ghost()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.format(&Format, window, cx)),
                            ),
                    )
                    .child(
                        Button::new("copy-code")
                            .label(ui::text("复制解法", cx))
                            .small()
                            .ghost()
                            .on_click(cx.listener(Self::copy_submission)),
                    ),
            )
            .child(
                Editor::new(&self.editor)
                    .readonly(
                        self.problem()
                            .online
                            .as_ref()
                            .is_some_and(|_| !self.problem().supports(self.practice_language)),
                    )
                    .size_full()
                    .min_w_0()
                    .flex_1()
                    .min_h_0()
                    .font_family(self.code_font().to_owned())
                    .text_size(px(self.editor_font_size as f32)),
            )
            .into_any_element()
    }

    fn bottom_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs = [
            (BottomTab::Cases, "测试用例"),
            (BottomTab::Results, "运行结果"),
            (BottomTab::Diagnostics, "代码问题"),
        ];
        let content = match self.bottom_tab {
            BottomTab::Cases => div()
                .size_full()
                .flex()
                .flex_col()
                .child(
                    div()
                        .px_4()
                        .py_2()
                        .border_b_1()
                        .border_color(theme::color(LINE, cx))
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme::color(ACCENT, cx))
                                .child(ui::text(
                                    if self.problem().online.is_some() {
                                        "在线测试输入"
                                    } else {
                                        "本地测试数据"
                                    },
                                    cx,
                                )),
                        )
                        .child(div().text_xs().text_color(theme::color(MUTED, cx)).child(
                            ui::text(
                                if self.problem().online.is_some() {
                                    "每个参数占一行 · 修改后点击上方测试"
                                } else {
                                    "按参数顺序填写 JSON · 链表和树使用数组"
                                },
                                cx,
                            ),
                        )),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .m_3()
                        .rounded_md()
                        .border_1()
                        .border_color(theme::color(LINE, cx))
                        .bg(theme::color(BG, cx))
                        .child(
                            Editor::new(&self.cases_editor)
                                .size_full()
                                .font_family(self.code_font().to_owned())
                                .text_size(px(13.)),
                        ),
                )
                .into_any_element(),
            BottomTab::Results => {
                let output = self.output.trim();
                let heading = output.lines().next().unwrap_or("尚无运行结果");
                let failed = output.contains("失败")
                    || output.contains("错误")
                    || output.contains("✗")
                    || output.contains("Wrong Answer")
                    || output.contains("Runtime Error")
                    || output.contains("Compile Error")
                    || output.contains("Time Limit Exceeded");
                let passed = !failed && (output.contains("通过") || output.contains("Accepted"));
                let tone = if failed {
                    0xff928c
                } else if passed {
                    ACCENT
                } else {
                    MUTED
                };
                let detail = output
                    .split_once('\n')
                    .map_or("", |(_, detail)| detail)
                    .replace("\r\n", "\n");
                div()
                    .id("results-scroll")
                    .size_full()
                    .overflow_y_scroll()
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .font_family(self.code_font().to_owned())
                    .when(self.has_run_input, |panel| {
                        panel.child(
                            div()
                                .id("run-input-snapshot")
                                .flex_shrink_0()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .p_3()
                                .rounded_md()
                                .border_1()
                                .border_color(theme::color(LINE, cx))
                                .bg(theme::color(CODE_BG, cx))
                                .child(div().text_xs().text_color(theme::color(MUTED, cx)).child(
                                    ui::text("本次运行的测试用例 · 保留运行开始时的输入", cx),
                                ))
                                .child(
                                    div().h(px(100.)).child(
                                        Editor::new(&self.run_input_editor)
                                            .readonly(true)
                                            .size_full()
                                            .font_family(self.code_font().to_owned())
                                            .text_size(px(13.)),
                                    ),
                                ),
                        )
                    })
                    .child(
                        div()
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(theme::color(
                                if failed {
                                    0x231313
                                } else if passed {
                                    0x102018
                                } else {
                                    CODE_BG
                                },
                                cx,
                            ))
                            .border_l_2()
                            .border_color(theme::color(tone, cx))
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::color(tone, cx))
                            .child(ui::text(heading, cx)),
                    )
                    .when(output.is_empty(), |panel| {
                        panel.child(
                            div()
                                .px_3()
                                .text_xs()
                                .text_color(theme::color(MUTED, cx))
                                .child(ui::text(
                                    "运行代码后，这里会显示每个用例的输入、预期和实际结果。",
                                    cx,
                                )),
                        )
                    })
                    .children(
                        detail
                            .split("\n\n")
                            .filter(|section| !section.trim().is_empty())
                            .map(|section| {
                                let case_ok = section.trim_start().starts_with('✓');
                                let case_failed = section.trim_start().starts_with('✗');
                                div()
                                    .p_3()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(theme::color(
                                        if case_ok {
                                            0x285440
                                        } else if case_failed {
                                            0x61332f
                                        } else {
                                            LINE
                                        },
                                        cx,
                                    ))
                                    .bg(theme::color(CODE_BG, cx))
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .children(
                                        section.lines().filter(|line| !line.trim().is_empty()).map(
                                            |line| {
                                                let line = line.trim();
                                                if line.starts_with('✓') || line.starts_with('✗')
                                                {
                                                    let ok = line.starts_with('✓');
                                                    div()
                                                        .text_sm()
                                                        .font_weight(FontWeight::SEMIBOLD)
                                                        .text_color(theme::color(
                                                            if ok { ACCENT } else { 0xff928c },
                                                            cx,
                                                        ))
                                                        .child(ui::text(line, cx))
                                                        .into_any_element()
                                                } else if let Some((label, value)) =
                                                    line.split_once('：')
                                                {
                                                    let is_error = label.contains("错误")
                                                        || label.contains("失败");
                                                    div()
                                                        .flex()
                                                        .items_start()
                                                        .gap_3()
                                                        .text_xs()
                                                        .child(
                                                            div()
                                                                .w(px(64.))
                                                                .flex_shrink_0()
                                                                .text_color(theme::color(MUTED, cx))
                                                                .child(ui::text(label, cx)),
                                                        )
                                                        .child(
                                                            div()
                                                                .flex_1()
                                                                .min_w_0()
                                                                .text_color(theme::color(
                                                                    if is_error {
                                                                        0xff928c
                                                                    } else {
                                                                        INK
                                                                    },
                                                                    cx,
                                                                ))
                                                                .child(value.trim().to_string()),
                                                        )
                                                        .into_any_element()
                                                } else {
                                                    div()
                                                        .text_xs()
                                                        .text_color(theme::color(INK, cx))
                                                        .child(ui::text(line, cx))
                                                        .into_any_element()
                                                }
                                            },
                                        ),
                                    )
                                    .into_any_element()
                            }),
                    )
                    .into_any_element()
            }
            BottomTab::Diagnostics => div()
                .id("diagnostics-scroll")
                .size_full()
                .overflow_y_scroll()
                .p_3()
                .flex()
                .flex_col()
                .gap_2()
                .when(self.diagnostics.is_empty(), |el| {
                    el.child(
                        div()
                            .p_4()
                            .rounded_md()
                            .border_1()
                            .border_color(theme::color(LINE, cx))
                            .bg(theme::color(CODE_BG, cx))
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::color(
                                        if self.language.is_some()
                                            || self.practice_language == PracticeLanguage::Java
                                        {
                                            ACCENT
                                        } else {
                                            MUTED
                                        },
                                        cx,
                                    ))
                                    .child(ui::text(
                                        if self.practice_language == PracticeLanguage::Java {
                                            "Java 本地编译时检查代码"
                                        } else if self.language.is_some() {
                                            "✓ 暂无代码问题"
                                        } else {
                                            "等待 Go 语言服务"
                                        },
                                        cx,
                                    )),
                            )
                            .child(div().text_xs().text_color(theme::color(MUTED, cx)).child(
                                ui::text(
                                    if self.practice_language == PracticeLanguage::Java {
                                        "运行用例后，编译错误会显示在运行结果中。"
                                    } else if self.language.is_some() {
                                        "继续编写代码，检查结果会实时更新。"
                                    } else {
                                        "连接后会在这里显示错误和警告。"
                                    },
                                    cx,
                                ),
                            )),
                    )
                })
                .children(
                    self.diagnostics
                        .iter()
                        .enumerate()
                        .map(|(index, diagnostic)| {
                            let position = diagnostic.range.start;
                            let (kind, color) = match diagnostic.severity {
                                Some(lsp_types::DiagnosticSeverity::ERROR) => ("错误", 0xff928c),
                                Some(lsp_types::DiagnosticSeverity::WARNING) => ("警告", 0xffcf79),
                                _ => ("提示", 0x89baff),
                            };
                            div()
                                .id(("diagnostic", index))
                                .p_3()
                                .rounded_md()
                                .bg(theme::color(CODE_BG, cx))
                                .border_l_2()
                                .border_color(theme::color(color, cx))
                                .cursor_pointer()
                                .hover(|el| el.bg(theme::color(LINE, cx)))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.reference_path = None;
                                    this.editor.update(cx, |state, cx| {
                                        state.set_cursor_position(position, window, cx)
                                    });
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_3()
                                        .text_xs()
                                        .child(
                                            div()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(theme::color(color, cx))
                                                .child(ui::text(kind, cx)),
                                        )
                                        .child(div().text_color(theme::color(MUTED, cx)).child(
                                            ui::text(
                                                format!(
                                                    "{}:{}",
                                                    position.line + 1,
                                                    position.character + 1
                                                ),
                                                cx,
                                            ),
                                        )),
                                )
                                .child(
                                    div()
                                        .mt_1()
                                        .text_sm()
                                        .text_color(theme::color(INK, cx))
                                        .child(diagnostic.message.clone()),
                                )
                        }),
                )
                .into_any_element(),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::color(PANEL, cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .h_10()
                    .border_b_1()
                    .border_color(theme::color(LINE, cx))
                    .children(tabs.into_iter().enumerate().map(|(index, (tab, label))| {
                        let label = if tab == BottomTab::Diagnostics {
                            format!("{label} ({})", self.diagnostics.len())
                        } else if tab == BottomTab::Results && self.cancellation.is_some() {
                            "运行结果 · 运行中".into()
                        } else {
                            label.into()
                        };
                        div()
                            .h_full()
                            .flex()
                            .items_center()
                            .border_b_2()
                            .border_color(theme::color(
                                if self.bottom_tab == tab {
                                    ACCENT
                                } else {
                                    PANEL
                                },
                                cx,
                            ))
                            .child(
                                Button::new(("bottom-tab", index))
                                    .label(ui::text(label, cx))
                                    .small()
                                    .ghost()
                                    .when(self.bottom_tab == tab, |button| {
                                        button.text_color(theme::color(ACCENT, cx))
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.bottom_tab = tab;
                                        cx.notify();
                                    })),
                            )
                    })),
            )
            .child(div().flex_1().min_h_0().child(content))
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.reading_workspace(window, cx)
    }
}

fn collect_markdown(path: &std::path::Path, output: &mut Vec<PathBuf>) {
    if path.is_file() && leetdesk::store::is_markdown(path) {
        output.push(path.into());
    } else if path.is_dir()
        && let Ok(entries) = fs::read_dir(path)
    {
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|kind| !kind.is_symlink()) {
                collect_markdown(&entry.path(), output);
            }
        }
    }
}

fn local_document_path(value: &str, base: Option<&std::path::Path>) -> Option<PathBuf> {
    if let Ok(url) = url::Url::parse(value) {
        return if url.scheme() == "file" {
            url.to_file_path().ok()
        } else {
            None
        };
    }
    base.map(|base| base.join(value))
}

fn default_note_export_directory(fallback: &std::path::Path) -> PathBuf {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));
    if let Some(home) = home {
        let home = PathBuf::from(home);
        let documents = home.join("Documents");
        if documents.is_dir() {
            return documents;
        }
        if home.is_dir() {
            return home;
        }
    }
    fallback.to_path_buf()
}

impl Drop for Workspace {
    fn drop(&mut self) {
        if let Some(cancel) = &self.cancellation {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}

pub fn launch(store: Store, smoke: bool) -> Result<()> {
    launch_options(store, smoke, None)
}

/// Capture this application's actual GPU-rendered window for design review.
pub fn capture(store: Store, path: PathBuf, width: f32, height: f32) -> Result<()> {
    launch_options(store, false, Some((path, width, height)))
}

fn launch_options(store: Store, smoke: bool, capture: Option<(PathBuf, f32, f32)>) -> Result<()> {
    let appearance = store.appearance();
    let smoke_ok = Arc::new(AtomicBool::new(!smoke));
    let result = smoke_ok.clone();
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            apply_appearance(appearance, cx);
            cx.on_action(|_: &Quit, cx| cx.quit());
            bind_keys(cx);
            let options = WindowOptions {
                titlebar: None,
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(
                        px(capture.as_ref().map_or(1488., |c| c.1)),
                        px(capture.as_ref().map_or(960., |c| c.2)),
                    ),
                    cx,
                ))),
                window_min_size: Some(size(px(1120.), px(650.))),
                app_id: Some("leetdesk".into()),
                show: !smoke && capture.is_none(),
                ..Default::default()
            };
            let (_, view) = gpui_kit::open_window(options, cx, move |window, cx| {
                window.set_window_title("LeetDesk · Go / Java");
                cx.new(|cx| Workspace::new(store, window, cx).expect("无法初始化刷题工作区"))
            })
            .expect("无法打开窗口");
            let handle = cx.windows().last().copied();
            if let Some(handle) = handle {
                let _ = handle.update(cx, |_, window, cx| {
                    if let Some((_, width, height)) = &capture {
                        window.resize(size(px(*width), px(*height)));
                    }
                    view.update(cx, |view, cx| {
                        view.connect_language(window, cx);
                        view.editor.update(cx, |state, cx| state.focus(window, cx));
                    });
                    let closing = view.downgrade();
                    window.on_window_should_close(cx, move |_, cx| {
                        closing
                            .update(cx, |this, cx| match this.save(cx) {
                                Ok(_) => true,
                                Err(error) => {
                                    this.notice = format!("保存失败，窗口保持打开：{error}");
                                    cx.notify();
                                    false
                                }
                            })
                            .unwrap_or(true)
                    });
                });
            }
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            if let Some((path, _, _)) = capture {
                cx.spawn(async move |cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(300))
                        .await;
                    cx.update(|cx| {
                        if let Some(handle) = handle {
                            let _ = handle.update(cx, |_, window, cx| {
                                let _ = window.draw(cx);
                            });
                        }
                    });
                    cx.background_executor()
                        .timer(Duration::from_millis(1500))
                        .await;
                    let captured = cx.update(|cx| -> Result<()> {
                        let handle = handle.ok_or_else(|| anyhow!("capture window missing"))?;
                        handle.update(cx, |_, window, cx| -> Result<()> {
                            capture_frame(window, cx, &path)
                        })?
                    });
                    if let Err(error) = captured {
                        eprintln!("CAPTURE failed: {error:#}");
                        smoke_ok.store(false, Ordering::Relaxed);
                    }
                    cx.update(|cx| cx.quit());
                })
                .detach();
            } else if smoke {
                cx.spawn(async move |cx| {
                    for attempt in 0..30 {
                        cx.background_executor()
                            .timer(Duration::from_millis(500))
                            .await;
                        let mut done = false;
                        cx.update(|cx| {
                            let workspace = view.read(cx);
                            let ready = workspace
                                .language
                                .as_ref()
                                .is_some_and(|service| service.error().is_none());
                            if ready || attempt == 29 {
                                let grammar =
                                    gpui_kit::component::highlighter::LanguageRegistry::singleton()
                                        .language("go")
                                        .is_some();
                                println!(
                                    "SMOKE window=ok gopls={} go_grammar={} problems={}",
                                    ready,
                                    grammar,
                                    workspace.problems.len()
                                );
                                smoke_ok.store(ready && grammar, Ordering::Relaxed);
                                done = true;
                                cx.quit();
                            }
                        });
                        if done {
                            break;
                        }
                    }
                })
                .detach();
            } else {
                cx.activate(true);
            }
        });
    if !result.load(Ordering::Relaxed) {
        return Err(anyhow!("原生窗口或 Go 语言服务启动验证失败"));
    }
    Ok(())
}

#[cfg(feature = "design-review")]
fn capture_frame(window: &mut Window, cx: &mut App, path: &std::path::Path) -> Result<()> {
    let _ = window.draw(cx);
    let image = window.render_to_image()?;
    println!(
        "viewport={:?} scale={}",
        window.viewport_size(),
        window.scale_factor()
    );
    image.save(path)?;
    println!(
        "CAPTURE {} {}x{}",
        path.display(),
        image.width(),
        image.height()
    );
    Ok(())
}

#[cfg(not(feature = "design-review"))]
fn capture_frame(_: &mut Window, _: &mut App, _: &std::path::Path) -> Result<()> {
    Err(anyhow!("Native capture requires --features design-review"))
}

#[cfg(test)]
pub fn apply_theme(cx: &mut App) {
    apply_appearance(Appearance::Dark, cx);
}

pub fn apply_appearance(appearance: Appearance, cx: &mut App) {
    let editor_theme = theme::editor_theme(cx);
    let interface_family = if cx
        .text_system()
        .all_font_names()
        .iter()
        .any(|name| name == "Noto Sans SC")
    {
        "Noto Sans SC"
    } else {
        UI_FONT
    };
    theme::select(appearance, cx);
    let color = |value| rgb(appearance.color(value));
    gpui_kit::component::Theme::change(
        if appearance == Appearance::Dark {
            gpui_kit::component::ThemeMode::Dark
        } else {
            gpui_kit::component::ThemeMode::Light
        },
        None,
        cx,
    );
    gpui_kit::component::Theme::update(cx, |theme| {
        theme.font_family = interface_family.into();
        theme.mono_font_family = CODE_FONT.into();
        theme.shadow = false;
        theme.colors.background = color(BG).into();
        theme.colors.foreground = color(INK).into();
        theme.colors.border = color(LINE).into();
        theme.colors.muted_foreground = color(MUTED).into();
        theme.colors.input = color(PANEL).into();
        theme.colors.popover = color(PANEL).into();
        theme.colors.popover_foreground = color(INK).into();
        theme.colors.sidebar = color(PANEL).into();
        theme.colors.sidebar_foreground = color(INK).into();
        theme.colors.sidebar_border = color(LINE).into();
        theme.colors.selection = color(0x663341).into();
        theme.colors.caret = color(INK).into();
        theme.colors.accent = color(0x4b2531).into();
        theme.colors.accent_foreground = color(INK).into();
        theme.colors.primary = color(ACCENT).into();
        theme.colors.primary_foreground = color(BG).into();
        theme.colors.button_primary = color(ACCENT).into();
        theme.colors.button_primary_hover = color(ACCENT).into();
        theme.colors.button_primary_active = color(ACCENT).into();
        theme.colors.button_primary_foreground = color(BG).into();
        theme.colors.button_foreground = color(INK).into();
        theme.colors.button = color(BG).into();
        theme.colors.ring = color(ACCENT).into();
        theme.colors.blue = color(ACCENT).into();
        let syntax = Arc::make_mut(&mut theme.highlight_theme);
        syntax.style.editor_background = Some(color(BG).into());
        syntax.style.editor_gutter_background = Some(color(BG).into());
        syntax.style.editor_foreground = Some(color(INK).into());
        syntax.style.editor_active_line = Some(color(0x222027).into());
        syntax.style.editor_line_number = Some(color(MUTED).into());
        // Build each style uniformly, including its optional non-color cues.
        let palette = [
            ("keyword", 0xff8f9c, Some(700), None),
            ("string", 0xffe2e7, None, None),
            ("number", 0xffdfcf, None, None),
            ("boolean", 0xffdfcf, Some(700), None),
            ("function", 0xffbec9, Some(600), None),
            ("type", 0xf4bd9f, Some(600), None),
            ("variable", INK, None, None),
            ("property", 0xf4d4db, None, None),
            ("constant", 0xffdfcf, None, None),
            ("operator", 0xffd1d9, None, None),
            ("comment", 0xc7c3c9, None, Some("italic")),
            ("punctuation", 0xeee3e6, None, None),
        ];
        let colors: serde_json::Map<String, serde_json::Value> = palette.into_iter().map(|(name, value, weight, style)| {
        (name.into(), json!({"color": gpui_kit::Hsla::from(rgb(editor_theme.color(value, appearance))), "font_weight": weight, "font_style": style}))
    }).collect();
        syntax.style.syntax = serde_json::from_value(colors.into()).expect("Go 高亮颜色有效");
    });
    let highlights = gpui_kit::component::Theme::global(cx)
        .highlight_theme
        .clone();
    gpui_kit::base::TextViewDefaults::new()
        .with_code_block_highlighter(move |block| {
            let source = block.code();
            let lang = block.lang().unwrap_or_default();
            let mut highlighter =
                gpui_kit::component::highlighter::SyntaxHighlighter::new(lang.as_ref());
            highlighter.update(None, &Rope::from(source.as_ref()), None);
            highlighter.styles(&(0..source.len()), &*highlights)
        })
        .install(cx);
    cx.refresh_windows();
}

fn document_style(cx: &App) -> TextViewStyle {
    let interface_family = gpui_kit::component::Theme::global(cx).font_family.clone();
    let mut code_block = gpui_kit::StyleRefinement::default();
    code_block.text.color = Some(theme::color(INK, cx).into());
    code_block.text.font_family = Some(CODE_FONT.into());
    let mut table = gpui_kit::StyleRefinement::default()
        .rounded_md()
        .bg(theme::color(0x101715, cx));
    if theme::current(cx) != Appearance::Dark {
        table = table.bg(theme::color(BG, cx));
    }
    table.overflow.x = Some(gpui_kit::Overflow::Scroll);
    let table_head = gpui_kit::StyleRefinement::default()
        .bg(theme::color(0x203c34, cx))
        .text_color(theme::color(INK, cx))
        .font_weight(FontWeight::SEMIBOLD);
    let table_cell = gpui_kit::StyleRefinement::default()
        .px_4()
        .py(px(3.))
        .font_family(interface_family.clone())
        .text_size(px(14.))
        .line_height(px(20.));
    TextViewStyle::default()
        .with_dark(theme::current(cx) == leetdesk::appearance::Appearance::Dark)
        .with_foreground(theme::color(INK, cx).into())
        .with_muted_foreground(theme::color(MUTED, cx).into())
        .with_paragraph_gap(gpui_kit::rems(0.9))
        .with_heading({
            let foreground = theme::color(INK, cx);
            move |level| {
                let mut style = gpui_kit::StyleRefinement::default()
                    .text_color(foreground)
                    .font_family(interface_family.clone())
                    .pt(px(6.))
                    .pb(px(4.))
                    .font_weight(FontWeight::BOLD);
                if level == 2 {
                    style.text.font_size = Some(px(26.).into());
                }
                style
            }
        })
        .with_link(theme::color(ACCENT, cx).into())
        .with_border(theme::color(0x536861, cx).into())
        .with_table(table)
        .with_table_head(table_head)
        .with_table_cell(table_cell)
        .with_selection(theme::color(ACCENT, cx).opacity(0.22).into())
        .with_code_background(theme::color(CODE_BG, cx).into())
        .with_inline_code(gpui_kit::HighlightStyle {
            color: Some(theme::color(ACCENT, cx).into()),
            background_color: Some(theme::color(CODE_BG, cx).into()),
            ..Default::default()
        })
        .with_code_block(code_block)
}

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-,", Settings, Some("LeetDesk")),
        KeyBinding::new("ctrl-1", StudyMode, Some("LeetDesk")),
        KeyBinding::new("ctrl-2", CodingMode, Some("LeetDesk")),
        KeyBinding::new("ctrl-k", SearchProblems, Some("LeetDesk")),
        KeyBinding::new("ctrl-s", Save, Some("LeetDesk")),
        KeyBinding::new("alt-z", ToggleWrap, Some("LeetDesk")),
        KeyBinding::new("ctrl-enter", RunCases, Some("LeetDesk")),
        KeyBinding::new("ctrl-shift-enter", Submit, Some("LeetDesk")),
        KeyBinding::new(
            "ctrl-shift-z",
            gpui_kit::component::input::Redo,
            Some("LeetDesk > Input"),
        ),
        KeyBinding::new("ctrl-shift-f", Format, Some("LeetDesk")),
        KeyBinding::new("ctrl-shift-o", OrganizeImports, Some("LeetDesk")),
        KeyBinding::new("ctrl-i", ImportNotes, Some("LeetDesk")),
        KeyBinding::new(
            "ctrl-space",
            gpui_kit::component::input::ShowCompletions,
            Some("Input && mode == editor"),
        ),
        KeyBinding::new(
            "f12",
            gpui_kit::component::input::GoToDefinition,
            Some("Input && mode == editor"),
        ),
    ]);
    #[cfg(target_os = "macos")]
    cx.bind_keys([
        KeyBinding::new("cmd-,", Settings, Some("LeetDesk")),
        KeyBinding::new("cmd-1", StudyMode, Some("LeetDesk")),
        KeyBinding::new("cmd-2", CodingMode, Some("LeetDesk")),
        KeyBinding::new("cmd-k", SearchProblems, Some("LeetDesk")),
        KeyBinding::new("cmd-s", Save, Some("LeetDesk")),
        KeyBinding::new("cmd-enter", RunCases, Some("LeetDesk")),
        KeyBinding::new("cmd-shift-enter", Submit, Some("LeetDesk")),
        KeyBinding::new("cmd-shift-f", Format, Some("LeetDesk")),
        KeyBinding::new("cmd-shift-o", OrganizeImports, Some("LeetDesk")),
        KeyBinding::new("cmd-i", ImportNotes, Some("LeetDesk")),
        KeyBinding::new(
            "cmd-j",
            gpui_kit::component::input::ShowCompletions,
            Some("Input && mode == editor"),
        ),
    ]);
}

#[cfg(test)]
mod ui_tests {
    use super::*;
    use gpui_kit::TestAppContext;
    use gpui_kit::test::TestWindowExt;

    #[gpui_kit::test]
    fn learning_tree_opens_bilingual_guides_filters_by_approach_and_keeps_drafts(
        cx: &mut TestAppContext,
    ) {
        let (handle, view, temp) = practice_window(cx);
        let source = "package solution\n// 学习图论时保留的草稿";
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |workspace, cx| {
                let mut problems = online::parse_page(online::Site::China, &json!({"data":{"problemsetQuestionList":{"total":4,"questions":[
                    {"frontendQuestionId":"1","titleSlug":"two-sum","title":"两数之和","difficulty":"Easy"},
                    {"frontendQuestionId":"200","titleSlug":"number-of-islands","title":"岛屿数量","difficulty":"Medium"},
                    {"frontendQuestionId":"994","titleSlug":"rotting-oranges","title":"腐烂的橘子","difficulty":"Medium","status":"ac"},
                    {"frontendQuestionId":"207","titleSlug":"course-schedule","title":"课程表","difficulty":"Medium"}
                ]}}})).unwrap().problems;
                for problem in &mut problems {
                    problem.online.as_mut().unwrap().loaded = true;
                    problem.online.as_mut().unwrap().go_supported = true;
                    problem.starter = "package solution\nfunc answer() int { return 0 }".into();
                }
                workspace.problems = problems;
                workspace.learning_navigation = true;
                workspace.catalog_view.replace(&workspace.problems);
                workspace.editor.update(cx, |state, cx| { state.set_value(source, window, cx); state.set_selected_range(8..16, cx); });
                workspace.document_view = None;
            });
            window.render_frame(cx);
            assert!(view.read(cx).show_categories);
            reveal_navigation("chapter-graph",window,cx);
            window.click("chapter-graph", cx);
            window.render_frame(cx);
            assert!(view.read(cx).document_tab == DocumentTab::Explanation);
            assert_eq!(view.read(cx).filter, "@chapter:graph");
            assert_eq!(view.read(cx).filtered_indices().as_ref(), &[1,2,3]);
        }).unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let text = view
                .read(cx)
                .document_view
                .as_ref()
                .unwrap()
                .1
                .read(cx)
                .rendered_text();
            assert!(text.as_str().contains("图与状态搜索"));
            assert!(text.as_str().contains("复杂度与前提"));
            assert!(text.as_str().contains("易错点"));
            window.click("pattern-bfs-distance", cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).filtered_indices().as_ref(), &[2]);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let text = view
                .read(cx)
                .document_view
                .as_ref()
                .unwrap()
                .1
                .read(cx)
                .rendered_text();
            assert!(text.as_str().contains("BFS 最少步数"));
            assert!(text.as_str().contains("994. 腐烂的橘子"));
            assert_eq!(view.read(cx).editor.read(cx).value().as_ref(), source);
            assert_eq!(
                view.read(cx).editor.read(cx).selected_value().as_ref(),
                "solution"
            );
            window.click(("study-plan", 1_usize), cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).filtered_indices().as_ref(), &[2]);
            window.click(("status-filter", 1_usize), cx);
            window.render_frame(cx);
            assert!(view.read(cx).filtered_indices().is_empty());
            window.click(("status-filter", 0_usize), cx);
            view.update(cx, |workspace, cx| {
                workspace.set_interface_language(InterfaceLanguage::English, window, cx)
            });
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let text = view
                .read(cx)
                .document_view
                .as_ref()
                .unwrap()
                .1
                .read(cx)
                .rendered_text();
            assert!(text.as_str().contains("BFS distance"));
            assert!(text.as_str().contains("Cost and assumptions"));
            window.click("chapter-toggle-graph", cx);
            window.render_frame(cx);
            assert!(window.try_find("pattern-bfs-distance").is_none());
            assert_eq!(view.read(cx).filter, "@pattern:bfs-distance");
            window.click("navigation-topics", cx);
            view.update(cx, |workspace, _| {
                workspace.filter = "@pattern:bfs-distance".into()
            });
            window.render_frame(cx);
            window.click(("problem", 994_usize), cx);
            window.render_frame(cx);
            assert!(view.read(cx).document_tab == DocumentTab::Explanation);
            assert_eq!(view.read(cx).problem().id, 994);
            assert_eq!(
                Store::open(temp.path().join("data"))
                    .unwrap()
                    .source(1)
                    .unwrap(),
                source
            );
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let text = view
                .read(cx)
                .document_view
                .as_ref()
                .unwrap()
                .1
                .read(cx)
                .rendered_text();
            assert!(text.as_str().contains("Graphs and state-space search"));
            assert!(text.as_str().contains("Seed BFS with all rotten oranges"));
            assert!(text.as_str().contains("Applying the general method"));
            assert!(text.as_str().contains("Why this is correct"));
            assert!(text.as_str().contains("Go reference solution"));
            assert!(text.as_str().contains("func orangesRotting"));
            assert!(text.as_str().contains("Step-by-step example"));
            assert!(text.as_str().contains("fresh=0"));
            assert!(text.as_str().contains("Problem-specific pitfall"));
            assert!(!text.as_str().contains("Look up target minus"));
        })
        .unwrap();
    }

    fn reveal_setting(id: &str, window: &mut Window, cx: &mut App) {
        for _ in 0..12 {
            window.render_frame(cx);
            let viewport = window.find("settings-scroll").bounds();
            let target = window.find(id.to_owned());
            if target.visible() && viewport.contains(&target.bounds().center()) {
                return;
            }
            let delta = if target.bounds().center().y < viewport.origin.y {
                160.
            } else {
                -160.
            };
            window.scroll(
                "settings-scroll",
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(delta))),
                cx,
            );
        }
        panic!("Setting {id} could not be scrolled into view");
    }

    fn reveal_navigation(id: &str, window: &mut Window, cx: &mut App) {
        for _ in 0..15 {
            window.render_frame(cx);
            let viewport = window.find("category-scroll").bounds();
            let target = window.find(id.to_owned());
            if target.visible() && viewport.contains(&target.bounds().center()) {
                return;
            }
            let delta = if target.bounds().center().y < viewport.origin.y {
                120.
            } else {
                -120.
            };
            window.scroll(
                "category-scroll",
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(delta))),
                cx,
            );
        }
        panic!("Navigation {id} was not reachable");
    }

    #[gpui_kit::test]
    fn markdown_import_picker_selects_files_and_folder_picker_selects_folders(
        cx: &mut TestAppContext,
    ) {
        let (handle, view, temp) = practice_window(cx);
        let file = temp.path().join("method.md");
        fs::write(&file, "# 双指针做法\n\n从两侧收缩。\n").unwrap();

        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("settings", cx);
            window.render_frame(cx);
            reveal_setting("import-md", window, cx);
            window.click("import-md", cx);
        })
        .unwrap();
        cx.simulate_path_prompt_response({
            let file = file.clone();
            move |options| {
                assert!(options.files && !options.directories && options.multiple);
                Some(vec![file])
            }
        });
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let workspace = view.read(cx);
            assert!(
                workspace
                    .explanations
                    .iter()
                    .any(|(_, body)| body.contains("双指针做法"))
            );
            assert!(workspace.document_tab == DocumentTab::Explanation);
            reveal_setting("import-md-folder", window, cx);
            window.click("import-md-folder", cx);
        })
        .unwrap();
        cx.simulate_path_prompt_response(move |options| {
            assert!(!options.files && options.directories && options.multiple);
            None
        });
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.click("close-settings", cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let workspace = view.read(cx);
            let (_, state, origin) = workspace.document_view.as_ref().unwrap();
            let text = state.read(cx).rendered_text();
            assert!(text.as_str().contains("哈希与前缀统计"));
            assert!(text.as_str().contains("哈希表保存已扫描值的下标"));
            assert!(text.as_str().contains("双指针做法"));
            assert!(text.as_str().contains("从两侧收缩"));
            assert_eq!(
                origin.as_ref().unwrap().canonicalize().unwrap(),
                temp.path().canonicalize().unwrap()
            );
            assert_eq!(
                fs::read_to_string(&file).unwrap(),
                "# 双指针做法\n\n从两侧收缩。\n"
            );
            assert!(window.try_find(("document-tab", 3_usize)).is_none());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn notes_toolbar_exports_and_imports_a_portable_bundle(cx: &mut TestAppContext) {
        let (handle, view, temp) = practice_window(cx);
        let bundle = temp.path().join("take-home.zip");
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(("document-tab", 2_usize), cx);
            view.read(cx).note_editor.clone().update(cx, |state, cx| {
                state.set_value("# 带回家的笔记", window, cx)
            });
            window.render_frame(cx);
            window.click("export-note-bundle", cx);
        })
        .unwrap();
        cx.simulate_new_path_selection({
            let bundle = bundle.clone();
            move |directory| {
                assert!(directory.is_dir());
                Some(bundle)
            }
        });
        cx.run_until_parked();
        assert!(bundle.is_file());
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(view.read(cx).notice.contains("已导出 1 篇个人笔记"));
            window.click("import-note-bundle", cx);
        })
        .unwrap();
        cx.simulate_path_prompt_response({
            let bundle = bundle.clone();
            move |options| {
                assert!(options.files && !options.directories && !options.multiple);
                Some(vec![bundle])
            }
        });
        cx.run_until_parked();
        cx.update_window(handle, |_, _, cx| {
            assert!(view.read(cx).notice.contains("已导入 0 篇个人笔记"));
            assert_eq!(view.read(cx).saved_note, "# 带回家的笔记");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn hot100_topic_status_and_difficulty_buttons_intersect_and_reset(cx: &mut TestAppContext) {
        let (handle, view, _temp) = practice_window(cx);
        let source = cx.update_window(handle, |_, window, cx| {
            view.update(cx, |workspace, _| {
                workspace.problems = online::parse_page(online::Site::China, &json!({"data":{"problemsetQuestionList":{"total":4,"questions":[
                    {"frontendQuestionId":"1","titleSlug":"two-sum","title":"两数之和","difficulty":"Easy","topicTags":[{"name":"Array"}]},
                    {"frontendQuestionId":"21","titleSlug":"merge-two-sorted-lists","title":"合并两个有序链表","difficulty":"Easy","topicTags":[{"name":"Linked List"}]},
                    {"frontendQuestionId":"203","titleSlug":"remove-linked-list-elements","title":"移除链表元素","difficulty":"Easy","topicTags":[{"name":"Linked List"}]},
                    {"frontendQuestionId":"206","titleSlug":"reverse-linked-list","title":"反转链表","difficulty":"MEDIUM","status":"ac","topicTags":[{"name":"Linked List"},{"name":"Two Pointers"}]}
                ]}}})).unwrap().problems;
                workspace.catalog_view.replace(&workspace.problems);
                workspace.document_view = None;
            });
            window.render_frame(cx);
            window.click("navigation-topics", cx);
            window.render_frame(cx);
            assert!(view.read(cx).show_categories);
            window.click(("study-plan", 1_usize), cx);
            window.render_frame(cx);
            window.click("topic-链表", cx);
            window.render_frame(cx);
            window.click(("status-filter", 1_usize), cx);
            window.render_frame(cx);
            window.click(("difficulty", 1_usize), cx);
            view.read(cx).editor.read(cx).value().to_string()
        }).unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let workspace = view.read(cx);
            assert_eq!(workspace.study_plan_filter, "热题 100");
            assert_eq!(workspace.filter, "链表");
            assert_eq!(workspace.status_filter, "未解答");
            assert_eq!(workspace.difficulty_filter, "简单");
            assert_eq!(workspace.filtered_indices().as_ref(), &[1]);
            assert_eq!(workspace.editor.read(cx).value().as_ref(), source);
            window.click(("study-plan", 0_usize), cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).filtered_indices().as_ref(), &[1, 2]);
            window.click("clear-filters", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(view.read(cx).filtered_indices().len(), 4);
            assert_eq!(view.read(cx).study_plan_filter, "全部题目");
            assert_eq!(view.read(cx).status_filter, "全部");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn html_paragraphs_select_copy_and_reflow_without_sharing_layouts(cx: &mut TestAppContext) {
        struct Statement {
            text: Entity<TextViewState>,
            width: f32,
        }
        impl Render for Statement {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                div()
                    .w(px(self.width))
                    .text_size(px(15.))
                    .child(gpui_kit::base::TextSelectionLayer)
                    .child(TextView::new(&self.text).style(document_style(cx)))
            }
        }
        cx.update(|cx| {
            gpui_kit::init(cx);
            apply_theme(cx);
        });
        let html = "<p>第一段 <code>head</code> 后面的长文字一二三四五六七八九。</p><p>第二段 <code>n</code> 短。</p><p>第三段 <code>node.Next</code> 仍应完整保留🙂。</p>";
        let (view, cx) = cx.add_window_view(|_, cx| Statement {
            text: cx.new(|cx| TextViewState::html(html, cx)),
            width: 640.,
        });
        cx.run_until_parked();
        let cx: &mut gpui_kit::VisualTestContext = cx;
        for width in [640., 180., 420.] {
            view.update(cx, |view, cx| {
                view.width = width;
                cx.notify();
            });
            cx.update(|window, cx| window.render_frame(cx));
            let bounds = cx.update(|_, cx| view.read(cx).text.read(cx).bounds());
            let start = bounds.origin + gpui_kit::point(px(2.), px(8.));
            let end = gpui_kit::point(bounds.right() - px(2.), bounds.bottom() - px(2.));
            cx.simulate_mouse_down(
                start,
                gpui_kit::MouseButton::Left,
                gpui_kit::Modifiers::default(),
            );
            cx.update(|window, cx| window.render_frame(cx));
            cx.simulate_mouse_move(
                end,
                Some(gpui_kit::MouseButton::Left),
                gpui_kit::Modifiers::default(),
            );
            cx.update(|window, cx| window.render_frame(cx));
            cx.simulate_mouse_up(
                end,
                gpui_kit::MouseButton::Left,
                gpui_kit::Modifiers::default(),
            );
            cx.update(|window, cx| {
                window.render_frame(cx);
                let selected = view.read(cx).text.read(cx).selected_text();
                for part in [
                    "第一段 head",
                    "一二三四五六七八九",
                    "第二段 n 短",
                    "第三段 node.Next",
                    "完整保留🙂",
                ] {
                    assert!(
                        selected.contains(part),
                        "width={width}, bounds={bounds:?}, selected={selected:?}"
                    );
                }
                window.press("ctrl-c", cx);
                let clipboard = cx
                    .read_from_clipboard()
                    .and_then(|item| item.text())
                    .unwrap();
                assert!(clipboard.contains("第二段 n 短"), "{clipboard:?}");
                assert!(clipboard.contains("第三段 node.Next"), "{clipboard:?}");
            });
        }
    }

    #[gpui_kit::test]
    fn html_examples_keep_line_breaks_entities_and_all_labels_on_reflow(cx: &mut TestAppContext) {
        struct Examples {
            state: Entity<TextViewState>,
            width: f32,
        }
        impl Render for Examples {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                div()
                    .w(px(self.width))
                    .text_size(px(15.))
                    .text_color(rgb(INK))
                    .bg(rgb(PANEL))
                    .child(TextView::new(&self.state).style(document_style(cx)))
            }
        }
        cx.update(|cx| {
            gpui_kit::init(cx);
            apply_theme(cx);
        });
        let html = "<p>两两交换<code>head</code>中的节点。</p><pre><strong>输入：</strong>head = [1,2]\n<strong>输出：</strong>[2,1]\n<strong>解释：</strong>交换节点</pre><ul><li><code>0 &lt;= Node.val &lt;= 100</code></li></ul>";
        let handle = cx.update(|cx| {
            cx.open_window(WindowOptions::default(), |_, cx| {
                cx.new(|cx| Examples {
                    state: cx.new(|cx| TextViewState::html(html, cx)),
                    width: 440.,
                })
            })
            .unwrap()
        });
        cx.run_until_parked();
        for width in [440., 200., 320.] {
            cx.update_window(handle.into(), |root, window, cx| {
                let root = root.downcast::<Examples>().unwrap();
                root.update(cx, |root, cx| {
                    root.width = width;
                    cx.notify();
                });
                window.render_frame(cx);
                let text = root.read(cx).state.read(cx).rendered_text();
                assert!(
                    text.as_str()
                        .contains("输入：head = [1,2]\n输出：[2,1]\n解释：交换节点"),
                    "{text:?}"
                );
                assert!(text.as_str().contains("0 <= Node.val <= 100"));
                let code: gpui_kit::Background = rgb(CODE_BG).into();
                assert!(
                    window
                        .painted_quads()
                        .iter()
                        .any(|q| q.background == code && q.bounds.size.height.0 > 60.),
                    "示例保留至少三行的高度"
                );
            })
            .unwrap();
        }
    }

    #[gpui_kit::test]
    fn inline_code_backgrounds_cannot_cover_adjacent_chinese_text(cx: &mut TestAppContext) {
        const TEXT_MARKER: u32 = 0x123abc;
        struct Statement {
            state: Entity<TextViewState>,
        }
        impl Render for Statement {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                div()
                    .w(px(200.))
                    .font_family(UI_FONT)
                    .text_size(px(15.))
                    .text_bg(rgb(TEXT_MARKER))
                    .child(TextView::new(&self.state).style(document_style(cx)))
            }
        }
        cx.update(|cx| {
            gpui_kit::init(cx);
            apply_theme(cx);
        });
        let handle = cx.update(|cx| cx.open_window(WindowOptions::default(), |_, cx| {
            cx.new(|cx| Statement { state: cx.new(|cx| TextViewState::html("<p>给你链表<code>head</code>和值<code>Node.val</code>，请保留所有文字。</p>", cx)) })
        }).unwrap());
        cx.run_until_parked();
        cx.update_window(handle.into(), |root, window, cx| {
            window.render_frame(cx);
            let root = root.downcast::<Statement>().unwrap();
            let text = root.read(cx).state.read(cx).rendered_text();
            for part in ["给你链表", "head", "Node.val", "请保留所有文字"] {
                assert!(text.as_str().contains(part));
            }
            let quads = window.painted_quads();
            let marker: gpui_kit::Background = rgb(TEXT_MARKER).into();
            let code: gpui_kit::Background = rgb(CODE_BG).into();
            let first_text = quads
                .iter()
                .position(|q| q.background == marker)
                .expect("实际文字背景已绘制");
            let last_code = quads
                .iter()
                .rposition(|q| q.background == code)
                .expect("行内代码背景已绘制");
            assert!(
                last_code < first_text,
                "背景必须先于所有文字绘制，避免遮盖相邻字符"
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn markdown_tables_have_distinct_header_and_body_surfaces(cx: &mut TestAppContext) {
        struct TablePreview {
            state: Entity<TextViewState>,
        }
        impl Render for TablePreview {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                div()
                    .w(px(340.))
                    .child(TextView::new(&self.state).style(document_style(cx)))
            }
        }
        cx.update(|cx| {
            gpui_kit::init(cx);
            apply_theme(cx);
        });
        let source =
            "| 思路 | 复杂度 |\n| --- | --- |\n| 哈希表查找与遍历 | O(n) |\n| 双重循环 | O(n²) |";
        let handle = cx.update(|cx| {
            cx.open_window(WindowOptions::default(), |_, cx| {
                cx.new(|cx| TablePreview {
                    state: cx.new(|cx| TextViewState::markdown(source, cx)),
                })
            })
            .unwrap()
        });
        cx.run_until_parked();
        cx.update_window(handle.into(), |root, window, cx| {
            window.render_frame(cx);
            let root = root.downcast::<TablePreview>().unwrap();
            let text = root.read(cx).state.read(cx).rendered_text();
            assert!(text.as_str().contains("哈希表查找与遍历"));
            let quads = window.painted_quads();
            let header: gpui_kit::Background = rgb(0x203c34).into();
            let body: gpui_kit::Background = rgb(0x101715).into();
            assert!(
                quads.iter().any(|quad| quad.background == header),
                "表头背景应可见"
            );
            assert!(
                quads.iter().any(|quad| quad.background == body),
                "表身背景应可见"
            );
            assert_eq!(
                document_style(cx).table().overflow.x,
                Some(gpui_kit::Overflow::Scroll)
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn source_edits_reuse_statement_and_sidebar_snapshots(cx: &mut TestAppContext) {
        let (handle, view, _temp) = practice_window(cx);
        let (state_id, indices) = cx
            .update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                let workspace = view.read(cx);
                (
                    workspace.document_view.as_ref().unwrap().1.entity_id(),
                    workspace.filtered_indices(),
                )
            })
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            view.read(cx)
                .editor
                .clone()
                .update(cx, |state, cx| state.focus(window, cx));
            window.input("\n// 修改代码", cx);
            window.render_frame(cx);
            assert_eq!(
                view.read(cx).document_view.as_ref().unwrap().1.entity_id(),
                state_id
            );
            assert!(Rc::ptr_eq(&indices, &view.read(cx).filtered_indices()));
            window.click("favorite", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            view.update(cx, |workspace, _| workspace.filter = "收藏".into());
            assert_eq!(
                view.read(cx).filtered_indices().as_ref(),
                &[view.read(cx).current]
            );
            view.update(cx, |workspace, _| workspace.filter = "全部题目".into());
            let next = if view.read(cx).current == 0 { 1 } else { 0 };
            view.update(cx, |workspace, cx| {
                workspace.select_problem(next, window, cx)
            });
            window.render_frame(cx);
            assert_ne!(
                view.read(cx).document_view.as_ref().unwrap().1.entity_id(),
                state_id
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn markdown_notes_save_render_autosave_and_restore_per_problem(cx: &mut TestAppContext) {
        let (handle, view, _temp) = practice_window(cx);
        let markdown = "# 中文思路🙂\n\n使用 **哈希表**，复杂度 $O(n)$。\n\n$$\n\\sum_{i=1}^{n} i\n$$\n\n- 边界：空数组\n\n```go\n\treturn 42\n```\n";
        let path = cx
            .update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.click(("document-tab", 2_usize), cx);
                view.read(cx)
                    .note_editor
                    .clone()
                    .update(cx, |state, cx| state.focus(window, cx));
                cx.write_to_clipboard(ClipboardItem::new_string(markdown.into()));
                window.press("ctrl-v", cx);
                assert_eq!(
                    view.read(cx).note_editor.read(cx).value().as_ref(),
                    markdown
                );
                assert!(view.read(cx).saved_note.is_empty());
                let workspace = view.read(cx);
                workspace
                    .store
                    .workspace(workspace.problem().id)
                    .join("notes.md")
            })
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert!(view.read(cx).note_dirty);
            window.click("save-notes", cx);
            assert!(!view.read(cx).note_dirty);
            assert_eq!(view.read(cx).saved_note, markdown);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let workspace = view.read(cx);
            let rendered = workspace.note_preview.read(cx).rendered_text();
            assert!(rendered.as_str().contains("中文思路🙂"));
            assert!(rendered.as_str().contains("哈希表"));
            assert!(rendered.as_str().contains("return 42"));
            assert!(!rendered.as_str().contains("# 中文思路"));
            assert!(!rendered.as_str().contains("**哈希表**"));
            assert_eq!(fs::read_to_string(&path).unwrap(), markdown);
            window.click("notes-preview", cx);
            assert!(view.read(cx).notes_view == NotesView::Preview);
            window.click("notes-edit", cx);
            view.read(cx).note_editor.clone().update(cx, |state, cx| {
                state.set_cursor_position(
                    language::byte_position(markdown, markdown.len()),
                    window,
                    cx,
                );
                state.focus(window, cx);
            });
            window.input("\n补充复习", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert!(view.read(cx).saved_note.ends_with("补充复习"));
            assert!(
                view.read(cx)
                    .note_preview
                    .read(cx)
                    .rendered_text()
                    .as_str()
                    .contains("补充复习")
            );
            assert!(fs::read_to_string(&path).unwrap().ends_with("补充复习"));
            window.click("notes-split", cx);
            view.update(cx, |workspace, cx| workspace.select_problem(1, window, cx));
            assert!(view.read(cx).saved_note.is_empty());
            view.update(cx, |workspace, cx| workspace.select_problem(0, window, cx));
            assert!(
                view.read(cx)
                    .note_editor
                    .read(cx)
                    .value()
                    .contains("补充复习")
            );
            view.update(cx, |workspace, cx| workspace.flush(cx).unwrap());
            let root = view.read(cx).store.root.clone();
            let restored =
                cx.new(|cx| Workspace::new(Store::open(root).unwrap(), window, cx).unwrap());
            assert!(restored.read(cx).saved_note.contains("补充复习"));
            assert_eq!(
                restored.read(cx).note_editor.read(cx).value().as_ref(),
                fs::read_to_string(&path).unwrap()
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn failed_markdown_save_keeps_draft_and_last_rendered_version(cx: &mut TestAppContext) {
        let (handle, view, _temp) = practice_window(cx);
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |workspace, cx| {
                workspace.document_tab = DocumentTab::Notes;
                workspace
                    .note_editor
                    .update(cx, |state, cx| state.set_value("# 已保存", window, cx));
                workspace.save_notes_document(cx).unwrap();
            });
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            let path = view
                .read(cx)
                .store
                .workspace(view.read(cx).problem().id)
                .join("notes.md");
            fs::remove_file(&path).unwrap();
            fs::create_dir(&path).unwrap();
            view.read(cx)
                .note_editor
                .clone()
                .update(cx, |state, cx| state.focus(window, cx));
            window.render_frame(cx);
            window.press("ctrl-a", cx);
            window.input("新草稿", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            let path = view
                .read(cx)
                .store
                .workspace(view.read(cx).problem().id)
                .join("notes.md");
            window.click("save-notes", cx);
            assert!(view.read(cx).notice.contains("笔记保存失败"));
            assert!(view.read(cx).note_dirty);
            assert_eq!(view.read(cx).saved_note, "# 已保存");
            assert_eq!(
                view.read(cx).note_editor.read(cx).value().as_ref(),
                "新草稿"
            );
            assert!(
                view.read(cx)
                    .note_preview
                    .read(cx)
                    .rendered_text()
                    .as_str()
                    .contains("已保存")
            );
            fs::remove_dir(&path).unwrap();
            window.click("save-notes", cx);
            assert_eq!(fs::read_to_string(path).unwrap(), "新草稿");
            assert!(!view.read(cx).note_dirty);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn go_hard_tabs_have_visible_indent_with_and_without_type_hints(cx: &mut TestAppContext) {
        let (handle, view, _temp) = practice_window(cx);
        let source = "value\n\tvalue\n\t\tvalue\n    value\n";
        cx.update_window(handle, |_, window, cx| {
            let editor = view.read(cx).editor.clone();
            editor.update(cx, |state, cx| state.set_value(source, window, cx));
            for hints in [vec![], vec![(12, ": int".into())]] {
                editor.update(cx, |state, cx| state.set_inlay_hints(hints, cx));
                window.render_frame(cx);
                let x = |offset| {
                    editor
                        .read(cx)
                        .range_to_bounds(&(offset..offset))
                        .unwrap()
                        .origin
                        .x
                };
                let plain = x(0);
                let tab = x(7);
                let nested = x(15);
                let spaces = x(25);
                assert!(
                    tab > plain + px(10.),
                    "Tab 必须有可见缩进：{plain:?}, {tab:?}"
                );
                assert!(
                    (tab - spaces).abs() < px(1.),
                    "Go 一个 Tab 应显示为四个空格的宽度"
                );
                assert!((nested - plain - (tab - plain) * 2.).abs() < px(1.));
                assert_eq!(editor.read(cx).value().as_ref(), source);
            }
            let wrapped = format!("\t// {}", "long ".repeat(70));
            editor.update(cx, |state, cx| state.set_value(wrapped.clone(), window, cx));
            window.render_frame(cx);
            let first = editor.read(cx).range_to_bounds(&(1..1)).unwrap();
            let end = wrapped.len();
            let last = editor.read(cx).range_to_bounds(&(end..end)).unwrap();
            assert!(last.origin.y > first.origin.y);
            assert!(
                last.origin.x >= first.origin.x - px(1.),
                "折行续行应保留 Tab 缩进"
            );
            click_position(last.origin + gpui_kit::point(px(1.), px(4.)), window, cx);
            assert_eq!(editor.read(cx).cursor(), end);
            window.press("ctrl-a", cx);
            window.press("ctrl-c", cx);
            assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), wrapped);
            editor.update(cx, |state, cx| state.set_value("value\n\t\t", window, cx));
            window.render_frame(cx);
            let base = editor.read(cx).range_to_bounds(&(0..0)).unwrap().origin.x;
            let trailing = editor.read(cx).range_to_bounds(&(8..8)).unwrap().origin.x;
            assert!(trailing > base + px(60.), "空白行末的 Tab 也要有宽度");
        })
        .unwrap();
    }

    fn click_position(
        position: gpui_kit::Point<gpui_kit::Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.simulate_mouse_move(position, cx);
        window.dispatch_event(
            gpui_kit::PlatformInput::MouseDown(gpui_kit::MouseDownEvent {
                button: gpui_kit::MouseButton::Left,
                position,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }),
            cx,
        );
        window.dispatch_event(
            gpui_kit::PlatformInput::MouseUp(gpui_kit::MouseUpEvent {
                button: gpui_kit::MouseButton::Left,
                position,
                modifiers: Default::default(),
                click_count: 1,
            }),
            cx,
        );
    }

    #[gpui_kit::test]
    fn mouse_focus_tab_block_indent_and_outdent_work(cx: &mut TestAppContext) {
        let (handle, view, _temp) = practice_window(cx);
        cx.update_window(handle, |_, window, cx| {
            let editor = view.read(cx).editor.clone();
            editor.update(cx, |state, cx| state.set_value("one\ntwo\n", window, cx));
            view.read(cx)
                .search
                .clone()
                .update(cx, |state, cx| state.focus(window, cx));
            window.render_frame(cx);
            let bounds = editor.read(cx).range_to_bounds(&(0..1)).unwrap();
            click_position(bounds.origin + gpui_kit::point(px(0.), px(4.)), window, cx);
            assert!(editor.read(cx).focus_handle(cx).is_focused(window));
            window.press("tab", cx);
            assert_eq!(editor.read(cx).value().as_ref(), "\tone\ntwo\n");
            window.press("ctrl-z", cx);
            window.press("ctrl-a", cx);
            window.press("tab", cx);
            assert_eq!(editor.read(cx).value().as_ref(), "\tone\n\ttwo\n");
            window.press("shift-tab", cx);
            assert_eq!(editor.read(cx).value().as_ref(), "one\ntwo\n");
            window.press("ctrl-z", cx);
            assert_eq!(editor.read(cx).value().as_ref(), "\tone\n\ttwo\n");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn go_type_hints_reserve_space_and_keep_source_cursor_and_clipboard(cx: &mut TestAppContext) {
        let (handle, view, _temp) = practice_window(cx);
        let source = "package solution\nfunc next(head *ListNode) *ListNode {\n\tmessage := \"中文🙂\"; count := len(message)\n\tnode := head\n\tfor index, value := range []int{count} { _ = index; _ = value }\n\treturn node\n}\n";
        let anchor = source.find("message :=").unwrap() + "message".len();
        let before = cx
            .update_window(handle, |_, window, cx| {
                view.update(cx, |workspace, cx| {
                    workspace
                        .editor
                        .update(cx, |state, cx| state.set_value(source, window, cx));
                    workspace
                        .search
                        .update(cx, |state, cx| state.focus(window, cx));
                    workspace.connect_language(window, cx);
                });
                window.render_frame(cx);
                view.read(cx)
                    .editor
                    .read(cx)
                    .range_to_bounds(&(anchor..anchor))
                    .unwrap()
                    .origin
            })
            .unwrap();
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let editor = view.read(cx).editor.clone();
            let state = editor.read(cx);
            assert_eq!(state.value().as_ref(), source);
            for (variable, kind) in [
                ("message", ": string"),
                ("count", ": int"),
                ("node", ": *ListNode"),
                ("index", ": int"),
                ("value", ": int"),
            ] {
                let offset = source.find(variable).unwrap() + variable.len();
                assert!(
                    state
                        .inlay_hints()
                        .iter()
                        .any(|(at, label)| *at == offset && label.as_ref() == kind),
                    "{variable}: {:?}",
                    state.inlay_hints()
                );
            }
            assert!(
                view.read(cx)
                    .search
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            );
            let after = state.range_to_bounds(&(anchor..anchor)).unwrap().origin;
            assert_eq!(before.y, after.y);
            assert!(
                after.x > before.x + px(10.),
                "类型提示必须为后续代码留出空间"
            );
            click_position(
                gpui_kit::point((before.x + after.x) / 2., after.y + px(4.)),
                window,
                cx,
            );
            assert_eq!(
                editor.read(cx).cursor(),
                anchor,
                "点击提示应映射到变量后的源码位置"
            );
            window.press("left", cx);
            assert_eq!(editor.read(cx).cursor(), anchor - 1, "光标不进入虚拟标签");
            window.press("ctrl-a", cx);
            window.press("ctrl-c", cx);
            assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), source);
            window.input("x", cx);
            assert!(
                editor.read(cx).inlay_hints().is_empty(),
                "修改后旧提示应立即消失"
            );
            window.press("ctrl-z", cx);
            assert_eq!(
                editor.read(cx).value().as_ref(),
                source,
                "提示不能进入撤销历史"
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn inlay_hints_wrap_with_unicode_text_and_map_mouse_to_source(cx: &mut TestAppContext) {
        let (handle, view, _temp) = practice_window(cx);
        let source = format!(
            "package solution\nfunc answer() string {{ value := \"{}\"; return value }}",
            "中文🙂 ".repeat(25)
        );
        let anchor = source.find("value :=").unwrap() + 5;
        cx.update_window(handle, |_, window, cx| {
            let editor = view.read(cx).editor.clone();
            editor.update(cx, |state, cx| {
                state.set_value(source.clone(), window, cx);
                state.set_inlay_hints(vec![(anchor, ": string".into())], cx);
            });
            window.render_frame(cx);
            let first = editor.read(cx).range_to_bounds(&(anchor..anchor)).unwrap();
            let end = source.len();
            let last = editor.read(cx).range_to_bounds(&(end..end)).unwrap();
            assert!(
                last.origin.y > first.origin.y,
                "含提示的 Unicode 长行仍须折行"
            );
            click_position(last.origin + gpui_kit::point(px(1.), px(4.)), window, cx);
            assert_eq!(
                editor.read(cx).cursor(),
                end,
                "折行后的点击仍使用源码字节坐标"
            );
            window.press("left", cx);
            assert_eq!(editor.read(cx).cursor(), end - 1);
            window.press("alt-z", cx);
            window.render_frame(cx);
            let unwrapped = editor.read(cx).range_to_bounds(&(end..end)).unwrap();
            assert_eq!(unwrapped.origin.y, first.origin.y);
            assert_eq!(editor.read(cx).value().as_ref(), source);
            window.press("ctrl-a", cx);
            window.press("ctrl-c", cx);
            assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), source);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn typing_a_go_member_opens_completion_and_tab_inserts_it(cx: &mut TestAppContext) {
        let (handle, view, _temp) = practice_window(cx);
        let source = "package solution\n\nfunc next(head *ListNode) *ListNode { _ = \"中文🙂\"; return head. }";
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |workspace, cx| {
                workspace.editor.update(cx, |state, cx| {
                    state.set_value(source, window, cx);
                    state.set_cursor_position(
                        language::byte_position(source, source.find("head.").unwrap() + 5),
                        window,
                        cx,
                    );
                    state.focus(window, cx);
                });
                workspace.connect_language(window, cx);
            });
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert_eq!(view.read(cx).language_status, "gopls 已连接");
            window.render_frame(cx);
            window.input("N", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(
                view.read(cx).editor.read(cx).completion_menu_state().open,
                "输入 N 应自动弹出补全菜单"
            );
            window.press("tab", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, _, cx| {
            assert!(
                view.read(cx)
                    .editor
                    .read(cx)
                    .value()
                    .contains("return head.Next"),
                "自动补全应插入真实 gopls 的 Next 字段，当前代码：{}",
                view.read(cx).editor.read(cx).value()
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn go_callable_variable_and_append_complete_with_editable_arguments(cx: &mut TestAppContext) {
        let (handle, view, _temp) = practice_window(cx);
        for (source, prefix, typed, expected, placeholder, argument) in [
            (
                "package solution\nfunc solve() {\n var xxx func(int)\n xx\n}",
                "xx\n",
                "x",
                "xxx(int)",
                "int",
                "7",
            ),
            (
                "package solution\nfunc solve() {\n q := []int{}\n q = ap\n}",
                "ap\n",
                "p",
                "append(q, value)",
                "value",
                "42",
            ),
        ] {
            let offset = source.find(prefix).unwrap() + 2;
            cx.update_window(handle, |_, window, cx| {
                view.update(cx, |workspace, cx| {
                    workspace.editor.update(cx, |state, cx| {
                        state.set_value(source, window, cx);
                        state.set_cursor_position(
                            language::byte_position(source, offset),
                            window,
                            cx,
                        );
                        state.focus(window, cx);
                    });
                    workspace.connect_language(window, cx);
                });
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                assert_eq!(view.read(cx).language_status, "gopls 已连接");
                window.render_frame(cx);
                window.input(typed, cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                let editor = view.read(cx).editor.clone();
                window.render_frame(cx);
                assert!(editor.read(cx).completion_menu_state().open);
                if expected.starts_with("xxx") {
                    assert!(
                        editor.read(cx).completion_menu_state().items[0]
                            .label
                            .starts_with("xxx("),
                        "{:?}",
                        editor.read(cx).completion_menu_state().items
                    );
                }
                window.press("tab", cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                let editor = view.read(cx).editor.clone();
                assert!(
                    editor.read(cx).value().contains(expected),
                    "{}",
                    editor.read(cx).value()
                );
                assert_eq!(editor.read(cx).selected_value().as_ref(), placeholder);
                window.input(argument, cx);
                window.press("tab", cx);
                let text = editor.read(cx).value().to_string();
                let call = expected.replace(placeholder, argument);
                assert!(text.contains(&call), "{text}");
                assert_eq!(
                    editor.read(cx).cursor(),
                    text.find(&call).unwrap() + call.len()
                );
            })
            .unwrap();
        }
    }

    #[gpui_kit::test]
    fn go_snippet_replaces_prefix_and_tab_tracks_edited_fields(cx: &mut TestAppContext) {
        let (handle, view, _temp) = practice_window(cx);
        let source = "package solution\n\nfunc f() {\n\tfo\n}";
        let offset = source.find("\tfo").unwrap() + 3;
        cx.update_window(handle, |_, window, cx| {
            let editor = view.read(cx).editor.clone();
            editor.update(cx, |state, cx| {
                state.set_value(source, window, cx);
                state.set_cursor_position(language::byte_position(source, offset), window, cx);
                let item = snippets::completions(source, offset)
                    .into_iter()
                    .find(|item| item.label.starts_with("for ·"))
                    .unwrap();
                state.insert_completion(&item, offset - 2..offset, window, cx);
            });
            window.render_frame(cx);
            assert_eq!(editor.read(cx).selected_value().as_ref(), "n");
            window.input("size", cx);
            assert_eq!(editor.read(cx).selected_value().as_ref(), "");
            window.press("tab", cx);
            let at_body = editor.read(cx).cursor();
            let body = editor.read(cx).value().to_string();
            assert!(body.contains("for i := 0; i < size; i++ {\n\t\t\n\t}"));
            assert_eq!(body.as_bytes()[at_body - 1], b'\t');
            window.input("sum += i", cx);
            window.press("shift-tab", cx);
            assert_eq!(editor.read(cx).selected_value().as_ref(), "size");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn close_button_saves_current_solution(cx: &mut TestAppContext) {
        let (handle, view, temp) = practice_window(cx);
        let source = "package solution\n\nfunc answer() int { return 42 }\n";
        let id = cx.update(|cx| view.read(cx).problem().id);
        cx.update_window(handle, |_, window, cx| {
            view.read(cx).editor.clone().update(cx, |state, cx| {
                state.set_value(source, window, cx);
            });
            window.render_frame(cx);
            window.click("close-window", cx);
        })
        .unwrap();
        let saved = temp
            .path()
            .join("data")
            .join("workspaces")
            .join(id.to_string())
            .join("solution.go");
        assert_eq!(std::fs::read_to_string(saved).unwrap(), source);
    }

    #[gpui_kit::test]
    fn unicode_navigation_selection_and_clipboard_are_reversible(cx: &mut TestAppContext) {
        let (handle, view, _temp) = practice_window(cx);
        cx.update_window(handle, |_, window, cx| {
            let editor = view.read(cx).editor.clone();
            let text = "a中文🙂z";
            editor.update(cx, |state, cx| {
                state.set_value(text, window, cx);
                state.set_cursor_position(language::byte_position(text, 11), window, cx);
                state.focus(window, cx);
            });
            window.render_frame(cx);
            window.press("left", cx);
            assert_eq!(editor.read(cx).cursor(), 7);
            window.press("shift-left", cx);
            assert_eq!(editor.read(cx).selected_value().as_ref(), "文");
            window.press("backspace", cx);
            assert_eq!(editor.read(cx).value().as_ref(), "a中🙂z");
            window.press("ctrl-z", cx);
            assert_eq!(editor.read(cx).value().as_ref(), text);
            window.press("ctrl-a", cx);
            window.press("ctrl-c", cx);
            window.press("ctrl-x", cx);
            assert_eq!(editor.read(cx).value().as_ref(), "");
            window.press("ctrl-v", cx);
            assert_eq!(editor.read(cx).value().as_ref(), text);
            window.press("ctrl-z", cx);
            assert_eq!(editor.read(cx).value().as_ref(), "");
            window.press("ctrl-z", cx);
            assert_eq!(editor.read(cx).value().as_ref(), text);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn go_syntax_has_real_keyword_string_and_number_colors(cx: &mut TestAppContext) {
        use gpui_kit::component::highlighter::SyntaxHighlighter;
        cx.update(|cx| {
            gpui_kit::init(cx);
            apply_theme(cx);
            let source =
                "package solution\nfunc answer() string { n := 42; _ = n; return \"hello\" }";
            let mut highlighter = SyntaxHighlighter::new("go");
            assert!(highlighter.update(None, &Rope::from(source), None));
            let theme = &gpui_kit::component::Theme::global(cx).highlight_theme;
            let styles = highlighter.styles(&(0..source.len()), &**theme);
            let color = |offset: usize| {
                styles
                    .iter()
                    .find(|(range, _)| range.contains(&offset))
                    .and_then(|(_, style)| style.color)
            };
            let keyword = color(source.find("return").unwrap()).expect("Go 关键字应该着色");
            let string = color(source.find("hello").unwrap()).expect("Go 字符串应该着色");
            let number = color(source.find("42").unwrap()).expect("Go 数字应该着色");
            assert_ne!(keyword, string);
            assert_ne!(keyword, number);
            assert_eq!(keyword, rgb(0xff8f9c).into());
            assert_eq!(string, rgb(0xffe2e7).into());
            assert_eq!(number, rgb(0xffdfcf).into());
            assert_eq!(theme.style.editor_background, Some(rgb(BG).into()));
        });
    }

    #[gpui_kit::test]
    fn code_palette_has_readable_contrast_and_non_color_cues(cx: &mut TestAppContext) {
        fn luminance(color: gpui_kit::Hsla) -> f32 {
            let rgba = gpui_kit::Rgba::from(color);
            let linear = |value: f32| {
                if value <= 0.04045 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(rgba.r) + 0.7152 * linear(rgba.g) + 0.0722 * linear(rgba.b)
        }
        cx.update(|cx| {
            gpui_kit::init(cx);
            for appearance in Appearance::ALL {
                apply_appearance(appearance, cx);
                let theme = gpui_kit::component::Theme::global(cx);
                let syntax = &theme.highlight_theme.style;
                for name in [
                    "keyword",
                    "string",
                    "number",
                    "boolean",
                    "function",
                    "type",
                    "variable",
                    "property",
                    "constant",
                    "operator",
                    "comment",
                    "punctuation",
                ] {
                    let style = syntax.syntax.style(name).unwrap();
                    let foreground = luminance(style.color.unwrap());
                    for (background, minimum) in [
                        (syntax.editor_background.unwrap(), 7.0),
                        (
                            syntax.editor_active_line.unwrap(),
                            if appearance == Appearance::Dark {
                                7.0
                            } else {
                                4.5
                            },
                        ),
                        (theme.colors.selection, 4.5),
                    ] {
                        let background = luminance(background);
                        let contrast = (foreground.max(background) + 0.05)
                            / (foreground.min(background) + 0.05);
                        assert!(
                            contrast >= minimum,
                            "{appearance:?} {name} contrast {contrast} < {minimum}"
                        );
                    }
                }
                assert_eq!(
                    syntax.syntax.style("keyword").unwrap().font_weight,
                    Some(FontWeight::BOLD)
                );
                assert_eq!(
                    syntax.syntax.style("comment").unwrap().font_style,
                    Some(gpui_kit::FontStyle::Italic)
                );
            }
        });
    }

    #[gpui_kit::test]
    fn java_syntax_highlights_keywords_strings_and_numbers(cx: &mut TestAppContext) {
        use gpui_kit::component::highlighter::SyntaxHighlighter;
        cx.update(|cx| {
            gpui_kit::init(cx);
            apply_theme(cx);
            let source = "class Solution { int answer() { String value = \"hello\"; return 42; } }";
            let mut highlighter = SyntaxHighlighter::new("java");
            assert!(highlighter.update(None, &Rope::from(source), None));
            let theme = &gpui_kit::component::Theme::global(cx).highlight_theme;
            let styles = highlighter.styles(&(0..source.len()), &**theme);
            let color = |offset: usize| {
                styles
                    .iter()
                    .find(|(range, _)| range.contains(&offset))
                    .and_then(|(_, style)| style.color)
            };
            let keyword = color(source.find("return").unwrap()).expect("Java 关键字应该着色");
            let string = color(source.find("hello").unwrap()).expect("Java 字符串应该着色");
            let number = color(source.find("42").unwrap()).expect("Java 数字应该着色");
            assert_ne!(keyword, string);
            assert_ne!(keyword, number);
        });
    }

    #[gpui_kit::test]
    fn formatter_preserves_selection_scroll_and_atomic_undo(cx: &mut TestAppContext) {
        let (handle, view, _temp) = practice_window(cx);
        let mut source = String::from("package solution\n\nfunc example() {\n");
        for i in 0..100 {
            source.push_str(&format!("// 行 {i} 中文🙂\n"));
        }
        source.push_str("value:=1\n_ = value\n}\n");
        let formatted = runner::format_go(&source).unwrap();
        let selection = source.rfind("value").unwrap();
        let mut viewport = gpui_kit::point(px(0.), px(0.));
        cx.update_window(handle, |_, window, cx| {
            view.read(cx).editor.clone().update(cx, |state, cx| {
                state.set_value(source.clone(), window, cx);
                state.set_selected_range(selection..selection + 5, cx);
                state.focus(window, cx);
            });
            window.render_frame(cx);
            view.read(cx).editor.clone().update(cx, |state, cx| {
                state.set_scroll_offset(gpui_kit::point(px(0.), px(-500.)), cx)
            });
            window.render_frame(cx);
            viewport = view.read(cx).editor.read(cx).scroll_offset();
            window.press("ctrl-s", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let state = view.read(cx).editor.read(cx);
            assert_eq!(state.value().as_ref(), formatted);
            assert_eq!(state.selected_value().as_ref(), "value");
            assert_eq!(state.scroll_offset(), viewport, "格式化不能把视口滚回顶部");
            window.press("ctrl-z", cx);
            let state = view.read(cx).editor.read(cx);
            assert_eq!(state.value().as_ref(), source, "整个格式化必须一次撤销");
            assert_eq!(state.selected_range(), selection..selection + 5);
        })
        .unwrap();
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert_eq!(
                view.read(cx).editor.read(cx).value().as_ref(),
                source,
                "后台保存不能重做刚撤销的格式化"
            );
            window.press("ctrl-shift-z", cx);
            assert_eq!(view.read(cx).editor.read(cx).value().as_ref(), formatted);
            assert_eq!(
                view.read(cx).editor.read(cx).selected_value().as_ref(),
                "value"
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn saving_markdown_keeps_keyboard_focus_and_does_not_format_go(cx: &mut TestAppContext) {
        let (handle, view, _temp) = practice_window(cx);
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |workspace, cx| {
                workspace.document_tab = DocumentTab::Notes;
                workspace.editor.update(cx, |state, cx| {
                    state.set_value("package solution\nfunc answer( )int{return 1}", window, cx)
                });
                workspace.note_editor.update(cx, |state, cx| {
                    state.set_value("笔记", window, cx);
                    state.focus(window, cx);
                });
            });
            window.render_frame(cx);
            window.press("ctrl-s", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert_eq!(
                view.read(cx).editor.read(cx).value().as_ref(),
                "package solution\nfunc answer( )int{return 1}"
            );
            assert!(
                view.read(cx)
                    .note_editor
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            );
            window.input("继续", cx);
            assert!(view.read(cx).note_editor.read(cx).value().contains("继续"));
            assert!(!view.read(cx).editor.read(cx).value().contains("继续"));
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn typing_selection_deletion_pairing_and_multiple_cursors_undo_consistently(
        cx: &mut TestAppContext,
    ) {
        let (handle, view, _temp) = practice_window(cx);
        cx.update_window(handle, |_, window, cx| {
            let editor = view.read(cx).editor.clone();
            editor.update(cx, |state, cx| {
                state.set_value("", window, cx);
                state.focus(window, cx);
            });
            window.render_frame(cx);
            window.input("a", cx);
            window.input("b", cx);
            window.input("c", cx);
            window.press("ctrl-z", cx);
            assert_eq!(
                editor.read(cx).value().as_ref(),
                "",
                "连续输入应该合并成一个撤销事务"
            );
            window.press("ctrl-shift-z", cx);
            assert_eq!(editor.read(cx).value().as_ref(), "abc");
            window.press("shift-left", cx);
            window.press("backspace", cx);
            assert_eq!(editor.read(cx).value().as_ref(), "ab");
            window.press("ctrl-z", cx);
            assert_eq!(editor.read(cx).value().as_ref(), "abc");
            editor.update(cx, |state, cx| state.set_value("", window, cx));
            window.input("{", cx);
            assert_eq!(editor.read(cx).value().as_ref(), "{}");
            window.press("backspace", cx);
            assert_eq!(editor.read(cx).value().as_ref(), "");
            window.press("ctrl-z", cx);
            assert_eq!(editor.read(cx).value().as_ref(), "{}");
            editor.update(cx, |state, cx| {
                state.set_value("one one", window, cx);
                state.set_selected_range(0..3, cx);
                state.add_selection(4..7, cx);
            });
            window.render_frame(cx);
            window.input("two", cx);
            assert_eq!(editor.read(cx).value().as_ref(), "two two");
            window.press("ctrl-z", cx);
            assert_eq!(editor.read(cx).value().as_ref(), "one one");
            window.press("ctrl-shift-z", cx);
            assert_eq!(editor.read(cx).value().as_ref(), "two two");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn submit_emits_one_snapshot_and_only_platform_results_update_ac(cx: &mut TestAppContext) {
        let (handle, view, _temp) = practice_window(cx);
        let (sender, events) = std::sync::mpsc::channel();
        let subscription = cx.update(|cx| {
            cx.subscribe(&view, move |_, event: &WorkspaceEvent, _| {
                sender.send(event.clone()).unwrap();
            })
        });
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |workspace, cx| {
                let mut problem = online::parse_page(online::Site::China, &json!({"data":{"problemsetQuestionList":{"total":1,"questions":[{"frontendQuestionId":"1","titleSlug":"two-sum","title":"Two Sum"}]}}})).unwrap().problems.remove(0);
                let remote = problem.online.as_mut().unwrap();
                remote.loaded = true; remote.go_supported = true; remote.question_id = "1".into();
                let mut cache = online::OnlineCache {site: online::Site::China, username:"alice".into(), problems:vec![problem.clone()], synced_at:1};
                workspace.store.save_online_cache(&mut cache).unwrap();
                workspace.problems = vec![problem]; workspace.catalog_view.replace(&workspace.problems); workspace.document_view = None; workspace.current = 0; workspace.online_enabled = true;
                workspace.editor.update(cx, |state, cx| state.set_value("package solution\nfunc twoSum(nums []int, target int) []int { return nil }", window, cx));
            });
            window.render_frame(cx); window.click("submit", cx); window.click("submit", cx);
        }).unwrap();
        cx.run_until_parked();
        let emitted: Vec<_> = events.try_iter().collect();
        assert_eq!(emitted.len(), 1, "判题中不能重复提交");
        assert!(
            matches!(&emitted[0], WorkspaceEvent::Run {submit:true, source, ..} if source.contains("twoSum"))
        );
        let snapshots = cx.update(|cx| view.read(cx).store.workspace(1).join("submissions"));
        let saved = fs::read_dir(snapshots)
            .unwrap()
            .collect::<std::io::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(
            fs::read_to_string(saved[0].path()).unwrap(),
            match &emitted[0] {
                WorkspaceEvent::Run { source, .. } => source.as_str(),
                _ => unreachable!(),
            }
        );
        cx.update_window(handle, |_, _, cx| {
            view.update(cx, |workspace, cx| {
                assert_eq!(online::status_label(workspace.problem()), "未解答");
                assert!(workspace.record_submission(1, "bob", true, cx).is_err());
                assert_eq!(online::status_label(workspace.problem()), "未解答");
                assert_eq!(
                    workspace.record_submission(1, "alice", false, cx).unwrap(),
                    0
                );
                assert_eq!(online::status_label(workspace.problem()), "尝试过");
                assert_eq!(
                    workspace.record_submission(1, "alice", true, cx).unwrap(),
                    1
                );
                workspace.record_submission(1, "alice", false, cx).unwrap();
                assert_eq!(online::status_label(workspace.problem()), "已通过");
                assert_eq!(
                    online::status_label(
                        &workspace
                            .store
                            .online_cache(online::Site::China)
                            .unwrap()
                            .unwrap()
                            .problems[0]
                    ),
                    "已通过"
                );
            });
        })
        .unwrap();
        drop(subscription);
    }

    fn practice_window(
        cx: &mut TestAppContext,
    ) -> (
        gpui_kit::AnyWindowHandle,
        Entity<Workspace>,
        tempfile::TempDir,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("data")).unwrap();
        cx.update(|cx| {
            gpui_kit::init(cx);
            apply_theme(cx);
            bind_keys(cx);
        });
        let (handle, view) = cx.update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Default::default(),
                        size: size(px(1380.), px(900.)),
                    })),
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| Workspace::new(store, window, cx).unwrap()),
            )
            .unwrap()
        });
        cx.update(|cx| {
            view.update(cx, |workspace, _| {
                workspace.workspace_mode = WorkspaceMode::Coding;
                workspace.filters_open = true;
                workspace.learning_navigation = false;
                workspace.document_tab = DocumentTab::Description;
            })
        });
        (handle, view, temp)
    }

    #[gpui_kit::test]
    fn reading_mode_outline_and_shortcuts_keep_all_drafts(cx: &mut TestAppContext) {
        let (handle, view, temp) = practice_window(cx);
        let source = "package solution\n// keep my code";
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |workspace, cx| {
                assert_eq!(
                    reading::load_mode(&workspace.store.root),
                    WorkspaceMode::Study
                );
                workspace.filters_open = false;
                workspace.learning_navigation = true;
                workspace.document_tab = DocumentTab::Explanation;
                workspace.editor.update(cx, |s, cx| {
                    s.set_value(source, window, cx);
                    s.set_selected_range(8..16, cx);
                });
                workspace
                    .note_editor
                    .update(cx, |s, cx| s.set_value("# 我的草稿", window, cx));
                workspace
                    .cases_editor
                    .update(cx, |s, cx| s.set_value("test input draft", window, cx));
            });
            window.render_frame(cx);
            window.click("mode-study", cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).workspace_mode, WorkspaceMode::Study);
            assert!(window.try_find("format").is_none());
            assert!(window.find("lesson-outline").visible());
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let state = view.read(cx).document_view.as_ref().unwrap().1.clone();
            assert!(
                state
                    .read(cx)
                    .rendered_text()
                    .as_str()
                    .starts_with("从大类方法到本题")
            );
            window.click(("lesson-section", 5usize), cx);
            window.render_frame(cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).active_section, 5);
            assert!(view.read(cx).pending_section.is_none());
            assert!(
                state.read(cx).list_state().logical_scroll_top().item_ix > 0,
                "outline must scroll the actual article"
            );
            assert!(
                state.read(cx).selected_text().is_empty(),
                "navigation must not alter the text selection"
            );
            window.click("start-coding", cx);
            window.render_frame(cx);
            assert!(window.find("format").visible());
            assert_eq!(
                view.read(cx).editor.read(cx).selected_value().as_ref(),
                "solution"
            );
            window.press("ctrl-1", cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).workspace_mode, WorkspaceMode::Study);
            window.press("ctrl-2", cx);
            window.render_frame(cx);
            let workspace = view.read(cx);
            assert_eq!(workspace.workspace_mode, WorkspaceMode::Coding);
            assert_eq!(workspace.editor.read(cx).value().as_ref(), source);
            assert_eq!(
                workspace.note_editor.read(cx).value().as_ref(),
                "# 我的草稿"
            );
            assert_eq!(
                workspace.cases_editor.read(cx).value().as_ref(),
                "test input draft"
            );
            assert_eq!(
                reading::load_mode(&temp.path().join("data")),
                WorkspaceMode::Coding
            );
            window.press("ctrl-k", cx);
            window.input("two-sum", cx);
            assert_eq!(view.read(cx).search.read(cx).value().as_ref(), "two-sum");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn long_lines_wrap_and_enter_expands_go_braces(cx: &mut TestAppContext) {
        let (handle, view, _temp) = practice_window(cx);
        let source = format!("package solution\n// {}\n", "long code comment ".repeat(30));
        cx.update_window(handle, |_, window, cx| {
            view.read(cx)
                .editor
                .clone()
                .update(cx, |state, cx| state.set_value(source.clone(), window, cx));
            window.render_frame(cx);
            let state = view.read(cx).editor.read(cx);
            let start = source.find("//").unwrap();
            let end = source.len() - 2;
            let first = state.range_to_bounds(&(start..start + 1)).unwrap();
            let last = state.range_to_bounds(&(end..end + 1)).unwrap();
            assert!(last.origin.y > first.origin.y, "长行应该跨显示行折行");
            assert_eq!(state.value().as_ref(), source, "软折行不能插入源码换行");
            window.click("toggle-wrap", cx);
            assert!(!view.read(cx).soft_wrap);
            view.read(cx)
                .editor
                .clone()
                .update(cx, |state, cx| state.focus(window, cx));
            window.render_frame(cx);
            window.press("alt-z", cx);
            assert!(view.read(cx).soft_wrap);
            view.read(cx).editor.clone().update(cx, |state, cx| {
                let text = "package solution\n\nfunc answer() int {}";
                state.set_value(text, window, cx);
                state.set_cursor_position(
                    language::byte_position(text, text.len() - 1),
                    window,
                    cx,
                );
                state.focus(window, cx);
            });
            window.render_frame(cx);
            window.press("enter", cx);
            assert_eq!(
                view.read(cx).editor.read(cx).value().as_ref(),
                "package solution\n\nfunc answer() int {\n\t\n}"
            );
            window.input("return 1", cx);
            assert!(
                view.read(cx)
                    .editor
                    .read(cx)
                    .value()
                    .contains("\n\treturn 1\n}")
            );
            window.press("tab", cx);
            assert!(
                view.read(cx).editor.read(cx).value().contains("return 1\t"),
                "无补全菜单时 Tab 仍应缩进"
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn settings_language_font_and_palette_apply_immediately_and_restore(cx: &mut TestAppContext) {
        let (handle, view, temp) = practice_window(cx);
        let source = "package solution\n// 我的代码\nfunc answer() int { return 42 }";
        cx.update_window(handle, |_, window, cx| {
            view.read(cx).editor.clone().update(cx, |state, cx| {
                state.set_value(source, window, cx);
                state.set_selected_range(8..16, cx);
            });
            window.render_frame(cx);
            window.click("settings", cx);
            window.render_frame(cx);
            window.click("interface-language-english", cx);
            window.render_frame(cx);
            assert_eq!(window.find("submit").label(), Some("Submit"));
            assert_eq!(
                view.read(cx)
                    .search
                    .read(cx)
                    .presentation()
                    .placeholder()
                    .as_ref(),
                "Search problems, titles, or topics"
            );
            ui::set_site(online::Site::China, cx);
            assert_eq!(ui::language(cx), leetdesk::i18n::UiLanguage::English);
            window.click("interface-language-chinese", cx);
            window.render_frame(cx);
            assert_eq!(window.find("submit").label(), Some("提交"));
            ui::set_site(online::Site::Global, cx);
            assert_eq!(ui::language(cx), leetdesk::i18n::UiLanguage::Chinese);
            window.click("interface-language-auto", cx);
            assert_eq!(ui::language(cx), leetdesk::i18n::UiLanguage::Chinese);
            ui::set_site(online::Site::Global, cx);
            assert_eq!(ui::language(cx), leetdesk::i18n::UiLanguage::English);
            window.click("interface-language-english", cx);
            let font = view
                .read(cx)
                .editor_fonts
                .iter()
                .find(|font| font.as_str() != CODE_FONT)
                .cloned()
                .or_else(|| view.read(cx).editor_fonts.first().cloned());
            if let Some(font) = &font {
                let id = format!("editor-font-{font}");
                reveal_setting(&id, window, cx);
                window.click(id, cx);
                assert_eq!(view.read(cx).code_font(), font);
            }
            for value in [EditorTheme::Mono, EditorTheme::Amber] {
                let id = format!("editor-theme-{}", value.key());
                reveal_setting(&id, window, cx);
                window.click(id, cx);
                window.render_frame(cx);
                assert_eq!(theme::editor_theme(cx), value);
                let keyword = gpui_kit::component::Theme::global(cx)
                    .highlight_theme
                    .style
                    .syntax
                    .style("keyword")
                    .unwrap()
                    .color;
                assert_eq!(
                    keyword,
                    Some(rgb(value.color(0xff8f9c, theme::current(cx))).into())
                );
                assert!(!view.read(cx).settings_preview.read(cx).is_editable());
            }
            reveal_setting("appearance-beige", window, cx);
            window.click("appearance-beige", cx);
            window.render_frame(cx);
            assert_eq!(theme::editor_theme(cx), EditorTheme::Amber);
            let keyword = gpui_kit::component::Theme::global(cx)
                .highlight_theme
                .style
                .syntax
                .style("keyword")
                .unwrap()
                .color;
            assert_eq!(
                keyword,
                Some(rgb(EditorTheme::Amber.color(0xff8f9c, Appearance::Beige)).into())
            );
            let saved = Store::open(temp.path().join("data"))
                .unwrap()
                .editor_preferences();
            assert_eq!(saved.interface_language, InterfaceLanguage::English);
            assert_eq!(saved.editor_theme, EditorTheme::Amber);
            assert_eq!(saved.font_family, font.unwrap_or_default());
            window.click("close-settings", cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).editor.read(cx).value().as_ref(), source);
            assert_eq!(
                view.read(cx).editor.read(cx).selected_value().as_ref(),
                "solution"
            );
            view.update(cx, |workspace, cx| workspace.save(cx).unwrap());
            let reopened = cx.new(|cx| {
                Workspace::new(Store::open(temp.path().join("data")).unwrap(), window, cx).unwrap()
            });
            assert_eq!(
                reopened.read(cx).editor_preferences.interface_language,
                InterfaceLanguage::English
            );
            assert_eq!(reopened.read(cx).code_font(), view.read(cx).code_font());
            assert_eq!(theme::editor_theme(cx), EditorTheme::Amber);
            assert_eq!(ui::language(cx), leetdesk::i18n::UiLanguage::English);
            assert_eq!(reopened.read(cx).editor.read(cx).value().as_ref(), source);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn settings_gear_and_shortcut_keep_drafts_and_persist_editor_preferences(
        cx: &mut TestAppContext,
    ) {
        let (handle, view, temp) = practice_window(cx);
        let source = "package solution\n// 我的草稿";
        cx.update_window(handle, |_, window, cx| {
            view.read(cx).editor.clone().update(cx, |state, cx| {
                state.set_value(source, window, cx);
                state.set_selected_range(8..16, cx);
                state.focus(window, cx);
            });
            ui::set_site(online::Site::Global, cx);
            window.render_frame(cx);
            assert_eq!(window.find("submit").label(), Some("Submit"));
            window.click("settings", cx);
            window.render_frame(cx);
            assert!(view.read(cx).settings_open);
            assert_eq!(window.find("appearance-beige").label(), Some("Beige"));
            reveal_setting("font-larger", window, cx);
            window.click("font-larger", cx);
            reveal_setting("settings-wrap", window, cx);
            window.click("settings-wrap", cx);
            window.render_frame(cx);
            let saved = Store::open(temp.path().join("data"))
                .unwrap()
                .editor_preferences();
            assert_eq!(saved.font_size, 17);
            assert!(!saved.soft_wrap);
            window.click("close-settings", cx);
            window.render_frame(cx);
            assert!(!view.read(cx).settings_open);
            window.press("ctrl-,", cx);
            window.render_frame(cx);
            assert!(view.read(cx).settings_open);
            window.press("ctrl-,", cx);
            assert!(!view.read(cx).settings_open);
            assert_eq!(view.read(cx).editor.read(cx).value().as_ref(), source);
            assert_eq!(
                view.read(cx).editor.read(cx).selected_value().as_ref(),
                "solution"
            );
            ui::set_site(online::Site::China, cx);
            window.render_frame(cx);
            assert_eq!(window.find("submit").label(), Some("提交"));
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn appearance_buttons_update_all_surfaces_without_changing_drafts(cx: &mut TestAppContext) {
        let (handle, view, temp) = practice_window(cx);
        let source = "package solution\nfunc answer() int { return 42 }";
        cx.update_window(handle, |_, window, cx| {
            view.read(cx).editor.clone().update(cx, |state, cx| {
                state.set_value(source, window, cx);
                state.set_selected_range(8..16, cx);
            });
            let input = view.read(cx).cases_editor.read(cx).value().to_string();
            window.render_frame(cx);
            window.click("settings", cx);
            for appearance in [Appearance::White, Appearance::Beige, Appearance::Dark] {
                window.render_frame(cx);
                window.click(format!("appearance-{}", appearance.key()), cx);
                window.render_frame(cx);
                assert_eq!(theme::current(cx), appearance);
                let background: gpui_kit::Background = rgb(appearance.color(BG)).into();
                let panel: gpui_kit::Background = rgb(appearance.color(PANEL)).into();
                assert!(
                    window
                        .painted_quads()
                        .iter()
                        .any(|quad| quad.background == background)
                );
                assert!(
                    window
                        .painted_quads()
                        .iter()
                        .any(|quad| quad.background == panel)
                );
                let current = gpui_kit::component::Theme::global(cx);
                assert_eq!(current.colors.background, rgb(appearance.color(BG)).into());
                assert_eq!(
                    current.highlight_theme.style.editor_background,
                    Some(rgb(appearance.color(BG)).into())
                );
                assert_eq!(
                    current
                        .highlight_theme
                        .style
                        .syntax
                        .style("keyword")
                        .unwrap()
                        .color,
                    Some(rgb(appearance.color(0xff8f9c)).into())
                );
                assert_eq!(view.read(cx).editor.read(cx).value().as_ref(), source);
                assert_eq!(
                    view.read(cx).editor.read(cx).selected_value().as_ref(),
                    "solution"
                );
                assert_eq!(view.read(cx).cases_editor.read(cx).value().as_ref(), input);
                assert_eq!(
                    Store::open(temp.path().join("data")).unwrap().appearance(),
                    appearance
                );
                for index in [0usize, 1, 2] {
                    if index == 0 {
                        window.click("close-settings", cx);
                        window.render_frame(cx);
                    }
                    window.click(("document-tab", index), cx);
                    window.render_frame(cx);
                }
                window.click("settings", cx);
                window.render_frame(cx);
            }
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn running_cases_stay_readable_and_completion_does_not_steal_the_tab(cx: &mut TestAppContext) {
        let (handle, view, _temp) = practice_window(cx);
        let input = "[2,7,11,15]\n9";
        let (sender, events) = std::sync::mpsc::channel();
        let _subscription = cx.update(|cx| {
            cx.subscribe(&view, move |_, event: &WorkspaceEvent, _| {
                sender.send(event.clone()).unwrap();
            })
        });
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |workspace, cx| {
                let mut problem = online::parse_page(online::Site::China, &json!({"data":{"problemsetQuestionList":{"total":1,"questions":[{"frontendQuestionId":"1","titleSlug":"two-sum","title":"Two Sum"}]}}})).unwrap().problems.remove(0);
                let remote = problem.online.as_mut().unwrap();
                remote.loaded = true; remote.go_supported = true; remote.question_id = "1".into();
                workspace.problems[0] = problem;
                workspace.current = 0;
                workspace.online_enabled = true;
                workspace.cases_editor.update(cx, |state, cx| state.set_value(input, window, cx));
            });
            window.render_frame(cx);
            window.click("run", cx);
            window.render_frame(cx);
            assert!(view.read(cx).has_run_input);
            assert!(!view.read(cx).run_input_editor.read(cx).is_editable());
            assert_eq!(view.read(cx).run_input_editor.read(cx).value().as_ref(), input);
            view.read(cx).run_input_editor.clone().update(cx, |state, cx| state.focus(window, cx));
            window.input("changed", cx);
            assert_eq!(view.read(cx).run_input_editor.read(cx).value().as_ref(), input);
        }).unwrap();
        cx.run_until_parked();
        let event = events.try_recv().unwrap();
        assert!(
            matches!(event, WorkspaceEvent::Run { input: ref sent, submit: false, .. } if sent == input)
        );
        cx.update_window(handle, |_, window, cx| {
            window.click(("bottom-tab", 0usize), cx);
            window.render_frame(cx);
            view.read(cx)
                .cases_editor
                .clone()
                .update(cx, |state, cx| state.set_value("[1,2]\n3", window, cx));
            view.update(cx, |workspace, cx| {
                workspace.online_result(
                    workspace.problem().id,
                    workspace.run_generation,
                    "source",
                    "通过".into(),
                    cx,
                );
            });
            assert!(view.read(cx).bottom_tab == BottomTab::Cases);
            window.render_frame(cx);
            window.click(("bottom-tab", 1usize), cx);
            window.render_frame(cx);
            assert_eq!(
                view.read(cx).run_input_editor.read(cx).value().as_ref(),
                input
            );
            assert_eq!(
                view.read(cx).cases_editor.read(cx).value().as_ref(),
                "[1,2]\n3"
            );
            view.update(cx, |workspace, cx| workspace.select_problem(1, window, cx));
            assert!(!view.read(cx).has_run_input);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn bottom_tabs_render_cases_results_and_diagnostics(cx: &mut TestAppContext) {
        let (handle, view, _temp) = practice_window(cx);
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |workspace, cx| {
                workspace.output = "本地用例：1/2 通过\n\n✓ 用例 1\n输入：[1]\n预期：1\n实际：1\n\n✗ 用例 2\n输入：[2]\n预期：2\n实际：3\n错误：答案不匹配".into();
                workspace.diagnostics = vec![serde_json::from_value(json!({
                    "range": {"start": {"line": 1, "character": 2}, "end": {"line": 1, "character": 5}},
                    "message": "undefined: value", "severity": 1
                })).unwrap()];
                cx.notify();
            });
            for index in [1usize, 2, 0] {
                window.render_frame(cx);
                window.click(("bottom-tab", index), cx);
                window.render_frame(cx);
            }
            assert!(view.read(cx).bottom_tab == BottomTab::Cases);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn format_button_formats_and_saves_without_editor_focus(cx: &mut TestAppContext) {
        let (handle, view, _temp) = practice_window(cx);
        let draft = "package solution\nfunc answer( )int{return 1}";
        let formatted = "package solution\n\nfunc answer() int { return 1 }\n";
        cx.update_window(handle, |_, window, cx| {
            view.read(cx)
                .editor
                .clone()
                .update(cx, |state, cx| state.set_value(draft, window, cx));
            window.render_frame(cx);
            window.click("format", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, _, cx| {
            let workspace = view.read(cx);
            assert_eq!(workspace.editor.read(cx).value().as_ref(), formatted);
            assert_eq!(
                workspace.store.source(workspace.problem().id).unwrap(),
                formatted
            );
            assert!(workspace.notice.contains("gofmt"));
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn initialize_code_restores_starter_and_backs_up_current_solution(cx: &mut TestAppContext) {
        let (handle, view, _temp) = practice_window(cx);
        let draft = "package solution\n\nfunc answer() int { return 42 }\n";
        let starter = cx.update(|cx| view.read(cx).problem().starter.clone());
        cx.update_window(handle, |_, window, cx| {
            view.read(cx).editor.clone().update(cx, |state, cx| {
                state.set_value(draft, window, cx);
                state.focus(window, cx);
            });
            window.render_frame(cx);
            window.click("initialize-code", cx);
        })
        .unwrap();
        cx.update_window(handle, |_, window, cx| {
            let workspace = view.read(cx);
            assert_eq!(workspace.editor.read(cx).value().as_ref(), starter);
            assert_eq!(
                workspace.store.source(workspace.problem().id).unwrap(),
                starter
            );
            assert_eq!(
                fs::read_to_string(
                    workspace
                        .store
                        .workspace(workspace.problem().id)
                        .join("solution-before-reset-1.go")
                )
                .unwrap(),
                draft
            );
            assert!(workspace.notice.contains("已恢复题目初始代码"));
            window.press("ctrl-z", cx);
            assert_eq!(view.read(cx).editor.read(cx).value().as_ref(), draft);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn language_switch_keeps_independent_drafts_and_java_reset_is_visible(cx: &mut TestAppContext) {
        let (handle, view, _temp) = practice_window(cx);
        let go_draft = "package solution\nfunc answer() int { return 42 }\n";
        let java_draft =
            "class Solution { public int twoSum(int[] nums, int target) { return 42; } }";
        cx.update_window(handle, |_, window, cx| {
            view.read(cx)
                .editor
                .clone()
                .update(cx, |state, cx| state.set_value(go_draft, window, cx));
            view.update(cx, |workspace, cx| {
                workspace.switch_language(PracticeLanguage::Java, window, cx)
            });
            assert_eq!(view.read(cx).practice_language, PracticeLanguage::Java);
            assert!(
                view.read(cx)
                    .editor
                    .read(cx)
                    .value()
                    .contains("class Solution")
            );
            view.read(cx)
                .editor
                .clone()
                .update(cx, |state, cx| state.set_value(java_draft, window, cx));
            view.update(cx, |workspace, cx| {
                workspace.switch_language(PracticeLanguage::Go, window, cx)
            });
            assert_eq!(view.read(cx).editor.read(cx).value().as_ref(), go_draft);
            view.update(cx, |workspace, cx| {
                workspace.switch_language(PracticeLanguage::Java, window, cx)
            });
            assert_eq!(view.read(cx).editor.read(cx).value().as_ref(), java_draft);
            window.render_frame(cx);
            window.click("initialize-code", cx);
            assert!(
                view.read(cx)
                    .editor
                    .read(cx)
                    .value()
                    .contains("class Solution")
            );
            assert_ne!(view.read(cx).editor.read(cx).value().as_ref(), java_draft);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn organize_imports_button_updates_the_editor_and_saved_file(cx: &mut TestAppContext) {
        if std::process::Command::new(language::gopls_executable())
            .arg("version")
            .output()
            .is_err()
        {
            return;
        }
        let (handle, view, _temp) = practice_window(cx);
        let draft = "package solution\n\nfunc answer() { fmt.Println(1) }\n";
        cx.update_window(handle, |_, window, cx| {
            view.read(cx)
                .editor
                .clone()
                .update(cx, |state, cx| state.set_value(draft, window, cx));
            window.render_frame(cx);
            window.click("organize-imports", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, _, cx| {
            let workspace = view.read(cx);
            let source = workspace.editor.read(cx).value().to_string();
            assert!(source.contains("import \"fmt\""), "{source}");
            assert_eq!(
                workspace.store.source(workspace.problem().id).unwrap(),
                source
            );
            assert!(workspace.notice.contains("已整理 Go 导入并保存"));
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn saving_formats_valid_go_and_keeps_invalid_drafts(cx: &mut TestAppContext) {
        let (handle, view, _temp) = practice_window(cx);
        cx.update_window(handle, |_, window, cx| {
            view.read(cx).editor.clone().update(cx, |state, cx| {
                state.set_value("package solution\nfunc answer( )int{return 1}", window, cx);
                state.focus(window, cx);
            });
            window.render_frame(cx);
            window.press("ctrl-s", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, _, cx| {
            let workspace = view.read(cx);
            assert_eq!(
                workspace.editor.read(cx).value().as_ref(),
                "package solution\n\nfunc answer() int { return 1 }\n"
            );
            assert_eq!(
                workspace.store.source(workspace.problem().id).unwrap(),
                workspace.editor.read(cx).value().as_ref()
            );
        })
        .unwrap();
        let draft = "package solution\nfunc answer() {";
        cx.update_window(handle, |_, window, cx| {
            view.read(cx)
                .editor
                .clone()
                .update(cx, |state, cx| state.set_value(draft, window, cx));
            window.press("ctrl-s", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, _, cx| {
            let workspace = view.read(cx);
            assert_eq!(workspace.editor.read(cx).value().as_ref(), draft);
            assert_eq!(
                workspace.store.source(workspace.problem().id).unwrap(),
                draft
            );
            assert!(workspace.notice.contains("已保存原代码"));
        })
        .unwrap();
        // An older formatting request cannot overwrite typing that follows Ctrl+S.
        let newer = "package solution\nfunc answer() int { return 2 }\n";
        cx.update_window(handle, |_, window, cx| {
            let editor = view.read(cx).editor.clone();
            editor.update(cx, |state, cx| {
                state.set_value("package solution\nfunc answer()int{return 1}", window, cx)
            });
            window.press("ctrl-s", cx);
            editor.update(cx, |state, cx| state.set_value(newer, window, cx));
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert_eq!(view.read(cx).editor.read(cx).value().as_ref(), newer);
            // A real input event schedules auto-save, without pressing Ctrl+S.
            let editor = view.read(cx).editor.clone();
            editor.update(cx, |state, cx| {
                state.set_cursor_position(language::byte_position(newer, newer.len()), window, cx);
                state.focus(window, cx);
            });
            window.render_frame(cx);
            window.input(" ", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(500));
        cx.run_until_parked();
        cx.update_window(handle, |_, _, cx| {
            let workspace = view.read(cx);
            assert_eq!(
                workspace.store.source(workspace.problem().id).unwrap(),
                "package solution\nfunc answer() int { return 2 }\n "
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn chinese_notes_tags_and_favorites_can_be_edited_and_saved_from_the_window(
        cx: &mut TestAppContext,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("data");
        let store = Store::open(root.clone()).unwrap();
        cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::component::Theme::change(gpui_kit::component::ThemeMode::Dark, None, cx);
            cx.bind_keys([KeyBinding::new("ctrl-s", Save, Some("LeetDesk"))]);
        });
        let (handle, view) = cx.update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Default::default(),
                        size: size(px(1380.), px(900.)),
                    })),
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| Workspace::new(store, window, cx).unwrap()),
            )
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("favorite", cx);
            assert!(view.read(cx).progress.favorite);
            window.click(("document-tab", 2_usize), cx);
            view.update(cx, |this, cx| {
                this.note_editor
                    .update(cx, |state, cx| state.focus(window, cx))
            });
            window.input("中文思路🙂：使用哈希表", cx);
            window.press("ctrl-s", cx);
            assert_eq!(
                view.read(cx).note_editor.read(cx).value(),
                "中文思路🙂：使用哈希表"
            );
            assert!(!view.read(cx).dirty);
            window.click("show-filters", cx);
            window.render_frame(cx);
            window.click("custom-tags", cx);
            window.input("面试复习, 高频", cx);
            window.press("ctrl-s", cx);
            assert_eq!(view.read(cx).progress.tags.len(), 2);
            assert!(view.read(cx).progress.tags.contains(&"面试复习".into()));
            window.click("problem-search", cx);
            assert_eq!(window.find("problem-search").focused(), Some(true));
            window.input("链表", cx);
            assert_eq!(view.read(cx).search.read(cx).value(), "链表");
        })
        .unwrap();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(view.read(cx).query, "链表");
        })
        .unwrap();
        let reopened = Store::open(root).unwrap();
        assert!(reopened.progress(1).favorite);
        assert!(
            fs::read_to_string(reopened.workspace(1).join("notes.md"))
                .unwrap()
                .contains("中文思路🙂")
        );
    }
}
