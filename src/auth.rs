use crate::desktop::{Workspace, WorkspaceEvent};
use anyhow::{Context, Result, anyhow};
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonVariants},
};
use gpui_kit::{
    Bounds, Entity, FontWeight, Subscription, Task, TitlebarOptions, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, rgb, size,
};
use gpui_wry::WebView;
use leetdesk::{
    catalog::Problem,
    online::{self, OnlineCache, Request, Site},
    store::Store,
};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Checking,
    Login,
    Syncing,
    Ready,
    Offline,
    Error,
}

enum BrowserEvent {
    Loaded(Site, String),
    Reply(Site, Value),
}

fn login_popup_allowed(url: &str) -> bool {
    // OAuth pages also open a blank window before assigning its HTTPS URL.
    url == "about:blank"
        || url::Url::parse(url).is_ok_and(|url| url.scheme() == "https" && url.host_str().is_some())
}

#[derive(Clone)]
enum Operation {
    Account,
    List(usize),
    Legacy,
    VerifyIndex,
    StudyPlan,
    Detail(Box<Problem>),
    Judge(Box<JudgeJob>),
    AuthorizeJudge {
        job: Box<JudgeJob>,
        request: Request,
    },
}

#[derive(Clone)]
struct JudgeJob {
    problem_id: u32,
    source: String,
    generation: u64,
    cancel: Arc<AtomicBool>,
    task: Option<String>,
    started: Instant,
    submit: bool,
    owner: String,
    site: Site,
}

pub struct AccountShell {
    root: PathBuf,
    site: Site,
    phase: Phase,
    username: Option<String>,
    message: String,
    workspace: Option<Entity<Workspace>>,
    browser: Option<Entity<WebView>>,
    // Keep the native view before its context so it drops first.
    _web_context: Option<wry::WebContext>,
    events: mpsc::Receiver<BrowserEvent>,
    pending: HashMap<u64, (Operation, Instant)>,
    delayed: Vec<(Instant, Operation, Request)>,
    next_id: u64,
    questions: Vec<Problem>,
    expected_total: usize,
    solved: usize,
    catalogue_total: usize,
    loaded: bool,
    last_account_check: Instant,
    _subscription: Option<Subscription>,
    _poll: Task<()>,
    probe: bool,
    plan_probe: bool,
    probe_result: Arc<AtomicBool>,
    probe_started: Instant,
}

impl AccountShell {
    fn blank(root: PathBuf, _cx: &mut gpui_kit::Context<Self>) -> Self {
        let site = fs::read_to_string(root.join("site.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let (_, events) = mpsc::channel();
        Self {
            root,
            site,
            phase: Phase::Checking,
            username: None,
            message: "正在连接 LeetCode，检查登录状态…".into(),
            workspace: None,
            browser: None,
            _web_context: None,
            events,
            pending: HashMap::new(),
            delayed: vec![],
            next_id: 1,
            questions: vec![],
            expected_total: 0,
            solved: 0,
            catalogue_total: 0,
            loaded: false,
            last_account_check: Instant::now(),
            _subscription: None,
            _poll: Task::ready(()),
            probe: false,
            plan_probe: false,
            probe_result: Arc::new(AtomicBool::new(false)),
            probe_started: Instant::now(),
        }
    }

    pub fn new(root: PathBuf, window: &mut Window, cx: &mut gpui_kit::Context<Self>) -> Self {
        let mut this = Self::blank(root, cx);
        if let Err(error) = this.build_browser(window, cx) {
            this.phase = Phase::Error;
            this.message = format!("无法打开官方登录页面：{error:#}");
        }
        this._poll = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
                if this
                    .update_in(cx, |this, window, cx| this.tick(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        this
    }

    fn build_browser(
        &mut self,
        window: &mut Window,
        cx: &mut gpui_kit::Context<Self>,
    ) -> Result<()> {
        if let Some(browser) = self.browser.take() {
            browser.update(cx, |view, _| view.hide());
        }
        self._web_context = None;
        self.pending.clear();
        self.delayed.clear();
        self.loaded = false;
        let dir = self.root.join("browser").join(self.site.key());
        fs::create_dir_all(&dir)?;
        let mut context = wry::WebContext::new(Some(dir));
        let (sender, events) = mpsc::channel();
        self.events = events;
        let site = self.site;
        let page_sender = sender.clone();
        let builder = wry::WebViewBuilder::new_with_web_context(&mut context)
            .with_url(site.origin())
            // WebView2 otherwise marks every window.open request as handled,
            // silently blocking WeChat's QR authorization window. The native
            // default popup retains its opener and the browser's login session.
            .with_new_window_req_handler(|url, _| {
                if login_popup_allowed(&url) {
                    wry::NewWindowResponse::Allow
                } else {
                    wry::NewWindowResponse::Deny
                }
            })
            .with_ipc_handler(move |request| {
                if site.accepts(&request.uri().to_string())
                    && request.body().len() < 32 * 1024 * 1024
                    && let Ok(value) = serde_json::from_str::<Value>(request.body())
                    && value["kind"] == "response"
                {
                    let _ = sender.send(BrowserEvent::Reply(site, value));
                }
            })
            .with_on_page_load_handler(move |event, url| {
                if matches!(event, wry::PageLoadEvent::Finished) {
                    let _ = page_sender.send(BrowserEvent::Loaded(site, url));
                }
            });
        let raw = builder
            .build_as_child(&*window)
            .context("WebView2 初始化失败，请确认 Microsoft Edge WebView2 Runtime 已安装")?;
        self.browser = Some(cx.new(|cx| WebView::new(raw, window, cx)));
        self._web_context = Some(context);
        Ok(())
    }

    fn send(
        &mut self,
        operation: Operation,
        request: Request,
        cx: &mut gpui_kit::Context<Self>,
    ) -> Result<()> {
        let id = self.next_id;
        self.next_id += 1;
        let script = online::script(id, &request)?;
        let browser = self.browser.as_ref().context("登录窗口尚未就绪")?;
        if !self.site.accepts(&browser.read(cx).raw().url()?) {
            return Err(anyhow!("请先返回 LeetCode 官方页面，再检查登录状态"));
        }
        browser.read(cx).raw().evaluate_script(&script)?;
        self.pending.insert(id, (operation, Instant::now()));
        Ok(())
    }

    fn check_account(&mut self, cx: &mut gpui_kit::Context<Self>) {
        if !self.loaded
            || self
                .pending
                .values()
                .any(|(op, _)| matches!(op, Operation::Account))
        {
            return;
        }
        self.last_account_check = Instant::now();
        if let Err(error) = self.send(Operation::Account, online::account_request(), cx) {
            self.message = error.to_string();
        }
    }

    fn show_browser(&self, show: bool, cx: &mut gpui_kit::Context<Self>) {
        if let Some(browser) = &self.browser {
            browser.update(cx, |view, _| if show { view.show() } else { view.hide() });
        }
    }

    fn tick(&mut self, window: &mut Window, cx: &mut gpui_kit::Context<Self>) {
        if self.probe && self.probe_started.elapsed() > Duration::from_secs(120) {
            println!("ONLINE_CHECK failed: {}", self.message);
            cx.quit();
            return;
        }
        while let Ok(event) = self.events.try_recv() {
            match event {
                BrowserEvent::Loaded(site, url) if site == self.site && site.accepts(&url) => {
                    self.loaded = true;
                    self.check_account(cx);
                }
                BrowserEvent::Reply(site, value) if site == self.site => {
                    if self.probe && value["id"] == 0 {
                        let opened = value["data"]["popup_opened"] == true;
                        println!("ONLINE_CHECK native_popup={opened}");
                        if !opened {
                            cx.quit();
                        } else if let Err(error) =
                            self.send(Operation::List(0), online::list_request(self.site, 0), cx)
                        {
                            println!("ONLINE_CHECK failed: {error}");
                            cx.quit();
                        }
                        continue;
                    }
                    if let Some(id) = value["id"].as_u64()
                        && let Some((operation, _)) = self.pending.remove(&id)
                    {
                        if value["ok"] == true {
                            self.response(operation, value["data"].clone(), window, cx);
                        } else {
                            self.failure(
                                operation,
                                value["error"]
                                    .as_str()
                                    .unwrap_or("LeetCode 请求失败")
                                    .into(),
                                cx,
                            );
                        }
                    }
                }
                _ => {}
            }
        }
        let expired: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, (_, started))| started.elapsed() > Duration::from_secs(35))
            .map(|(id, _)| *id)
            .collect();
        for id in expired {
            if let Some((op, _)) = self.pending.remove(&id) {
                self.failure(
                    op,
                    "网站响应超时，请在官方页面完成登录或验证后重试".into(),
                    cx,
                );
            }
        }
        let mut ready = vec![];
        self.delayed.retain(|(when, op, request)| {
            if *when <= Instant::now() {
                ready.push((op.clone(), request.clone()));
                false
            } else {
                true
            }
        });
        for (op, request) in ready {
            if let Operation::Judge(job) = &op
                && job.cancel.load(Ordering::Relaxed)
            {
                self.judge_result(
                    job,
                    "已停止等待。平台已经接收的测试或提交仍会继续判题。".into(),
                    cx,
                );
                continue;
            }
            if let Err(error) = self.send(op.clone(), request, cx) {
                self.failure(op, error.to_string(), cx);
            }
        }
        if self.phase == Phase::Login
            && self.username.is_none()
            && self.last_account_check.elapsed() > Duration::from_secs(4)
        {
            self.check_account(cx);
        }
    }

    fn response(
        &mut self,
        op: Operation,
        body: Value,
        window: &mut Window,
        cx: &mut gpui_kit::Context<Self>,
    ) {
        if self.probe {
            self.probe_response(op, body, cx);
            return;
        }
        match op {
            Operation::Account => match online::parse_account(&body) {
                Ok(Some(username)) => {
                    let changed = self.username.as_deref() != Some(username.as_str());
                    self.username = Some(username);
                    self.show_browser(false, cx);
                    if changed || self.phase != Phase::Ready {
                        self.load_cached_or_sync(window, cx);
                    }
                }
                Ok(None) => {
                    self.username = None;
                    self.phase = Phase::Login;
                    self.message =
                        "请在官方页面完成登录；已有缓存会直接打开，首次登录才拉取题库。".into();
                    self.show_browser(true, cx);
                    if let Some(browser) = &self.browser {
                        let login = format!("{}/accounts/login/", self.site.origin());
                        if browser
                            .read(cx)
                            .raw()
                            .url()
                            .is_ok_and(|url| !url.contains("/accounts/login"))
                        {
                            browser.update(cx, |view, _| view.load_url(&login));
                        }
                    }
                }
                Err(error) => self.failure(Operation::Account, error.to_string(), cx),
            },
            Operation::List(skip) => match online::parse_page(self.site, &body) {
                Ok(page) => {
                    if !self.matches_account(&body) {
                        self.failure(
                            Operation::Account,
                            "登录会话已经变化，请重新连接账号".into(),
                            cx,
                        );
                        return;
                    }
                    self.expected_total = page.total;
                    let added = page.problems.len();
                    if (added == 0 && self.questions.len() < page.total)
                        || page.problems.iter().any(|p| {
                            self.questions
                                .iter()
                                .any(|existing| existing.slug == p.slug)
                        })
                    {
                        self.failure(Operation::List(skip), "分页结果不完整".into(), cx);
                        return;
                    }
                    self.questions.extend(page.problems);
                    self.message = format!(
                        "正在同步题库与通过状态：{}/{}",
                        self.questions.len(),
                        self.expected_total
                    );
                    if added > 0 && self.questions.len() < self.expected_total {
                        if let Err(error) = self.send(
                            Operation::List(skip + added),
                            online::list_request(self.site, skip + added),
                            cx,
                        ) {
                            self.failure(Operation::List(skip), error.to_string(), cx);
                        }
                    } else {
                        self.finish_sync(window, cx);
                    }
                }
                Err(error) => self.failure(Operation::List(skip), error.to_string(), cx),
            },
            Operation::Legacy => match online::parse_legacy(self.site, &body) {
                Ok(page) => {
                    self.questions = page.problems;
                    self.expected_total = page.total;
                    if let Err(error) =
                        self.send(Operation::VerifyIndex, online::account_request(), cx)
                    {
                        self.failure(Operation::Account, error.to_string(), cx);
                    }
                }
                Err(error) => self.failure(Operation::Legacy, error.to_string(), cx),
            },
            Operation::VerifyIndex => match online::parse_account(&body) {
                Ok(Some(username)) if self.username.as_deref() == Some(username.as_str()) => {
                    self.finish_sync(window, cx)
                }
                _ => self.failure(
                    Operation::Account,
                    "题库同步时登录会话已变化，请重新连接".into(),
                    cx,
                ),
            },
            Operation::StudyPlan => {
                if !self.matches_account(&body) {
                    self.failure(
                        Operation::Account,
                        "同步题单时账号已变化，请重新连接".into(),
                        cx,
                    );
                    return;
                }
                let result = online::parse_study_plan(&body).and_then(|plan| {
                    Store::open(self.root.clone())?.save_study_plan(self.site, &plan)?;
                    if let Some(workspace) = &self.workspace {
                        workspace.update(cx, |this, cx| this.update_study_plan(plan, cx));
                    }
                    Ok(())
                });
                if let Err(error) = result {
                    self.failure(Operation::StudyPlan, format!("{error:#}"), cx);
                }
            }
            Operation::Detail(problem) => match online::parse_detail(&problem, &body) {
                Ok(detail) => {
                    if !self.matches_account(&body) {
                        self.failure(
                            Operation::Account,
                            "登录会话已经变化，请重新连接账号".into(),
                            cx,
                        );
                        return;
                    }
                    if let Some(workspace) = &self.workspace {
                        workspace.update(cx, |this, cx| {
                            if let Err(error) = this.install_detail(detail, window, cx) {
                                this.online_error(error.to_string(), cx);
                            }
                        });
                    }
                }
                Err(error) => self.failure(Operation::Detail(problem), error.to_string(), cx),
            },
            Operation::AuthorizeJudge { job, request } => {
                if online::parse_account(&body).ok().flatten().as_deref() != Some(&job.owner)
                    || self.username.as_deref() != Some(&job.owner)
                    || self.site != job.site
                    || self.phase != Phase::Ready
                {
                    self.failure(
                        Operation::Judge(job),
                        "登录账号已变化，请重新登录后操作".into(),
                        cx,
                    );
                } else if job.cancel.load(Ordering::Relaxed) {
                    self.judge_result(&job, "已取消，未发送测试或提交。".into(), cx);
                } else if let Err(error) = self.send(Operation::Judge(job.clone()), request, cx) {
                    self.failure(Operation::Judge(job), error.to_string(), cx);
                }
            }
            Operation::Judge(job) => self.judge_response(*job, body, cx),
        }
        cx.notify();
    }

    fn judge_result(&self, job: &JudgeJob, output: String, cx: &mut gpui_kit::Context<Self>) {
        if let Some(workspace) = &self.workspace {
            workspace.update(cx, |this, cx| {
                this.online_result(job.problem_id, job.generation, &job.source, output, cx)
            });
        }
    }

    fn judge_response(&mut self, mut job: JudgeJob, body: Value, cx: &mut gpui_kit::Context<Self>) {
        if job.cancel.load(Ordering::Relaxed) {
            self.judge_result(
                &job,
                "已停止等待。平台已经接收的测试或提交仍会继续判题。".into(),
                cx,
            );
            return;
        }
        if job.task.is_none() {
            match online::judge_id(&body, job.submit) {
                Ok(id) => job.task = Some(id),
                Err(error) => {
                    self.failure(Operation::Judge(Box::new(job)), format!("{error:#}"), cx);
                    return;
                }
            }
        } else if online::judge_finished(&body) {
            let mut output = if job.submit {
                online::submission_output(&body)
            } else {
                online::run_output(&body)
            };
            if job.submit
                && body["status_code"].as_i64().is_some()
                && self.username.as_deref() == Some(&job.owner)
                && self.site == job.site
                && let Some(workspace) = &self.workspace
            {
                match workspace.update(cx, |this, cx| {
                    this.record_submission(
                        job.problem_id,
                        &job.owner,
                        online::submission_accepted(&body),
                        cx,
                    )
                }) {
                    Ok(solved) => self.solved = solved,
                    Err(error) => output.push_str(&format!("\n状态缓存更新失败：{error}")),
                }
            }
            self.judge_result(&job, output, cx);
            return;
        }
        if job.started.elapsed() > Duration::from_secs(90) {
            self.failure(
                Operation::Judge(Box::new(job)),
                "等待判题超时；已接收的提交不会自动重发".into(),
                cx,
            );
            return;
        }
        match online::check_request(job.task.as_deref().unwrap()) {
            Ok(request) => self.delayed.push((
                Instant::now() + Duration::from_millis(800),
                Operation::Judge(Box::new(job)),
                request,
            )),
            Err(error) => self.failure(Operation::Judge(Box::new(job)), error.to_string(), cx),
        }
    }

    fn probe_response(&mut self, op: Operation, body: Value, cx: &mut gpui_kit::Context<Self>) {
        let result = (|| -> Result<()> {
            match op {
                Operation::Account => {
                    let signed = online::parse_account(&body)?.is_some();
                    println!("ONLINE_CHECK browser_session signed_in={signed}");
                    if self.plan_probe {
                        self.send(Operation::StudyPlan, online::study_plan_request(), cx)?;
                        return Ok(());
                    }
                    self.browser.as_ref().context("浏览器未就绪")?.read(cx).raw()
                        .evaluate_script("(()=>{const popup=window.open('about:blank','leetdesk-popup-check','width=480,height=400');setTimeout(()=>{const opened=!!popup&&!popup.closed;if(opened)popup.close();window.ipc.postMessage(JSON.stringify({kind:'response',id:0,ok:true,data:{popup_opened:opened}}));},1200);})();")?;
                }
                Operation::List(_) | Operation::Legacy => {
                    let page = if matches!(op, Operation::Legacy) {
                        online::parse_legacy(self.site, &body)?
                    } else {
                        match online::parse_page(self.site, &body) {
                            Ok(page) => page,
                            Err(_) => {
                                self.send(Operation::Legacy, online::legacy_list_request(), cx)?;
                                return Ok(());
                            }
                        }
                    };
                    self.expected_total = page.total;
                    if page.problems.is_empty() && self.questions.len() < page.total {
                        return Err(anyhow!("题库分页意外为空"));
                    }
                    if page.problems.iter().any(|p| {
                        self.questions
                            .iter()
                            .any(|existing| existing.slug == p.slug)
                    }) {
                        return Err(anyhow!("题库分页返回了重复题目"));
                    }
                    self.questions.extend(page.problems);
                    self.message = format!(
                        "已验证题库分页 {}/{}",
                        self.questions.len(),
                        self.expected_total
                    );
                    if self.questions.len().is_multiple_of(500) {
                        println!(
                            "ONLINE_CHECK progress={}/{}",
                            self.questions.len(),
                            self.expected_total
                        );
                    }
                    if self.questions.len() < self.expected_total {
                        self.send(
                            Operation::List(self.questions.len()),
                            online::list_request(self.site, self.questions.len()),
                            cx,
                        )?;
                        return Ok(());
                    }
                    println!(
                        "ONLINE_CHECK catalogue={} downloaded_rows={}",
                        self.expected_total,
                        self.questions.len()
                    );
                    let problem = self
                        .questions
                        .iter()
                        .find(|p| p.slug == "two-sum")
                        .or(self.questions.first())
                        .context("网站没有返回题目")?
                        .clone();
                    self.send(
                        Operation::Detail(Box::new(problem.clone())),
                        online::detail_request(self.site, &problem.slug)?,
                        cx,
                    )?;
                }
                Operation::StudyPlan => {
                    let plan = online::parse_study_plan(&body)?;
                    Store::open(self.root.clone())?.save_study_plan(self.site, &plan)?;
                    println!(
                        "STUDY_PLAN_CHECK groups={} questions={}",
                        plan.groups.len(),
                        plan.groups
                            .iter()
                            .map(|group| group.questions.len())
                            .sum::<usize>()
                    );
                    self.probe_result.store(true, Ordering::Relaxed);
                    cx.quit();
                }
                Operation::Detail(problem) => {
                    let detail = online::parse_detail(&problem, &body)?;
                    let go = detail.online.as_ref().is_some_and(|m| m.go_supported);
                    println!("ONLINE_CHECK detail={} go_template={go}", detail.slug);
                    self.probe_result.store(go, Ordering::Relaxed);
                    cx.quit();
                }
                _ => {}
            }
            Ok(())
        })();
        if let Err(error) = result {
            println!("ONLINE_CHECK failed: {error}");
            cx.quit();
        }
    }

    fn failure(&mut self, op: Operation, message: String, cx: &mut gpui_kit::Context<Self>) {
        if self.probe {
            println!("ONLINE_CHECK failed: {message}");
            cx.quit();
            return;
        }
        match op {
            Operation::StudyPlan => {
                if let Some(workspace) = &self.workspace {
                    workspace.update(cx, |this, cx| {
                        this.online_error(
                            format!("题库已同步；热题 100 更新失败，保留缓存：{message}"),
                            cx,
                        )
                    });
                }
            }
            Operation::List(_) => {
                self.questions.clear();
                self.message = "正在使用兼容接口同步题库…".into();
                if let Err(error) = self.send(Operation::Legacy, online::legacy_list_request(), cx)
                {
                    self.phase = Phase::Error;
                    self.message = error.to_string();
                }
            }
            Operation::Detail(_) => {
                if let Some(workspace) = &self.workspace {
                    workspace.update(cx, |this, cx| {
                        this.online_error(
                            format!("题面加载失败：{message}。再次点击当前题目可重试。"),
                            cx,
                        )
                    });
                }
            }
            Operation::Judge(job) | Operation::AuthorizeJudge { job, .. } => {
                let label = if job.submit { "提交" } else { "测试" };
                let mut output = format!("{label}请求失败：{message}");
                if job.submit {
                    output.push_str("\n若平台已接收提交，请先到题目网页查看记录，避免重复提交。");
                }
                self.judge_result(&job, output, cx);
            }
            Operation::Account | Operation::VerifyIndex => {
                self.phase = Phase::Login;
                self.username = None;
                self.message = format!("无法验证登录状态：{message}");
                self.show_browser(true, cx);
            }
            Operation::Legacy => {
                self.phase = Phase::Error;
                self.message = format!("同步失败：{message}");
                self.show_browser(true, cx);
            }
        }
        cx.notify();
    }

    fn matches_account(&self, body: &Value) -> bool {
        online::parse_account(body)
            .is_ok_and(|username| username.is_some() && username == self.username)
    }

    fn load_cached_or_sync(&mut self, window: &mut Window, cx: &mut gpui_kit::Context<Self>) {
        let result = (|| -> Result<bool> {
            let store = Store::open(self.root.clone())?;
            let Some(mut cache) = store.online_cache(self.site)? else {
                return Ok(false);
            };
            if cache.problems.is_empty() || self.username.as_deref() != Some(&cache.username) {
                return Ok(false);
            }
            // Activating a cached site also preserves downloaded details and the
            // previous sync timestamp; it does not fetch a new question index.
            store.save_online_cache(&mut cache)?;
            self.solved = cache
                .problems
                .iter()
                .filter(|p| online::status_label(p) == "已通过")
                .count();
            self.catalogue_total = cache.problems.len();
            if let Some(workspace) = &self.workspace {
                workspace.update(cx, |this, cx| {
                    this.merge_online_index(cache.problems, cx)?;
                    this.set_online_enabled(true);
                    Ok::<_, anyhow::Error>(())
                })?;
            } else {
                let view =
                    cx.new(|cx| Workspace::new(store, window, cx).expect("无法打开缓存工作区"));
                self.attach_workspace(view, window, cx, true);
            }
            self.phase = Phase::Ready;
            self.message = format!(
                "已载入 {} 道缓存题目；点击“同步题库”更新",
                self.catalogue_total
            );
            self.show_browser(false, cx);
            Ok(true)
        })();
        match result {
            Ok(true) => {}
            Ok(false) => self.start_sync(cx),
            Err(error) => {
                self.phase = Phase::Error;
                self.message = format!("缓存读取失败：{error}，请点击同步题库重试");
                self.show_browser(true, cx);
            }
        }
        cx.notify();
    }

    fn start_sync(&mut self, cx: &mut gpui_kit::Context<Self>) {
        if self
            .pending
            .values()
            .any(|(op, _)| matches!(op, Operation::List(_) | Operation::Legacy))
        {
            return;
        }
        if let Some(workspace) = &self.workspace {
            workspace.update(cx, |this, cx| {
                let _ = this.flush(cx);
            });
        }
        self.phase = Phase::Syncing;
        self.questions.clear();
        self.expected_total = 0;
        self.message = "已登录，正在同步题库与已通过状态…".into();
        self.show_browser(false, cx);
        if let Err(error) = self.send(Operation::List(0), online::list_request(self.site, 0), cx) {
            self.failure(Operation::List(0), error.to_string(), cx);
        }
    }

    fn finish_sync(&mut self, window: &mut Window, cx: &mut gpui_kit::Context<Self>) {
        let result = (|| -> Result<()> {
            if self.questions.is_empty() {
                return Err(anyhow!("网站没有返回题目，请完成验证后重试"));
            }
            let store = Store::open(self.root.clone())?;
            let mut cache = OnlineCache {
                site: self.site,
                username: self.username.clone().context("尚未验证登录账号")?,
                problems: std::mem::take(&mut self.questions),
                synced_at: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
            };
            store.save_online_cache(&mut cache)?;
            self.solved = cache
                .problems
                .iter()
                .filter(|p| online::status_label(p) == "已通过")
                .count();
            self.catalogue_total = cache.problems.len();
            if let Some(workspace) = &self.workspace {
                workspace.update(cx, |this, cx| {
                    this.merge_online_index(cache.problems, cx)?;
                    this.set_online_enabled(true);
                    Ok::<_, anyhow::Error>(())
                })?;
            } else {
                let view =
                    cx.new(|cx| Workspace::new(store, window, cx).expect("无法打开在线工作区"));
                self.attach_workspace(view, window, cx, true);
            }
            self.phase = Phase::Ready;
            self.message = format!(
                "同步完成 · {} 道题目 · {} 道已通过",
                self.catalogue_total, self.solved
            );
            self.show_browser(false, cx);
            if let Err(error) = self.send(Operation::StudyPlan, online::study_plan_request(), cx) {
                self.failure(Operation::StudyPlan, error.to_string(), cx);
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.phase = Phase::Error;
            self.message = error.to_string();
            self.show_browser(true, cx);
        }
    }

    fn attach_workspace(
        &mut self,
        view: Entity<Workspace>,
        window: &mut Window,
        cx: &mut gpui_kit::Context<Self>,
        enabled: bool,
    ) {
        self._subscription = Some(cx.subscribe(&view, |this, _, event, cx| match event {
            WorkspaceEvent::LoadDetail(problem) => {
                if this.phase == Phase::Offline {
                    return;
                }
                if this
                    .pending
                    .values()
                    .any(|(op, _)| matches!(op,Operation::Detail(p) if p.slug==problem.slug))
                {
                    return;
                }
                match online::detail_request(this.site, &problem.slug) {
                    Ok(request) => {
                        if let Err(error) =
                            this.send(Operation::Detail(Box::new(problem.clone())), request, cx)
                        {
                            this.failure(
                                Operation::Detail(Box::new(problem.clone())),
                                error.to_string(),
                                cx,
                            );
                        }
                    }
                    Err(error) => this.failure(
                        Operation::Detail(Box::new(problem.clone())),
                        error.to_string(),
                        cx,
                    ),
                }
            }
            WorkspaceEvent::Run {
                problem,
                source,
                input,
                generation,
                cancel,
                submit,
            } => {
                let job = Box::new(JudgeJob {
                    problem_id: problem.id,
                    source: source.clone(),
                    generation: *generation,
                    cancel: cancel.clone(),
                    task: None,
                    started: Instant::now(),
                    submit: *submit,
                    owner: this.username.clone().unwrap_or_default(),
                    site: this.site,
                });
                if this.phase != Phase::Ready || job.owner.is_empty() {
                    this.failure(
                        Operation::Judge(job),
                        "请连接 LeetCode 账号后再操作".into(),
                        cx,
                    );
                    return;
                }
                let request = if *submit {
                    online::submit_request(problem, source)
                } else {
                    online::run_request(problem, source, input)
                };
                match request {
                    Ok(request) => {
                        let op = Operation::AuthorizeJudge { job, request };
                        if let Err(error) = this.send(op.clone(), online::account_request(), cx) {
                            this.failure(op, error.to_string(), cx);
                        }
                    }
                    Err(error) => this.failure(Operation::Judge(job), error.to_string(), cx),
                }
            }
        }));
        self.workspace = Some(view.clone());
        view.update(cx, |this, cx| {
            this.set_online_enabled(enabled);
            this.request_current(window, cx);
        });
    }

    pub fn flush(&mut self, cx: &mut gpui_kit::Context<Self>) -> bool {
        if let Some(workspace) = &self.workspace
            && let Err(error) = workspace.update(cx, |this, cx| this.flush(cx))
        {
            self.message = format!("保存失败：{error}");
            cx.notify();
            return false;
        }
        true
    }

    fn offline(&mut self, window: &mut Window, cx: &mut gpui_kit::Context<Self>) {
        let result = (|| -> Result<()> {
            let store = Store::open(self.root.clone())?;
            let mut cache = store
                .online_cache(self.site)?
                .context("还没有在线题库缓存，请先登录同步")?;
            if cache.problems.is_empty() {
                return Err(anyhow!("题库缓存为空"));
            }
            self.catalogue_total = cache.problems.len();
            self.solved = cache
                .problems
                .iter()
                .filter(|p| online::status_label(p) == "已通过")
                .count();
            self.phase = Phase::Offline;
            self.username = None;
            self.pending.clear();
            self.delayed.clear();
            store.save_online_cache(&mut cache)?;
            let view =
                cx.new(|cx| Workspace::new(store, window, cx).expect("缓存工作区初始化失败"));
            self.attach_workspace(view, window, cx, false);
            self.show_browser(false, cx);
            self.message = format!("离线缓存 · 上次同步账号 {}，状态可能已变化", cache.username);
            Ok(())
        })();
        if let Err(error) = result {
            self.message = error.to_string();
        }
        cx.notify();
    }
}

impl Render for AccountShell {
    fn render(&mut self, _: &mut Window, cx: &mut gpui_kit::Context<Self>) -> impl IntoElement {
        let editing = self.workspace.is_some()
            && matches!(self.phase, Phase::Ready | Phase::Offline | Phase::Syncing);
        let account = self.username.clone().unwrap_or_else(|| {
            if self.phase == Phase::Offline {
                "离线缓存".into()
            } else {
                "等待登录".into()
            }
        });
        let mut header = div()
            .h(px(if editing { 36. } else { 62. }))
            .px_4()
            .flex()
            .items_center()
            .gap_3()
            .border_b_1()
            .border_color(rgb(0x263247))
            .child(
                div()
                    .text_color(rgb(0x78ddbd))
                    .font_weight(FontWeight::BOLD)
                    .child("LeetDesk"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(0x8b9ab1))
                    .child(format!("{} · {account}", self.site.label())),
            );
        if editing {
            header = header
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(0x78ddbd))
                        .child(format!("已通过 {} / {}", self.solved, self.catalogue_total)),
                )
                .child(div().flex_1())
                .child(
                    Button::new("sync-account")
                        .label(if self.phase == Phase::Syncing {
                            "同步中…"
                        } else {
                            "同步题库"
                        })
                        .small()
                        .ghost()
                        .on_click(cx.listener(|this, _, _, cx| {
                            if this.phase == Phase::Syncing {
                                return;
                            }
                            if this.phase == Phase::Ready {
                                this.start_sync(cx);
                            } else {
                                this.phase = Phase::Login;
                                this.show_browser(true, cx);
                                this.check_account(cx);
                            }
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("account-page")
                        .label("账号")
                        .small()
                        .ghost()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.phase = Phase::Login;
                            this.show_browser(true, cx);
                            cx.notify();
                        })),
                );
        } else {
            header =
                header
                    .child(div().flex_1())
                    .children([Site::China, Site::Global].into_iter().map(|site| {
                        Button::new(site.key())
                            .label(site.label())
                            .small()
                            .ghost()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if this.site != site {
                                    if !this.flush(cx) {
                                        return;
                                    }
                                    this.site = site;
                                    this.username = None;
                                    this.workspace = None;
                                    this._subscription = None;
                                    this.phase = Phase::Checking;
                                    this.message = "正在连接所选站点…".into();
                                    let _ = fs::write(
                                        this.root.join("site.json"),
                                        serde_json::to_vec(&site).unwrap(),
                                    );
                                    if let Err(error) = this.build_browser(window, cx) {
                                        this.message = error.to_string();
                                        this.phase = Phase::Error;
                                    }
                                    cx.notify();
                                }
                            }))
                    }));
        }
        let mut root = div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x0e1420))
            .text_color(rgb(0xdce5f2))
            .font_family("Microsoft YaHei")
            .child(header);
        if editing {
            root = root.child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(self.workspace.as_ref().unwrap().clone()),
            );
        } else {
            let controls = div()
                .flex()
                .gap_2()
                .child(
                    Button::new("sync-from-login")
                        .label("同步题库")
                        .ghost()
                        .on_click(cx.listener(|this, _, _, cx| {
                            if this.username.is_some() {
                                this.start_sync(cx);
                            } else {
                                this.check_account(cx);
                            }
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("check-login")
                        .label("我已登录，开始同步")
                        .primary()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.check_account(cx);
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("offline-cache")
                        .label("使用离线缓存")
                        .ghost()
                        .on_click(cx.listener(|this, _, window, cx| this.offline(window, cx))),
                );
            root = root.child(
                div()
                    .px_5()
                    .py_4()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(div().text_lg().child(if self.phase == Phase::Syncing {
                                "正在准备你的题库"
                            } else {
                                "连接你的 LeetCode 账号"
                            }))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x8b9ab1))
                                    .child(self.message.clone()),
                            ),
                    )
                    .child(controls),
            );
            if matches!(self.phase, Phase::Login | Phase::Checking | Phase::Error) {
                root = root.child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .when_some(self.browser.clone(), |el, browser| el.child(browser)),
                );
            } else {
                root = root.child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(rgb(0x78ddbd))
                        .child(self.message.clone()),
                );
            }
            root = root.child(div().px_5().py_2().text_xs().text_color(rgb(0x8b9ab1)).child("在官方页面自行完成登录。登录会话由此设备的浏览器保存，程序不读取你的密码或会话 Cookie。"));
        }
        root
    }
}

pub fn launch(store: Store, probe: bool, plan_probe: bool) -> Result<()> {
    let root = store.root.clone();
    drop(store);
    let probe_result = Arc::new(AtomicBool::new(false));
    let result = probe_result.clone();
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            crate::desktop::apply_theme(cx);
            crate::desktop::bind_keys(cx);
            let (_, view) = gpui_kit::open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: Some("LeetDesk · LeetCode Go 工作区".into()),
                        ..Default::default()
                    }),
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(1480.), px(960.)),
                        cx,
                    ))),
                    window_min_size: Some(size(px(1120.), px(720.))),
                    app_id: Some("leetdesk".into()),
                    show: !probe,
                    ..Default::default()
                },
                cx,
                move |window, cx| {
                    cx.new(|cx| {
                        let mut shell = AccountShell::new(root, window, cx);
                        shell.probe = probe;
                        shell.plan_probe = plan_probe;
                        shell.probe_result = probe_result;
                        shell
                    })
                },
            )
            .expect("无法打开登录工作区");
            if let Some(handle) = cx.windows().last().copied() {
                let _ = handle.update(cx, |_, window, cx| {
                    let closing = view.downgrade();
                    window.on_window_should_close(cx, move |_, cx| {
                        closing
                            .update(cx, |this, cx| this.flush(cx))
                            .unwrap_or(true)
                    });
                });
            }
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit()
                }
            })
            .detach();
            if !probe {
                cx.activate(true);
            }
        });
    if probe && !result.load(Ordering::Relaxed) {
        return Err(anyhow!("LeetCode 原生浏览器接口验证未通过"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use gpui_kit::AppContext;
    #[gpui_kit::test]
    fn signed_in_startup_uses_cached_catalogue_without_scheduling_sync(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use super::*;
        use serde_json::json;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("data");
        let store = Store::open(root.clone()).unwrap();
        let mut problem = online::parse_page(Site::China, &json!({"data":{"problemsetQuestionList":{"total":1,"questions":[{"frontendQuestionId":"1","title":"Two Sum","titleSlug":"two-sum","status":"ac"}]}}})).unwrap().problems.remove(0);
        problem.starter =
            "package solution\nfunc twoSum(nums []int, target int) []int { return nil }".into();
        problem.online.as_mut().unwrap().loaded = true;
        problem.online.as_mut().unwrap().go_supported = true;
        let mut cache = OnlineCache {
            site: Site::China,
            username: "alice".into(),
            problems: vec![problem],
            synced_at: 123,
        };
        store.save_online_cache(&mut cache).unwrap();
        drop(store);
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::desktop::apply_theme(cx);
        });
        let (handle, view) = cx.update(|cx| {
            gpui_kit::open_window(Default::default(), cx, |_, cx| {
                cx.new(|cx| AccountShell::blank(root.clone(), cx))
            })
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |shell, cx| {
                shell.response(
                    Operation::Account,
                    json!({"data":{"userStatus":{"isSignedIn":true,"username":"alice"}}}),
                    window,
                    cx,
                );
                assert!(matches!(shell.phase, Phase::Ready));
                assert!(shell.workspace.is_some());
                assert_eq!(shell.catalogue_total, 1);
                assert_eq!(shell.solved, 1);
                assert!(shell.pending.is_empty());
                shell.last_account_check = Instant::now() - Duration::from_secs(600);
                shell.tick(window, cx);
                assert!(
                    shell.pending.is_empty(),
                    "有缓存的工作区不会自动安排联网同步"
                );
            });
        })
        .unwrap();
        assert_eq!(
            Store::open(root)
                .unwrap()
                .online_cache(Site::China)
                .unwrap()
                .unwrap()
                .synced_at,
            123
        );
    }
    #[gpui_kit::test]
    fn judge_polling_never_reposts_and_stops_on_terminal_cancel_or_timeout(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use super::*;
        use serde_json::json;
        let temp = tempfile::tempdir().unwrap();
        cx.update(gpui_kit::init);
        let (handle, view) = cx.update(|cx| {
            gpui_kit::open_window(Default::default(), cx, |_, cx| {
                cx.new(|cx| AccountShell::blank(temp.path().join("data"), cx))
            })
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |shell, cx| {
                shell.username = Some("alice".into());
                shell.phase = Phase::Ready;
                let job = JudgeJob {
                    problem_id: 1,
                    source: "package solution".into(),
                    generation: 1,
                    cancel: Arc::new(AtomicBool::new(false)),
                    task: None,
                    started: Instant::now(),
                    submit: true,
                    owner: "alice".into(),
                    site: Site::China,
                };
                shell.response(
                    Operation::Judge(Box::new(job.clone())),
                    json!({"submission_id":123}),
                    window,
                    cx,
                );
                assert_eq!(shell.delayed.len(), 1);
                let (_, poll, request) = shell.delayed.pop().unwrap();
                assert_eq!(request.path, "/submissions/detail/123/check/");
                assert!(request.body.is_none());
                shell.response(poll, json!({"state":"PENDING"}), window, cx);
                let (_, poll, request) = shell.delayed.pop().unwrap();
                assert!(request.body.is_none());
                shell.response(
                    poll,
                    json!({"state":"SUCCESS","status_code":10}),
                    window,
                    cx,
                );
                assert!(shell.delayed.is_empty());
                let mut expired = job.clone();
                let mut test_job = job.clone();
                test_job.submit = false;
                shell.response(
                    Operation::Judge(Box::new(test_job)),
                    json!({"interpret_id":"runcode_1627219627.5662382_EI7iasnhLm"}),
                    window,
                    cx,
                );
                let (_, poll, request) =
                    shell.delayed.pop().expect("带小数点的测试任务必须进入轮询");
                assert_eq!(
                    request.path,
                    "/submissions/detail/runcode_1627219627.5662382_EI7iasnhLm/check/"
                );
                assert!(request.body.is_none());
                shell.response(poll, json!({"state":"PENDING"}), window, cx);
                let (_, poll, request) = shell.delayed.pop().unwrap();
                assert!(request.body.is_none(), "只轮询，不能重发测试 POST");
                shell.response(
                    poll,
                    json!({"state":"SUCCESS","status_code":10}),
                    window,
                    cx,
                );
                assert!(shell.delayed.is_empty());
                expired.task = Some("123".into());
                expired.started = Instant::now() - Duration::from_secs(91);
                shell.response(
                    Operation::Judge(Box::new(expired)),
                    json!({"state":"PENDING"}),
                    window,
                    cx,
                );
                assert!(shell.delayed.is_empty());
                job.cancel.store(true, Ordering::Relaxed);
                shell.response(
                    Operation::Judge(Box::new(job.clone())),
                    json!({"submission_id":123}),
                    window,
                    cx,
                );
                assert!(shell.delayed.is_empty());
                let request = Request {
                    path: "/problems/two-sum/submit/".into(),
                    body: Some(json!({})),
                };
                shell.response(
                    Operation::AuthorizeJudge {
                        job: Box::new(job),
                        request,
                    },
                    json!({"data":{"userStatus":{"isSignedIn":true,"username":"bob"}}}),
                    window,
                    cx,
                );
                assert!(shell.pending.is_empty());
            });
        })
        .unwrap();
    }
    #[gpui_kit::test]
    fn unauthenticated_or_changed_account_cannot_install_a_catalogue(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use super::{AccountShell, Operation, Phase};
        use serde_json::json;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("data");
        cx.update(gpui_kit::init);
        let (handle, view) = cx.update(|cx| {
            gpui_kit::open_window(Default::default(), cx, |_, cx| {
                cx.new(|cx| AccountShell::blank(root.clone(), cx))
            })
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |shell, cx| {
                shell.response(Operation::Account, json!({"data":{"userStatus":{"isSignedIn":false}}}), window, cx);
                assert!(matches!(shell.phase, Phase::Login));
                assert!(shell.workspace.is_none());
                shell.username = Some("alice".into());
                shell.phase = Phase::Syncing;
                shell.response(Operation::List(0), json!({"data":{"userStatus":{"isSignedIn":true,"username":"bob"},"problemsetQuestionList":{"total":1,"questions":[{"frontendQuestionId":"1","title":"Two Sum","titleSlug":"two-sum","status":"ac"}]}}}), window, cx);
                assert!(shell.username.is_none());
                assert!(shell.workspace.is_none());
                assert!(shell.questions.is_empty());
                assert!(matches!(shell.phase, Phase::Login));
            });
        }).unwrap();
        assert!(!root.join("leetcode/cn/index.json").exists());
    }

    #[test]
    fn oauth_windows_and_blank_opener_are_allowed() {
        assert!(super::login_popup_allowed(
            "https://open.weixin.qq.com/connect/qrconnect?appid=test"
        ));
        assert!(super::login_popup_allowed(
            "https://leetcode.cn/accounts/weixin/login/"
        ));
        assert!(super::login_popup_allowed("about:blank"));
        for url in [
            "javascript:alert(1)",
            "file:///C:/secret.txt",
            "http://example.com",
            "invalid",
        ] {
            assert!(!super::login_popup_allowed(url));
        }
    }
}
