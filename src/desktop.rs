use anyhow::{Result, anyhow};
use gpui_kit::base::{Disableable, TextView, TextViewState, TextViewStyle};
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
    catalog::{self, Case, Problem},
    catalog_view::CatalogView,
    language::{self, GoLanguage},
    runner, snippets,
    store::{Progress, Store},
};
use lsp_types::{CompletionContext, CompletionResponse, Hover, LocationLink, Position};
use serde_json::json;
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

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
        Quit
    ]
);

const BG: u32 = 0x000000;
const PANEL: u32 = 0x0a0a0a;
const LINE: u32 = 0x292929;
const MUTED: u32 = 0xa9b0bb;
const INK: u32 = 0xf4f5f7;
const ACCENT: u32 = 0x79e6bd;
const CODE_BG: u32 = 0x101010;

#[cfg(target_os = "macos")]
const UI_FONT: &str = "PingFang SC";
#[cfg(not(target_os = "macos"))]
const UI_FONT: &str = "Microsoft YaHei";
#[cfg(target_os = "macos")]
const CODE_FONT: &str = "Menlo";
#[cfg(not(target_os = "macos"))]
const CODE_FONT: &str = "Consolas";

pub(crate) fn ui_font() -> &'static str { UI_FONT }

type DocumentCache = ((u32, usize, u8), Entity<TextViewState>, Option<PathBuf>);

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
        let local = snippets::completions(&source, offset);
        if let Err(error) = self.service.sync(&source) {
            if local.is_empty() { return Task::ready(Err(error)); }
            return Task::ready(Ok(CompletionResponse::Array(local)));
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
                Err(_error) if !local.is_empty() => return Ok(CompletionResponse::Array(local)),
                Err(error) => return Err(error),
            };
            let mut items = local;
            if !value.is_null() {
                match serde_json::from_value::<CompletionResponse>(value)? {
                    CompletionResponse::Array(found) => items.extend(found),
                    CompletionResponse::List(found) => items.extend(found.items),
                }
            }
            Ok(CompletionResponse::Array(items))
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
    focus_mode: bool,
    online_enabled: bool,
    soft_wrap: bool,
    window_handle: AnyWindowHandle,
    progress: Progress,
    output: String,
    notice: String,
    language_status: String,
    language: Option<GoLanguage>,
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
        store.prepare(problem)?;
        let last_source = store
            .source(problem.id)
            .unwrap_or_else(|_| problem.starter.clone());
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("go")
                .soft_wrap(true)
                .smart_indent(true)
                .auto_close(true)
                .line_number(true)
                .folding(true)
                .tab_size(TabSize {
                    tab_size: 4,
                    hard_tabs: true,
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
        if let Ok(text) = fs::read_to_string(store.workspace(problem.id).join("cursor.json"))
            && let Ok(position) = serde_json::from_str::<Position>(&text)
        {
            editor.update(cx, |state, cx| {
                state.set_cursor_position(position, window, cx)
            });
        }
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索题号、标题、标签"));
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
                .placeholder(
                    "# 我的思路\n\n记录做法、复杂度与易错点。支持 Markdown，保存后自动渲染。",
                )
                .default_value(saved_note.clone())
        });
        let progress = store.progress(problem.id);
        let tag_editor = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("自定义标签，用逗号分隔")
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
        Ok(Self {
            store,
            problems,
            catalog_view,
            current,
            editor,
            search,
            cases_editor,
            note_editor,
            note_preview,
            document_view: None,
            saved_note,
            notes_view: NotesView::Split,
            note_dirty: false,
            tag_editor,
            reference_editor,
            reference_path: None,
            document_tab: DocumentTab::Description,
            bottom_tab: BottomTab::Cases,
            selected_note: 0,
            explanations,
            filter: "全部题目".into(),
            study_plan_filter: "全部题目".into(),
            status_filter: "全部".into(),
            query: String::new(),
            difficulty_filter: "全部".into(),
            show_categories: true,
            focus_mode: false,
            online_enabled: false,
            soft_wrap: true,
            window_handle: window.window_handle(),
            progress,
            output,
            notice: "Go 工作区已就绪 · 可导入你的 Markdown 讲解".into(),
            language_status: "正在连接 gopls…".into(),
            language: None,
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
            self.store.save_source(id, &self.editor.read(cx).value())?;
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
            .source(id)
            .unwrap_or_else(|_| detail.starter.clone());
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
            if let Some(existing) = loaded.get(problem.slug.as_str()) {
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
            self.bottom_tab = BottomTab::Results;
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
        if index == self.current {
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
        let source = self
            .store
            .source(problem.id)
            .unwrap_or_else(|_| problem.starter.clone());
        self.last_source = source.clone();
        self.editor.update(cx, |state, cx| {
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

    fn run_cases(&mut self, _: &RunCases, _: &mut Window, cx: &mut Context<Self>) {
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
            if let Err(error) = online::run_request(self.problem(), &source, &input) {
                self.notice = error.to_string();
                cx.notify();
                return;
            }
            self.run_generation += 1;
            let cancel = Arc::new(AtomicBool::new(false));
            self.cancellation = Some(cancel.clone());
            self.output = "正在使用当前账号在 LeetCode 运行…".into();
            self.bottom_tab = BottomTab::Results;
            cx.emit(WorkspaceEvent::Run {
                problem: self.problem().clone(),
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
        let path = self.store.workspace(problem.id);
        self.output = "正在编译并运行 Go 用例…\n首次编译标准库可能需要一些时间。".into();
        self.bottom_tab = BottomTab::Results;
        let task = cx.background_executor().spawn(async move {
            let result = runner::run(&problem, &source, &cases, &path, cancel);
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
        if let Err(error) =
            online::submit_request(self.problem(), &source).and_then(|_| self.save(cx))
        {
            self.notice = error.to_string();
            cx.notify();
            return;
        }
        self.run_generation += 1;
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancellation = Some(cancel.clone());
        self.output = "正在提交当前代码快照，等待 LeetCode 判题…".into();
        self.bottom_tab = BottomTab::Results;
        cx.emit(WorkspaceEvent::Run {
            problem: self.problem().clone(),
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

    fn organize_imports(
        &mut self,
        _: &OrganizeImports,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(service) = self.language.clone() else {
            self.notice = "请等待 gopls 连接，或点击重连".into();
            cx.notify();
            return;
        };
        let source = self.editor.read(cx).value().to_string();
        let id = self.problem().id;
        let generation = self.edit_generation;
        let task = cx.background_executor().spawn(async move {
            service.sync(&source)?;
            let actions = service.request("textDocument/codeAction", json!({"textDocument":{"uri":service.uri()}, "range":{"start":{"line":0,"character":0},"end":language::byte_position(&source,source.len())}, "context":{"diagnostics":[],"only":["source.organizeImports"]}}))?;
            let mut edits = vec![];
            if let Some(actions) = actions.as_array() {
                for action in actions {
                    if let Some(changes) = action["edit"]["documentChanges"].as_array() {
                        for change in changes { if change["textDocument"]["uri"] == service.uri() { edits.extend(serde_json::from_value::<Vec<lsp_types::TextEdit>>(change["edits"].clone())?); } }
                    }
                    if let Some(changes) = action["edit"]["changes"].as_object()
                        && let Some(values) = changes.get(service.uri()) { edits.extend(serde_json::from_value::<Vec<lsp_types::TextEdit>>(values.clone())?); }
                }
            }
            let text = language::apply_edits(&source, &edits)?;
            let patch = leetdesk::editing::diff(&source, &text);
            Ok((text, patch))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result: Result<(String, leetdesk::editing::Edits)> = task.await;
            _ = this.update_in(cx, |this, window, cx| {
                if this.problem().id != id || this.edit_generation != generation {
                    this.notice = "代码已发生变化，请再次整理导入".into();
                    cx.notify();
                    return;
                }
                match result {
                    Ok((_, edits)) => {
                        if !this.editor.update(cx, |state, cx| {
                            state.apply_programmatic_edits(&edits, window, cx)
                        }) {
                            this.notice = "正在输入，请输入完成后再整理导入".into();
                            cx.notify();
                            return;
                        }
                        this.changed(cx);
                        this.notice = "已整理 Go 导入 · 可撤销".into();
                    }
                    Err(error) => this.notice = error.to_string(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn import_notes(&mut self, _: &ImportNotes, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(error) = self.save(cx) {
            self.notice = error.to_string();
            cx.notify();
            return;
        }
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: true,
            multiple: true,
            prompt: Some("导入 Markdown 文件或文件夹；未标明题号的文件将关联当前题目".into()),
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
            prompt: Some("导入题目 JSON（格式参见 assets/problems.json）".into()),
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
        let source = source
            .lines()
            .filter(|line| !line.trim_start().starts_with("package "))
            .collect::<Vec<_>>()
            .join("\n");
        cx.write_to_clipboard(ClipboardItem::new_string(source));
        self.notice = "已复制解法（移除了本地 package 声明）".into();
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
        self.notice = "已保存，正在运行 gofmt…".into();
        let input = source.clone();
        let task = cx.background_executor().spawn(async move {
            runner::format_go(&input).map(|text| {
                let edits = leetdesk::editing::diff(&input, &text);
                (text, edits)
            })
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            _ = this.update_in(cx, |this, window, cx| {
                // A formatter for an older snapshot may not replace newer edits.
                if this.problem().id != id {
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
                            Ok(()) if text == source => "已保存 · 代码已符合 gofmt 格式".into(),
                            Ok(()) => "已保存并使用 gofmt 格式化 · 可撤销".into(),
                            Err(error) => format!("格式化完成，保存失败：{error}"),
                        };
                    }
                    Err(error) => this.notice = format!("已保存原代码，gofmt 未完成：{error}"),
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
        let problem = &self.problems[index];
        let remote = problem.online.as_ref();
        let status = if remote.is_some() {
            online::status_label(problem).to_string()
        } else {
            self.catalog_view
                .progress(problem.id)
                .map(|p| p.state.clone())
                .unwrap_or_default()
        };
        let number = remote
            .map(|r| r.frontend_id.clone())
            .unwrap_or_else(|| problem.id.to_string());
        div()
            .id(("problem", problem.id as usize))
            .h(px(78.))
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(rgb(LINE))
            .cursor_pointer()
            .overflow_hidden()
            .when(index == self.current, |el| {
                el.bg(rgb(0x18352a)).border_l_2().border_color(rgb(ACCENT))
            })
            .hover(|el| el.bg(rgb(0x181818)))
            .on_click(
                cx.listener(move |this, _, window, cx| this.select_problem(index, window, cx)),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .gap_2()
                    .text_xs()
                    .child(div().text_color(rgb(MUTED)).child(number))
                    .child(
                        div()
                            .text_color(rgb(if status == "已通过" { ACCENT } else { MUTED }))
                            .child(status),
                    ),
            )
            .child(div().text_sm().mt_1().child(problem.title.clone()))
            .child(div().text_xs().text_color(rgb(MUTED)).mt_1().child(format!(
                "{}{}",
                leetdesk::topics::difficulty(&problem.difficulty),
                if remote.is_some_and(|m| m.paid_only) {
                    " · 会员"
                } else {
                    ""
                }
            )))
    }
    fn navigation(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (solved, available) = self.catalog_view.hot_progress();
        let topics = self.catalog_view.topics().to_vec();
        let custom = self.catalog_view.custom_tags().to_vec();
        div()
            .size_full()
            .bg(rgb(BG))
            .flex()
            .flex_col()
            .p_2()
            .gap_1()
            .child(
                div()
                    .px_2()
                    .py_3()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child("分类与练习"),
            )
            .child(
                div()
                    .id("category-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .px_3()
                            .py_2()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child("题单"),
                    )
                    .children(["全部题目", "热题 100"].into_iter().enumerate().map(
                        |(index, label)| {
                            Button::new(("study-plan", index))
                                .label(label)
                                .small()
                                .ghost()
                                .w_full()
                                .justify_start()
                                .rounded_md()
                                .cursor_pointer()
                                .text_sm()
                                .text_color(rgb(if self.study_plan_filter == label {
                                    ACCENT
                                } else {
                                    INK
                                }))
                                .when(self.study_plan_filter == label, |el| el.bg(rgb(0x18352a)))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.study_plan_filter = label.into();
                                    cx.notify();
                                }))
                        },
                    ))
                    .child(
                        div()
                            .px_3()
                            .pb_2()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child(format!("热题：{solved}/{available} 已通过")),
                    )
                    .when(available < 100, |el| {
                        el.child(
                            div()
                                .px_3()
                                .text_xs()
                                .text_color(rgb(MUTED))
                                .child("完整题单需同步在线题库"),
                        )
                    })
                    .child(
                        div()
                            .px_3()
                            .pt_4()
                            .pb_2()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child("主题"),
                    )
                    .child(self.topic_row("全部题目", false, cx))
                    .children(topics.iter().map(|topic| self.topic_row(topic, false, cx)))
                    .when(!custom.is_empty(), |el| {
                        el.child(
                            div()
                                .px_3()
                                .pt_4()
                                .pb_2()
                                .text_xs()
                                .text_color(rgb(MUTED))
                                .child("我的标签"),
                        )
                    })
                    .children(custom.iter().map(|tag| self.topic_row(tag, true, cx))),
            )
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
        .label(text)
        .small()
        .ghost()
        .w_full()
        .justify_start()
        .rounded_md()
        .text_sm()
        .cursor_pointer()
        .text_color(rgb(if selected { ACCENT } else { MUTED }))
        .when(selected, |el| el.bg(rgb(0x18352a)))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.filter = value.clone();
            cx.notify();
        }))
        .into_any_element()
    }
    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let indices = self.filtered_indices();
        let count = indices.len();
        let active_visible = indices.contains(&self.current);
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(PANEL))
            .child(
                div()
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(Input::new(&self.search).id("problem-search").small())
                    .child(
                        div().flex().flex_wrap().gap_1().children(
                            ["全部", "未解答", "尝试过", "已通过", "收藏", "待复习"]
                                .into_iter()
                                .enumerate()
                                .map(|(index, label)| {
                                    Button::new(("status-filter", index))
                                        .label(if index == 0 { "全部状态" } else { label })
                                        .small()
                                        .ghost()
                                        .when(self.status_filter == label, |button| {
                                            button.text_color(rgb(ACCENT))
                                        })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.status_filter = label.into();
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
                                .map(|(index, label)| {
                                    Button::new(("difficulty", index))
                                        .label(label)
                                        .small()
                                        .ghost()
                                        .when(self.difficulty_filter == label, |b| {
                                            b.text_color(rgb(ACCENT))
                                        })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.difficulty_filter = label.into();
                                            cx.notify();
                                        }))
                                }),
                        ),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child(format!(
                                "{}{} · {count} 道",
                                if self.study_plan_filter == "热题 100" {
                                    "热题 100 · "
                                } else {
                                    ""
                                },
                                if self.filter == "全部题目" {
                                    "全部主题"
                                } else {
                                    self.filter.strip_prefix("@custom:").unwrap_or(&self.filter)
                                }
                            ))
                            .child(
                                Button::new("clear-filters")
                                    .label("重置")
                                    .small()
                                    .ghost()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.filter = "全部题目".into();
                                        this.study_plan_filter = "全部题目".into();
                                        this.status_filter = "全部".into();
                                        this.difficulty_filter = "全部".into();
                                        this.search.update(cx, |state, cx| {
                                            state.set_value("", window, cx)
                                        });
                                        this.query.clear();
                                        cx.notify();
                                    })),
                            ),
                    )
                    .when(!active_visible, |el| {
                        el.child(
                            div()
                                .text_xs()
                                .text_color(rgb(0xd5ae74))
                                .child("当前编辑题目不在筛选结果内"),
                        )
                    }),
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
                                cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                                    range
                                        .filter_map(|row| indices.get(row).copied())
                                        .map(|index| {
                                            this.question_row(index, cx).into_any_element()
                                        })
                                        .collect::<Vec<_>>()
                                }),
                            )
                            .size_full(),
                        )
                    })
                    .when(count == 0, |el| {
                        el.child(
                            div()
                                .p_4()
                                .text_sm()
                                .text_color(rgb(MUTED))
                                .child("没有匹配题目"),
                        )
                    }),
            )
            .when(self.problem().online.is_none(), |el| {
                el.child(
                    Button::new("import-catalog")
                        .label("导入题目 JSON")
                        .small()
                        .ghost()
                        .on_click(cx.listener(Self::import_catalog)),
                )
            })
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
                    .text_color(rgb(MUTED))
                    .child("Markdown 源码"),
            )
            .child(
                Editor::new(&self.note_editor)
                    .size_full()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .font_family(CODE_FONT)
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
                    .text_color(rgb(MUTED))
                    .child("保存后的预览"),
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
                                .text_color(rgb(MUTED))
                                .child("写下思路并保存，Markdown 将在这里渲染。"),
                        )
                    })
                    .when(!self.saved_note.is_empty(), |el| {
                        el.child(
                            TextView::new(&self.note_preview)
                                .selectable(true)
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
                    .border_color(rgb(LINE))
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .items_center()
                            .child(div().text_xs().text_color(rgb(MUTED)).child(format!(
                                "notes.md · {}",
                                if self.note_dirty {
                                    "未保存"
                                } else {
                                    "已保存"
                                }
                            )))
                            .child(
                                Button::new("open-notes-file")
                                    .small()
                                    .ghost()
                                    .label("打开 MD")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let path = this
                                            .store
                                            .workspace(this.problem().id)
                                            .join("notes.md");
                                        if let Err(error) = this.save_notes_document(cx) {
                                            this.notice = format!("笔记保存失败：{error:#}");
                                        } else if let Ok(uri) = language::file_uri(&path) {
                                            cx.open_url(&uri);
                                        }
                                        cx.notify();
                                    })),
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
                                                .label(label)
                                                .small()
                                                .ghost()
                                                .when(self.notes_view == mode, |button| {
                                                    button.text_color(rgb(ACCENT))
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
                                        .label("保存笔记")
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
            (DocumentTab::Explanation, "做法讲解"),
            (DocumentTab::Notes, "我的笔记"),
        ];
        let mut content = div()
            .id("document-content")
            .flex_1()
            .min_h_0()
            .min_w_0()
            .size_full();
        if self.document_tab == DocumentTab::Notes {
            content = content.child(self.notes_document(cx));
        } else {
            let key = (
                self.problem().id,
                self.selected_note,
                self.document_tab as u8,
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
                    self.explanations
                        .get(self.selected_note)
                        .map(|(_, body)| catalog::markdown_body(body).to_string())
                        .unwrap_or_else(|| "还没有讲解。点击“导入 MD”添加。".into())
                };
                let state =
                    if self.document_tab == DocumentTab::Description && problem.online.is_some() {
                        cx.new(|cx| {
                            TextViewState::html(
                                if markdown.is_empty() {
                                    "<p>正在从 LeetCode 加载题面和 Go 模板…</p>"
                                } else {
                                    &markdown
                                },
                                cx,
                            )
                        })
                    } else {
                        cx.new(|cx| TextViewState::markdown(&markdown, cx))
                    };
                self.document_view = Some((key, state, origin));
            }
            let (_, state, origin) = self.document_view.as_ref().unwrap();
            let origin = origin.clone();
            let image_origin = origin.clone();
            content = content.overflow_y_scroll().p_5().child(
                TextView::new(state)
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
                    .style(document_style())
                    .text_size(px(15.)),
            );
        }
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(PANEL))
            .child(
                div()
                    .flex()
                    .items_center()
                    .h_12()
                    .px_3()
                    .gap_1()
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .children(tabs.into_iter().enumerate().map(|(index, (tab, label))| {
                        Button::new(("document-tab", index))
                            .label(label)
                            .small()
                            .ghost()
                            .when(self.document_tab == tab, |button| {
                                button.text_color(rgb(ACCENT))
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.document_tab = tab;
                                cx.notify();
                            }))
                    })),
            )
            .when(self.document_tab == DocumentTab::Explanation, |el| {
                el.child(
                    div().px_3().py_2().flex().flex_wrap().gap_1().children(
                        self.explanations
                            .iter()
                            .enumerate()
                            .map(|(index, (name, _))| {
                                Button::new(("explanation", index))
                                    .label(name.clone())
                                    .small()
                                    .ghost()
                                    .when(index == self.selected_note, |button| {
                                        button.text_color(rgb(ACCENT))
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.selected_note = index;
                                        cx.notify();
                                    }))
                            }),
                    ),
                )
            })
            .child(content)
            .child(
                div()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(rgb(LINE))
                    .child(Input::new(&self.tag_editor).id("custom-tags").small()),
            )
    }

    fn editor_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(path) = &self.reference_path {
            return div()
                .size_full()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .h(px(36.))
                        .px_3()
                        .flex()
                        .items_center()
                        .justify_between()
                        .bg(rgb(PANEL))
                        .child(div().text_xs().child(path.clone()))
                        .child(
                            Button::new("close-definition")
                                .label("返回解法")
                                .small()
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
                        .min_w_0()
                        .flex_1()
                        .min_h_0()
                        .font_family(CODE_FONT)
                        .text_size(px(15.)),
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
                    .h(px(36.))
                    .px_4()
                    .flex()
                    .items_center()
                    .justify_between()
                    .bg(rgb(PANEL))
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .items_center()
                            .child(
                                Button::new("toggle-wrap")
                                    .label("折行")
                                    .small()
                                    .ghost()
                                    .when(self.soft_wrap, |button| button.text_color(rgb(ACCENT)))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.toggle_wrap(&ToggleWrap, window, cx)
                                    })),
                            )
                            .child(div().text_sm().text_color(rgb(ACCENT)).child("solution.go"))
                            .child(div().text_xs().text_color(rgb(MUTED)).child(if self.dirty {
                                "未保存"
                            } else {
                                "已保存"
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .items_center()
                            .child(
                                Button::new("organize-imports")
                                    .label("整理导入")
                                    .small()
                                    .ghost()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.organize_imports(&OrganizeImports, window, cx)
                                    })),
                            )
                            .child(
                                Button::new("format")
                                    .label("格式化")
                                    .small()
                                    .ghost()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.format(&Format, window, cx)
                                    })),
                            )
                            .child(
                                Button::new("copy-code")
                                    .label("复制解法")
                                    .small()
                                    .ghost()
                                    .on_click(cx.listener(Self::copy_submission)),
                            ),
                    ),
            )
            .child(
                Editor::new(&self.editor)
                    .readonly(
                        self.problem()
                            .online
                            .as_ref()
                            .is_some_and(|r| !r.loaded || !r.go_supported),
                    )
                    .size_full()
                    .min_w_0()
                    .flex_1()
                    .min_h_0()
                    .font_family(CODE_FONT)
                    .text_size(px(16.)),
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
                        .border_color(rgb(LINE))
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(div().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(rgb(ACCENT)).child(
                            if self.problem().online.is_some() { "在线测试输入" } else { "本地测试数据" },
                        ))
                        .child(div().text_xs().text_color(rgb(MUTED)).child(
                            if self.problem().online.is_some() {
                                "每个参数占一行 · 修改后点击上方测试"
                            } else {
                                "按参数顺序填写 JSON · 链表和树使用数组"
                            },
                        )),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .m_3()
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(LINE))
                        .bg(rgb(BG))
                        .child(
                            Editor::new(&self.cases_editor)
                                .size_full()
                                .font_family(CODE_FONT)
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
                let tone = if failed { 0xff928c } else if passed { ACCENT } else { MUTED };
                let detail = output.split_once('\n').map_or("", |(_, detail)| detail).replace("\r\n", "\n");
                div()
                    .id("results-scroll")
                    .size_full()
                    .overflow_y_scroll()
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .font_family(CODE_FONT)
                    .child(
                        div()
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(rgb(if failed { 0x231313 } else if passed { 0x102018 } else { CODE_BG }))
                            .border_l_2()
                            .border_color(rgb(tone))
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(tone))
                            .child(heading.to_string()),
                    )
                    .when(output.is_empty(), |panel| {
                        panel.child(div().px_3().text_xs().text_color(rgb(MUTED)).child("运行代码后，这里会显示每个用例的输入、预期和实际结果。"))
                    })
                    .children(detail.split("\n\n").filter(|section| !section.trim().is_empty()).map(|section| {
                        let case_ok = section.trim_start().starts_with('✓');
                        let case_failed = section.trim_start().starts_with('✗');
                        div()
                            .p_3()
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(if case_ok { 0x285440 } else if case_failed { 0x61332f } else { LINE }))
                            .bg(rgb(CODE_BG))
                            .flex()
                            .flex_col()
                            .gap_1()
                            .children(section.lines().filter(|line| !line.trim().is_empty()).map(|line| {
                                let line = line.trim();
                                if line.starts_with('✓') || line.starts_with('✗') {
                                    let ok = line.starts_with('✓');
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(rgb(if ok { ACCENT } else { 0xff928c }))
                                        .child(line.to_string())
                                        .into_any_element()
                                } else if let Some((label, value)) = line.split_once('：') {
                                    let is_error = label.contains("错误") || label.contains("失败");
                                    div()
                                        .flex()
                                        .items_start()
                                        .gap_3()
                                        .text_xs()
                                        .child(div().w(px(64.)).flex_shrink_0().text_color(rgb(MUTED)).child(label.to_string()))
                                        .child(div().flex_1().min_w_0().text_color(rgb(if is_error { 0xff928c } else { INK })).child(value.trim().to_string()))
                                        .into_any_element()
                                } else {
                                    div().text_xs().text_color(rgb(INK)).child(line.to_string()).into_any_element()
                                }
                            }))
                            .into_any_element()
                    }))
                    .into_any_element()
            },
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
                            .border_color(rgb(LINE))
                            .bg(rgb(CODE_BG))
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).text_color(rgb(if self.language.is_some() { ACCENT } else { MUTED })).child(
                                if self.language.is_some() { "✓ 暂无代码问题" } else { "等待 Go 语言服务" },
                            ))
                            .child(div().text_xs().text_color(rgb(MUTED)).child(
                                if self.language.is_some() { "继续编写代码，检查结果会实时更新。" } else { "连接后会在这里显示错误和警告。" },
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
                                .bg(rgb(CODE_BG))
                                .border_l_2()
                                .border_color(rgb(color))
                                .cursor_pointer()
                                .hover(|el| el.bg(rgb(LINE)))
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
                                        .child(div().font_weight(FontWeight::SEMIBOLD).text_color(rgb(color)).child(kind))
                                        .child(div().text_color(rgb(MUTED)).child(format!("{}:{}", position.line + 1, position.character + 1))),
                                )
                                .child(div().mt_1().text_sm().text_color(rgb(INK)).child(diagnostic.message.clone()))
                        }),
                )
                .into_any_element(),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(PANEL))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .h_10()
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .children(tabs.into_iter().enumerate().map(|(index, (tab, label))| {
                        let label = if tab == BottomTab::Diagnostics {
                            format!("{label} ({})", self.diagnostics.len())
                        } else {
                            label.into()
                        };
                        div()
                            .h_full()
                            .flex()
                            .items_center()
                            .border_b_2()
                            .border_color(rgb(if self.bottom_tab == tab { ACCENT } else { PANEL }))
                            .child(
                                Button::new(("bottom-tab", index))
                                    .label(label)
                                    .small()
                                    .ghost()
                                    .when(self.bottom_tab == tab, |button| {
                                        button.text_color(rgb(ACCENT))
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let problem = self.problem();
        let number = problem
            .online
            .as_ref()
            .map(|r| r.frontend_id.clone())
            .unwrap_or_else(|| problem.id.to_string());
        let title = format!("{number}. {}", problem.title);
        let platform = problem
            .online
            .as_ref()
            .map(|_| online::status_label(problem))
            .unwrap_or("本地练习");
        let origin = problem
            .online
            .as_ref()
            .map(|r| r.site.origin())
            .unwrap_or("https://leetcode.cn");
        let link = format!("{origin}/problems/{}/", problem.slug);
        let running = self.cancellation.is_some();
        let right = v_resizable("code-and-results")
            .child(
                resizable_panel()
                    .size(px(550.))
                    .size_range(px(180.)..px(2400.))
                    .child(self.editor_panel(cx)),
            )
            .child(
                resizable_panel()
                    .size(px(200.))
                    .size_range(px(90.)..px(1200.))
                    .child(self.bottom_panel(cx)),
            );
        let mut panels = h_resizable("workspace-v2");
        if self.show_categories && !self.focus_mode {
            panels = panels.child(
                resizable_panel()
                    .size(px(160.))
                    .size_range(px(135.)..px(250.))
                    .child(self.navigation(cx)),
            );
        }
        if !self.focus_mode {
            panels = panels
                .child(
                    resizable_panel()
                        .size(px(270.))
                        .size_range(px(220.)..px(450.))
                        .child(self.sidebar(cx)),
                )
                .child(
                    resizable_panel()
                        .size(px(440.))
                        .size_range(px(300.)..px(1500.))
                        .child(self.document(cx)),
                );
        }
        panels = panels.child(
            resizable_panel()
                .size(px(720.))
                .size_range(px(400.)..px(3000.))
                .child(right),
        );
        let position = self.editor.read(cx).cursor_position();
        div()
            .key_context("LeetDesk")
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(BG))
            .text_color(rgb(INK))
            .font_family(UI_FONT)
            .on_action(cx.listener(Self::save_action))
            .on_action(cx.listener(Self::run_cases))
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::format))
            .on_action(cx.listener(Self::import_notes))
            .on_action(cx.listener(Self::organize_imports))
            .on_action(cx.listener(Self::toggle_wrap))
            .child(
                div()
                    .h(px(46.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .child(
                        Button::new("toggle-categories")
                            .label("☰ 分类")
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_categories = !this.show_categories;
                                this.focus_mode = false;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            .window_control_area(WindowControlArea::Drag)
                            .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
                            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(title))
                            .child(div().text_xs().text_color(rgb(ACCENT)).child(platform)),
                    )
                    .child(
                        Button::new("import-md")
                            .label("导入 MD")
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.import_notes(&ImportNotes, window, cx)
                            })),
                    )
                    .child(
                        Button::new("favorite")
                            .label(if self.progress.favorite {
                                "★ 收藏"
                            } else {
                                "☆ 收藏"
                            })
                            .small()
                            .ghost()
                            .on_click(cx.listener(Self::toggle_favorite)),
                    )
                    .child(
                        Button::new("review")
                            .label(if self.progress.state == "待复习" {
                                "✓ 待复习"
                            } else {
                                "复习"
                            })
                            .small()
                            .ghost()
                            .on_click(cx.listener(Self::mark_review)),
                    )
                    .child(
                        Button::new("focus-code")
                            .label(if self.focus_mode {
                                "退出专注"
                            } else {
                                "专注编码"
                            })
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.focus_mode = !this.focus_mode;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("run")
                            .label(if running && self.problem().online.is_some() {
                                "停止等待"
                            } else if running {
                                "取消运行"
                            } else if self.problem().online.is_some() {
                                "▶ 测试"
                            } else {
                                "▶ 运行用例"
                            })
                            .small()
                            .primary()
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
                            .label("提交")
                            .small()
                            .primary()
                            .disabled(
                                running
                                    || !self.online_enabled
                                    || self
                                        .problem()
                                        .online
                                        .as_ref()
                                        .is_none_or(|m| !m.loaded || !m.go_supported),
                            )
                            .on_click(
                                cx.listener(|this, _, window, cx| this.submit(&Submit, window, cx)),
                            ),
                    )
                    .child(
                        Button::new("platform")
                            .label("题目网页")
                            .small()
                            .ghost()
                            .on_click(move |_, _, cx| cx.open_url(&link)),
                    )
                    .child(
                        Button::new("close-window")
                            .label("关闭")
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, window, cx| {
                                match this.save(cx) {
                                    Ok(_) => window.remove_window(),
                                    Err(error) => {
                                        this.notice = format!("保存失败，窗口保持打开：{error}");
                                        cx.notify();
                                    }
                                }
                            })),
                    ),
            )
            .child(div().flex_1().min_h_0().child(panels))
            .child(
                div()
                    .h(px(26.))
                    .px_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_t_1()
                    .border_color(rgb(LINE))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child(self.notice.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("reconnect")
                                    .label(if self.language.is_some() {
                                        "Go 检查已连接"
                                    } else {
                                        "连接 Go 检查"
                                    })
                                    .small()
                                    .ghost()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.request_current(window, cx)
                                    })),
                            )
                            .child(div().text_xs().text_color(rgb(MUTED)).child(format!(
                                "Ln {}, Col {}",
                                position.line + 1,
                                position.character + 1
                            ))),
                    ),
            )
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

impl Drop for Workspace {
    fn drop(&mut self) {
        if let Some(cancel) = &self.cancellation {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}

pub fn launch(store: Store, smoke: bool) -> Result<()> {
    let smoke_ok = Arc::new(AtomicBool::new(!smoke));
    let result = smoke_ok.clone();
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            apply_theme(cx);
            cx.on_action(|_: &Quit, cx| cx.quit());
            bind_keys(cx);
            let options = WindowOptions {
                titlebar: None,
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1380.), px(900.)),
                    cx,
                ))),
                window_min_size: Some(size(px(1060.), px(650.))),
                app_id: Some("leetdesk".into()),
                show: !smoke,
                ..Default::default()
            };
            let (_, view) = gpui_kit::open_window(options, cx, move |window, cx| {
                window.set_window_title("LeetDesk · Go 刷题工作区");
                cx.new(|cx| Workspace::new(store, window, cx).expect("无法初始化刷题工作区"))
            })
            .expect("无法打开窗口");
            let handle = cx.windows().last().copied();
            if let Some(handle) = handle {
                let _ = handle.update(cx, |_, window, cx| {
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
            if smoke {
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

pub fn apply_theme(cx: &mut App) {
    gpui_kit::component::Theme::change(gpui_kit::component::ThemeMode::Dark, None, cx);
    let theme = gpui_kit::component::Theme::global_mut(cx);
    theme.shadow = false;
    theme.colors.background = rgb(BG).into();
    theme.colors.foreground = rgb(INK).into();
    theme.colors.border = rgb(LINE).into();
    theme.colors.muted_foreground = rgb(MUTED).into();
    theme.colors.input = rgb(PANEL).into();
    theme.colors.popover = rgb(PANEL).into();
    theme.colors.popover_foreground = rgb(INK).into();
    theme.colors.sidebar = rgb(PANEL).into();
    theme.colors.sidebar_foreground = rgb(INK).into();
    theme.colors.sidebar_border = rgb(LINE).into();
    theme.colors.selection = rgb(0x255c49).into();
    let syntax = Arc::make_mut(&mut theme.highlight_theme);
    syntax.style.editor_background = Some(rgb(BG).into());
    syntax.style.editor_gutter_background = Some(rgb(BG).into());
    syntax.style.editor_foreground = Some(rgb(INK).into());
    syntax.style.editor_active_line = Some(rgb(0x171717).into());
    syntax.style.editor_line_number = Some(rgb(MUTED).into());
    syntax.style.syntax = serde_json::from_value(json!({
        "keyword": {"color": gpui_kit::Hsla::from(rgb(0xd3a7ff))},
        "string": {"color": gpui_kit::Hsla::from(rgb(0x91e7a5))},
        "number": {"color": gpui_kit::Hsla::from(rgb(0xffb86b))},
        "boolean": {"color": gpui_kit::Hsla::from(rgb(0xffb86b))},
        "function": {"color": gpui_kit::Hsla::from(rgb(0x86c9ff))},
        "type": {"color": gpui_kit::Hsla::from(rgb(0x79e5e1))},
        "variable": {"color": gpui_kit::Hsla::from(rgb(INK))},
        "property": {"color": gpui_kit::Hsla::from(rgb(0xe8dbff))},
        "constant": {"color": gpui_kit::Hsla::from(rgb(0xffd084))},
        "operator": {"color": gpui_kit::Hsla::from(rgb(0xff91b7))},
        "comment": {"color": gpui_kit::Hsla::from(rgb(0x9ba2ac))},
        "punctuation": {"color": gpui_kit::Hsla::from(rgb(0xc9d0da))}
    })).expect("Go 高亮颜色有效");
}

fn document_style() -> TextViewStyle {
    let mut code_block = gpui_kit::StyleRefinement::default();
    code_block.text.color = Some(rgb(INK).into());
    TextViewStyle::default()
        .with_dark(true)
        .with_foreground(rgb(INK).into())
        .with_muted_foreground(rgb(MUTED).into())
        .with_link(rgb(ACCENT).into())
        .with_border(rgb(LINE).into())
        .with_selection(rgb(ACCENT).opacity(0.22).into())
        .with_code_background(rgb(CODE_BG).into())
        .with_inline_code(gpui_kit::HighlightStyle {
            color: Some(rgb(INK).into()),
            background_color: Some(rgb(CODE_BG).into()),
            ..Default::default()
        })
        .with_code_block(code_block)
}

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
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
        KeyBinding::new("cmd-s", Save, Some("LeetDesk")),
        KeyBinding::new("cmd-enter", RunCases, Some("LeetDesk")),
        KeyBinding::new("cmd-shift-enter", Submit, Some("LeetDesk")),
        KeyBinding::new("cmd-shift-f", Format, Some("LeetDesk")),
        KeyBinding::new("cmd-shift-o", OrganizeImports, Some("LeetDesk")),
        KeyBinding::new("cmd-i", ImportNotes, Some("LeetDesk")),
        KeyBinding::new("cmd-j", gpui_kit::component::input::ShowCompletions, Some("Input && mode == editor")),
    ]);
}

#[cfg(test)]
mod ui_tests {
    use super::*;
    use gpui_kit::TestAppContext;
    use gpui_kit::test::TestWindowExt;

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
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div()
                    .w(px(self.width))
                    .text_size(px(15.))
                    .child(gpui_kit::base::TextSelectionLayer)
                    .child(TextView::new(&self.text).style(document_style()))
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
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div()
                    .w(px(self.width))
                    .text_size(px(15.))
                    .text_color(rgb(INK))
                    .bg(rgb(PANEL))
                    .child(TextView::new(&self.state).style(document_style()))
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
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div()
                    .w(px(200.))
                    .font_family(UI_FONT)
                    .text_size(px(15.))
                    .text_bg(rgb(TEXT_MARKER))
                    .child(TextView::new(&self.state).style(document_style()))
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
        let markdown = "# 中文思路🙂\n\n使用 **哈希表**，复杂度 O(n)。\n\n- 边界：空数组\n\n```go\n\treturn 42\n```\n";
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
                    .into_iter().find(|item| item.label.starts_with("for ·")).unwrap();
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
        }).unwrap();
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
        }).unwrap();
        let saved = temp.path().join("data").join("workspaces").join(id.to_string()).join("solution.go");
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
            assert_eq!(keyword, rgb(0xd3a7ff).into());
            assert_eq!(string, rgb(0x91e7a5).into());
            assert_eq!(number, rgb(0xffb86b).into());
            assert_eq!(theme.style.editor_background, Some(rgb(BG).into()));
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
        (handle, view, temp)
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
            assert_eq!(workspace.store.source(workspace.problem().id).unwrap(), formatted);
            assert!(workspace.notice.contains("gofmt"));
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
