//! A small stdio LSP client. The desktop layer supplies GPUI editor providers.
use crate::catalog::PracticeLanguage;
use anyhow::{Context, Result, anyhow, bail};
use lsp_types::{Diagnostic, Position};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs,
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

type Pending = Arc<Mutex<HashMap<u64, mpsc::Sender<Result<Value>>>>>;

#[derive(Clone, Debug, Default)]
pub struct Diagnostics {
    pub version: Option<i32>,
    pub items: Vec<Diagnostic>,
    pub generation: u64,
}

struct Inner {
    process: Mutex<Child>,
    writer: Arc<Mutex<ChildStdin>>,
    pending: Pending,
    diagnostics: Arc<Mutex<HashMap<String, Diagnostics>>>,
    error: Arc<Mutex<Option<String>>>,
    next_id: AtomicU64,
    document: Mutex<Document>,
    uri: String,
    language: PracticeLanguage,
    name: &'static str,
    inlay_hints: bool,
    // Keep each Java session isolated, including rapid language/problem switches.
    _project: Option<tempfile::TempDir>,
}

#[derive(Default)]
struct Document {
    text: String,
    version: i32,
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Ok(mut child) = self.process.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[derive(Clone)]
pub struct LanguageService {
    inner: Arc<Inner>,
}

pub type GoLanguage = LanguageService;

impl LanguageService {
    pub fn start(workspace: &Path, source: &str) -> Result<Self> {
        Self::start_for(workspace, source, PracticeLanguage::Go)
    }

    pub fn start_for(workspace: &Path, source: &str, language: PracticeLanguage) -> Result<Self> {
        let workspace = process_path(workspace.canonicalize()?);
        let name = match language {
            PracticeLanguage::Go => "gopls",
            PracticeLanguage::Java => "JDT LS",
        };
        let project = if language == PracticeLanguage::Java {
            Some(prepare_java_project(&workspace, source)?)
        } else {
            None
        };
        let root = project
            .as_ref()
            .map_or_else(|| workspace.clone(), |p| p.path().join("project"));
        let uri = file_uri(&root.join(language.filename()))?;
        let log = fs::File::create(workspace.join(format!("{name}.log")))?;
        let mut command = if language == PracticeLanguage::Java {
            java_server_command(&root)?
        } else {
            let cache = workspace
                .parent()
                .context("工作区路径无效")?
                .join(".go-cache");
            fs::create_dir_all(&cache)?;
            let mut command = tool_command(gopls_executable());
            command
                .current_dir(&root)
                .arg("serve")
                .env("GOCACHE", &cache)
                .env("GOTOOLCHAIN", "local");
            command
        };
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(log));
        hide_console(&mut command);
        let mut process = command.spawn().with_context(|| match language {
            PracticeLanguage::Go => "无法启动 gopls；请安装 Go 和 gopls，并把它们加入 PATH",
            PracticeLanguage::Java => {
                "无法启动 JDT LS；请安装 Java 25+ 和 JDT LS，设置 LEETDESK_JDTLS_HOME 指向解压目录"
            }
        })?;
        let writer = Arc::new(Mutex::new(process.stdin.take().context("LSP stdin")?));
        let stdout = process.stdout.take().context("LSP stdout")?;
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let diagnostics: Arc<Mutex<HashMap<String, Diagnostics>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let error = Arc::new(Mutex::new(None));
        let (thread_writer, thread_pending, thread_diagnostics, thread_error) = (
            writer.clone(),
            pending.clone(),
            diagnostics.clone(),
            error.clone(),
        );
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let result = (|| -> Result<()> {
                loop {
                    let message = read_message(&mut reader)?;
                    if let Some(method) = message.get("method").and_then(Value::as_str) {
                        let params = &message["params"];
                        if method == "textDocument/publishDiagnostics"
                            && let (Some(uri), Ok(items)) = (
                                params["uri"].as_str(),
                                serde_json::from_value::<Vec<Diagnostic>>(
                                    params["diagnostics"].clone(),
                                ),
                            )
                        {
                            let mut items = items;
                            if language == PracticeLanguage::Java {
                                items.retain(|d| d.range.start.line >= JAVA_PREFIX_LINES);
                                for item in &mut items {
                                    unmap_range(&mut item.range);
                                }
                            }
                            let mut values = thread_diagnostics.lock().unwrap();
                            let uri = diagnostic_uri(uri);
                            let previous = values.get(&uri).map_or(0, |d| d.generation);
                            values.insert(
                                uri,
                                Diagnostics {
                                    version: params["version"].as_i64().map(|v| v as i32),
                                    items,
                                    generation: previous + 1,
                                },
                            );
                        }
                        if let Some(id) = message.get("id") {
                            let result = match method {
                                "workspace/configuration" => Value::Array(
                                    params["items"].as_array().map_or(vec![], |items| {
                                        items
                                            .iter()
                                            .map(|item| {
                                                configuration(language, item["section"].as_str())
                                            })
                                            .collect()
                                    }),
                                ),
                                "workspace/applyEdit" => {
                                    json!({"applied":false, "failureReason":"Use editor formatting or completion edits"})
                                }
                                _ => Value::Null,
                            };
                            write_message(
                                &mut *thread_writer.lock().unwrap(),
                                &json!({"jsonrpc":"2.0", "id":id, "result":result}),
                            )?;
                        }
                    } else if let Some(id) = message["id"].as_u64()
                        && let Some(sender) = thread_pending.lock().unwrap().remove(&id)
                    {
                        let result = if let Some(error) = message.get("error") {
                            Err(anyhow!("{name}: {error}"))
                        } else {
                            Ok(message["result"].clone())
                        };
                        let _ = sender.send(result);
                    }
                }
            })();
            let message = result
                .err()
                .map(|e| e.to_string())
                .unwrap_or_else(|| format!("{name} 已退出"));
            *thread_error.lock().unwrap() = Some(message.clone());
            for (_, sender) in thread_pending.lock().unwrap().drain() {
                let _ = sender.send(Err(anyhow!(message.clone())));
            }
        });
        let mut service = Self {
            inner: Arc::new(Inner {
                process: Mutex::new(process),
                writer,
                pending,
                diagnostics,
                error,
                next_id: AtomicU64::new(1),
                document: Mutex::new(Document::default()),
                uri,
                language,
                name,
                inlay_hints: true,
                _project: project,
            }),
        };
        let root_uri = file_uri(&root)?;
        let settings = settings_for(language);
        let initialization = if language == PracticeLanguage::Java {
            json!({"settings":settings})
        } else {
            settings.clone()
        };
        let initialized = service.request_timeout("initialize", json!({
            "processId":std::process::id(), "clientInfo":{"name":"LeetDesk","version":"0.1.0"},
            "rootUri":root_uri, "workspaceFolders":[{"uri":root_uri,"name":format!("{} practice", language.label())}],
            "initializationOptions":initialization,
            "capabilities":{
                "general":{"positionEncodings":["utf-16"]},
                "workspace":{"configuration":true,"workspaceFolders":true},
                "textDocument":{
                    "synchronization":{"didSave":true},
                    "publishDiagnostics":{"versionSupport":true},
                    "completion":{"completionItem":{"snippetSupport":true,"documentationFormat":["markdown","plaintext"],"insertReplaceSupport":true}},
                    "hover":{"contentFormat":["markdown","plaintext"]},
                    "inlayHint":{},
                    "definition":{"linkSupport":true}
                }
            }
        }), Duration::from_secs(if language == PracticeLanguage::Java { 90 } else { 45 })).with_context(|| {
            let logs = [root.parent().unwrap_or(&root).join(".jdtls/.metadata/.log"), root.parent().unwrap_or(&root).join(".configuration/.log")]
                .iter().filter_map(|path| fs::read_to_string(path).ok()).collect::<Vec<_>>().join("\n");
            format!("{name} 初始化失败。{logs}")
        })?;
        Arc::get_mut(&mut service.inner).unwrap().inlay_hints = initialized["capabilities"]
            .get("inlayHintProvider")
            .is_some_and(|v| v != &Value::Bool(false));
        service.notify("initialized", json!({}))?;
        if language == PracticeLanguage::Java {
            service.notify(
                "workspace/didChangeConfiguration",
                json!({"settings":settings}),
            )?;
        }
        {
            let mut doc = service.inner.document.lock().unwrap();
            doc.text = source.into();
            doc.version = 1;
        }
        service.notify("textDocument/didOpen", json!({"textDocument":{"uri":service.uri(),"languageId":language.highlighter(),"version":1,"text":server_source(language, source)}}))?;
        Ok(service)
    }

    pub fn uri(&self) -> &str {
        &self.inner.uri
    }
    pub fn name(&self) -> &'static str {
        self.inner.name
    }
    pub fn language(&self) -> PracticeLanguage {
        self.inner.language
    }
    pub fn version(&self) -> i32 {
        self.inner.document.lock().unwrap().version
    }
    pub fn error(&self) -> Option<String> {
        self.inner.error.lock().unwrap().clone()
    }
    pub fn diagnostics(&self) -> Diagnostics {
        self.inner
            .diagnostics
            .lock()
            .unwrap()
            .get(&diagnostic_uri(self.uri()))
            .cloned()
            .unwrap_or_default()
    }

    /// Poll without cloning diagnostic messages that have already been delivered.
    pub fn diagnostics_since(&self, generation: u64) -> Option<Diagnostics> {
        let snapshots = self.inner.diagnostics.lock().unwrap();
        let snapshot = snapshots.get(&diagnostic_uri(self.uri()))?;
        (snapshot.generation != generation).then(|| snapshot.clone())
    }

    pub fn sync(&self, source: &str) -> Result<()> {
        let mut doc = self.inner.document.lock().unwrap();
        if doc.text == source {
            return Ok(());
        }
        doc.version += 1;
        self.notify("textDocument/didChange", json!({"textDocument":{"uri":self.uri(),"version":doc.version},"contentChanges":[{"text":server_source(self.language(), source)}]}))?;
        doc.text = source.into();
        Ok(())
    }

    pub fn saved(&self) -> Result<()> {
        if let Some(project) = &self.inner._project {
            let doc = self.inner.document.lock().unwrap();
            fs::write(
                project.path().join("project/Solution.java"),
                server_source(self.language(), &doc.text),
            )?;
        }
        self.notify(
            "textDocument/didSave",
            json!({"textDocument":{"uri":self.uri()}}),
        )
    }

    pub fn request(&self, method: &str, params: Value) -> Result<Value> {
        self.request_timeout(method, params, Duration::from_secs(20))
    }

    pub fn request_timeout(&self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        if let Some(error) = self.error() {
            bail!(error);
        }
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = mpsc::channel();
        self.inner.pending.lock().unwrap().insert(id, sender);
        let result = (|| {
            write_message(
                &mut *self.inner.writer.lock().unwrap(),
                &json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
            )?;
            let mut value = receiver
                .recv_timeout(timeout)
                .with_context(|| format!("{} 请求 {method} 超时", self.name()))??;
            if self.language() == PracticeLanguage::Java {
                unmap_response(method, &mut value, self.uri());
            }
            Ok(value)
        })();
        self.inner.pending.lock().unwrap().remove(&id);
        result
    }

    fn notify(&self, method: &str, params: Value) -> Result<()> {
        write_message(
            &mut *self.inner.writer.lock().unwrap(),
            &json!({"jsonrpc":"2.0","method":method,"params":params}),
        )
    }

    pub fn at(&self, source: &str, offset: usize) -> Value {
        let mut position = byte_position(source, offset);
        if self.language() == PracticeLanguage::Java {
            position.line += JAVA_PREFIX_LINES;
        }
        json!({"textDocument":{"uri":self.uri()},"position":position})
    }

    pub fn format_java(&self, source: &str) -> Result<String> {
        if self.language() != PracticeLanguage::Java {
            bail!("需要 Java 语言服务");
        }
        self.sync(source)?;
        let value = self.request(
            "textDocument/formatting",
            json!({
                "textDocument":{"uri":self.uri()},
                "options":{"tabSize":4,"insertSpaces":true}
            }),
        )?;
        let edits: Vec<lsp_types::TextEdit> = if value.is_null() {
            vec![]
        } else {
            serde_json::from_value(value)?
        };
        let formatted = apply_edits(&server_source(self.language(), source), &edits)?;
        Ok(formatted
            .strip_prefix(JAVA_PREFIX)
            .context("Java 格式化返回了不一致的分析导入，已保留原草稿")?
            .into())
    }

    /// The caller synchronizes the snapshot before requesting its annotations.
    pub fn type_hints(&self, source: &str) -> Result<Vec<(usize, String)>> {
        if !self.inner.inlay_hints {
            return Ok(vec![]);
        }
        let value = self.request(
            "textDocument/inlayHint",
            json!({
                "textDocument":{"uri":self.uri()},
                "range":{"start":self.at(source, 0)["position"],"end":self.at(source, source.len())["position"]}
            }),
        )?;
        let hints: Vec<lsp_types::InlayHint> = if value.is_null() {
            vec![]
        } else {
            serde_json::from_value(value)?
        };
        Ok(hints
            .into_iter()
            .filter(|hint| {
                self.language() == PracticeLanguage::Java
                    || hint
                        .kind
                        .is_none_or(|kind| kind == lsp_types::InlayHintKind::TYPE)
            })
            .filter_map(|hint| {
                let label = match hint.label {
                    lsp_types::InlayHintLabel::String(label) => label,
                    lsp_types::InlayHintLabel::LabelParts(parts) => {
                        parts.into_iter().map(|part| part.value).collect()
                    }
                };
                let label = label.trim();
                if label.is_empty() {
                    return None;
                }
                let label = if self.language() == PracticeLanguage::Java {
                    format!("{label} ")
                } else {
                    format!(": {label}")
                };
                Some((position_byte(source, hint.position), label))
            })
            .collect())
    }
}

fn settings() -> Value {
    json!({"gofumpt":false,"staticcheck":false,"analyses":{"ST1000":false,"U1000":false,"unusedparams":false},"completeUnimported":false,"completeFunctionCalls":true,"usePlaceholders":true,"diagnosticsDelay":"200ms", "hints":{"assignVariableTypes":true,"rangeVariableTypes":true}})
}

const JAVA_PREFIX: &str = "import java.util.*;\nimport java.math.*;\n";
const JAVA_PREFIX_LINES: u32 = 2;

fn server_source(language: PracticeLanguage, source: &str) -> String {
    match language {
        PracticeLanguage::Go => source.into(),
        PracticeLanguage::Java => format!("{JAVA_PREFIX}{source}"),
    }
}

fn settings_for(language: PracticeLanguage) -> Value {
    match language {
        PracticeLanguage::Go => settings(),
        PracticeLanguage::Java => json!({"java":{
            "autobuild":{"enabled":true},
            "completion":{"enabled":true,"guessMethodArguments":true},
            "signatureHelp":{"enabled":true},
            "inlayHints":{"parameterNames":{"enabled":"all"}},
            "import":{"gradle":{"enabled":false},"maven":{"enabled":false}},
            "configuration":{"updateBuildConfiguration":"automatic"}
        }}),
    }
}

fn configuration(language: PracticeLanguage, section: Option<&str>) -> Value {
    let settings = settings_for(language);
    if language == PracticeLanguage::Go {
        return settings;
    }
    section
        .filter(|s| !s.is_empty())
        .map_or(settings.clone(), |section| {
            section
                .split('.')
                .fold(&settings, |value, key| &value[key])
                .clone()
        })
}

fn prepare_java_project(workspace: &Path, source: &str) -> Result<tempfile::TempDir> {
    let project = tempfile::Builder::new()
        .prefix("leetdesk-java-")
        .tempdir_in(workspace)?;
    let root = project.path().join("project");
    fs::create_dir(&root)?;
    fs::write(
        root.join("Solution.java"),
        server_source(PracticeLanguage::Java, source),
    )?;
    // An explicit Eclipse project enables full diagnostics instead of standalone-file mode.
    fs::write(
        root.join(".project"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<projectDescription><name>LeetDeskPractice</name><buildSpec><buildCommand><name>org.eclipse.jdt.core.javabuilder</name></buildCommand></buildSpec><natures><nature>org.eclipse.jdt.core.javanature</nature></natures></projectDescription>"#,
    )?;
    fs::write(
        root.join(".classpath"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<classpath><classpathentry kind="src" path="" excluding=".jdtls/**|.configuration/**|bin/**"/><classpathentry kind="con" path="org.eclipse.jdt.launching.JRE_CONTAINER"/><classpathentry kind="output" path="bin"/></classpath>"#,
    )?;
    fs::create_dir(root.join(".settings"))?;
    fs::write(
        root.join(".settings/org.eclipse.jdt.core.prefs"),
        "eclipse.preferences.version=1\norg.eclipse.jdt.core.compiler.problem.unusedLocal=warning\norg.eclipse.jdt.core.compiler.problem.unusedImport=ignore\n",
    )?;
    // Platform-provided nodes live in separate files; drafts defining their own are respected.
    let comments = regex::Regex::new(r#"(?s)/\*.*?\*/|//[^\r\n]*|"(?:\\.|[^"\\])*""#)?;
    let code = comments.replace_all(source, "");
    for (name, fields, constructors) in [
        (
            "ListNode",
            "int val; ListNode next;",
            "ListNode() {} ListNode(int val) { this.val = val; } ListNode(int val, ListNode next) { this.val = val; this.next = next; }",
        ),
        (
            "TreeNode",
            "int val; TreeNode left, right;",
            "TreeNode() {} TreeNode(int val) { this.val = val; } TreeNode(int val, TreeNode left, TreeNode right) { this.val = val; this.left = left; this.right = right; }",
        ),
    ] {
        let pattern = regex::Regex::new(&format!(r"\b(?:class|record|interface|enum)\s+{name}\b"))?;
        if !pattern.is_match(&code) {
            fs::write(
                root.join(format!("{name}.java")),
                format!("class {name} {{ {fields} {constructors} }}\n"),
            )?;
        }
    }
    Ok(project)
}

pub fn jdtls_home() -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("LEETDESK_JDTLS_HOME") {
        return Some(path.into());
    }
    let mut candidates = vec![];
    if let Ok(exe) = std::env::current_exe() {
        candidates.extend(
            exe.ancestors()
                .skip(1)
                .take(4)
                .map(|p| p.join("tools/jdtls")),
        );
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("tools/jdtls"));
    }
    candidates.into_iter().find(|p| p.join("plugins").is_dir())
}

fn java_server_command(root: &Path) -> Result<Command> {
    let session = root.parent().context("Java session path")?;
    fs::create_dir_all(session.join(".tmp"))?;
    let mut command = if let Some(home) = jdtls_home() {
        let home = process_path(
            home.canonicalize()
                .context("LEETDESK_JDTLS_HOME 路径无效")?,
        );
        let launcher = fs::read_dir(home.join("plugins"))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .find(|p| {
                p.file_name().is_some_and(|n| {
                    let n = n.to_string_lossy();
                    (n.starts_with("org.eclipse.equinox.launcher_")
                        || n == "org.eclipse.equinox.launcher.jar")
                        && n.ends_with(".jar")
                })
            })
            .context("JDT LS 缺少 Equinox launcher JAR")?;
        let java = std::env::var_os("LEETDESK_JAVA")
            .or_else(|| {
                std::env::var_os("JAVA_HOME").map(|p| {
                    std::path::PathBuf::from(p)
                        .join(if cfg!(windows) {
                            "bin/java.exe"
                        } else {
                            "bin/java"
                        })
                        .into_os_string()
                })
            })
            .unwrap_or_else(|| "java".into());
        let mut command = tool_command(java);
        // Java's Windows launcher can lose non-ACP characters in absolute JAR
        // arguments. Use a relative JAR and URI-encoded Eclipse locations.
        command.current_dir(&home);
        command.arg(format!(
            "-Djava.io.tmpdir={}",
            session.join(".tmp").display()
        ));
        let config = if cfg!(windows) {
            "config_win"
        } else if cfg!(target_os = "macos") {
            if cfg!(target_arch = "aarch64") {
                "config_mac_arm"
            } else {
                "config_mac"
            }
        } else if cfg!(target_arch = "aarch64") {
            "config_linux_arm"
        } else {
            "config_linux"
        };
        command
            .args([
                "-Declipse.application=org.eclipse.jdt.ls.core.id1",
                "-Dosgi.bundles.defaultStartLevel=4",
                "-Declipse.product=org.eclipse.jdt.ls.core.product",
                "-Dosgi.sharedConfiguration.area.readOnly=true",
                "-Dosgi.configuration.cascaded=true",
                "-Djdk.xml.maxGeneralEntitySizeLimit=0",
                "-Djdk.xml.totalEntitySizeLimit=0",
                "-Xmx1G",
                "--add-modules=ALL-SYSTEM",
                "--add-opens=java.base/java.util=ALL-UNNAMED",
                "--add-opens=java.base/java.lang=ALL-UNNAMED",
            ])
            .arg(format!(
                "-Dosgi.sharedConfiguration.area={}",
                file_uri(&home.join(config))?
            ))
            .arg("-jar")
            .arg(launcher.strip_prefix(&home)?);
        command
    } else {
        let mut command =
            tool_command(std::env::var_os("LEETDESK_JDTLS").unwrap_or_else(|| "jdtls".into()));
        command.current_dir(root);
        command
    };
    command
        .arg("-configuration")
        .arg(file_uri(&session.join(".configuration"))?)
        .arg("-data")
        .arg(file_uri(&session.join(".jdtls"))?);
    // Force stdio even if the parent environment was used for a socket client.
    for key in ["CLIENT_PORT", "CLIENT_HOST", "CLIENT_CONNECTION"] {
        command.env_remove(key);
    }
    Ok(command)
}

fn unmap_range(range: &mut lsp_types::Range) {
    for position in [&mut range.start, &mut range.end] {
        if position.line < JAVA_PREFIX_LINES {
            *position = Position::new(0, 0);
        } else {
            position.line -= JAVA_PREFIX_LINES;
        }
    }
}

fn unmap_json_range(value: &mut Value) {
    if let Ok(mut range) = serde_json::from_value::<lsp_types::Range>(value.clone()) {
        unmap_range(&mut range);
        *value = json!(range);
    }
}

fn unmap_response(method: &str, value: &mut Value, uri: &str) {
    if value.is_null() {
        return;
    }
    match method {
        "textDocument/completion" => {
            let items = if value.is_array() {
                value.as_array_mut()
            } else {
                value["items"].as_array_mut()
            };
            if let Some(items) = items {
                for item in items {
                    for key in ["range", "insert", "replace"] {
                        if let Some(range) =
                            item.get_mut("textEdit").and_then(|edit| edit.get_mut(key))
                        {
                            unmap_json_range(range);
                        }
                    }
                    if let Some(edits) = item["additionalTextEdits"].as_array_mut() {
                        for edit in edits {
                            unmap_json_range(&mut edit["range"]);
                        }
                    }
                }
            }
        }
        "textDocument/hover" => {
            if let Some(range) = value.get_mut("range") {
                unmap_json_range(range);
            }
        }
        "textDocument/definition" => {
            if let Some(items) = value.as_array_mut() {
                for item in items {
                    unmap_definition(item, uri);
                }
            } else {
                unmap_definition(value, uri);
            }
        }
        "textDocument/inlayHint" => {
            if let Some(items) = value.as_array_mut() {
                for item in items {
                    if let Some(line) = item["position"]["line"].as_u64() {
                        item["position"]["line"] =
                            json!(line.saturating_sub(JAVA_PREFIX_LINES as u64));
                    }
                }
            }
        }
        _ => {}
    }
}

fn unmap_definition(value: &mut Value, uri: &str) {
    if value["uri"]
        .as_str()
        .is_some_and(|target| diagnostic_uri(target) == diagnostic_uri(uri))
    {
        unmap_json_range(&mut value["range"]);
    }
    if value["targetUri"]
        .as_str()
        .is_some_and(|target| diagnostic_uri(target) == diagnostic_uri(uri))
    {
        unmap_json_range(&mut value["targetRange"]);
        unmap_json_range(&mut value["targetSelectionRange"]);
    }
    if let Some(range) = value.get_mut("originSelectionRange") {
        unmap_json_range(range);
    }
}

pub fn apply_edits(source: &str, edits: &[lsp_types::TextEdit]) -> Result<String> {
    let mut edits: Vec<_> = edits
        .iter()
        .map(|edit| {
            (
                position_byte(source, edit.range.start),
                position_byte(source, edit.range.end),
                &edit.new_text,
            )
        })
        .collect();
    edits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    let mut previous_start = source.len();
    let mut result = source.to_string();
    for (start, end, text) in edits {
        if start > end || end > previous_start {
            bail!("语言服务器返回重叠的修改范围");
        }
        result.replace_range(start..end, text);
        previous_start = start;
    }
    Ok(result)
}

/// Give macOS GUI-launched children the standard Go installation paths.
/// Preserve the inherited order so terminal users keep their chosen toolchain.
pub fn tool_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new(program);
    #[cfg(target_os = "macos")]
    {
        let mut paths: Vec<std::path::PathBuf> = std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).collect())
            .unwrap_or_default();
        let mut extra = Vec::new();
        if let Some(root) = std::env::var_os("GOROOT") {
            extra.push(std::path::PathBuf::from(root).join("bin"));
        }
        if let Some(bin) = std::env::var_os("GOBIN") {
            extra.push(bin.into());
        }
        if let Some(gopath) = std::env::var_os("GOPATH") {
            extra.extend(std::env::split_paths(&gopath).map(|path| path.join("bin")));
        }
        if let Some(home) = std::env::var_os("HOME") {
            extra.push(std::path::PathBuf::from(home).join("go/bin"));
        }
        extra.extend([
            "/usr/local/go/bin".into(),
            "/opt/homebrew/bin".into(),
            "/usr/local/bin".into(),
            "/usr/bin".into(),
            "/bin".into(),
            "/usr/sbin".into(),
            "/sbin".into(),
        ]);
        for path in extra {
            if path.is_absolute() && path.is_dir() && !paths.contains(&path) {
                paths.push(path);
            }
        }
        if let Ok(path) = std::env::join_paths(paths) {
            command.env("PATH", path);
        }
    }
    command
}

pub fn gopls_executable() -> std::ffi::OsString {
    if let Some(path) = std::env::var_os("LEETDESK_GOPLS") {
        return path;
    }
    // A standard `go install` puts binaries here, even when GOBIN is absent from PATH.
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        let path = std::path::PathBuf::from(home)
            .join("go/bin")
            .join(if cfg!(windows) { "gopls.exe" } else { "gopls" });
        if path.is_file() {
            return path.into_os_string();
        }
    }
    "gopls".into()
}

pub fn gofmt_executable() -> std::ffi::OsString {
    if let Some(path) = std::env::var_os("LEETDESK_GOFMT") {
        return path;
    }
    if let Some(root) = std::env::var_os("GOROOT") {
        let path = std::path::PathBuf::from(root)
            .join("bin")
            .join(if cfg!(windows) { "gofmt.exe" } else { "gofmt" });
        if path.is_file() {
            return path.into_os_string();
        }
    }
    #[cfg(windows)]
    {
        if let Some(program_files) = std::env::var_os("ProgramFiles") {
            let path = std::path::PathBuf::from(program_files).join("Go/bin/gofmt.exe");
            if path.is_file() {
                return path.into_os_string();
            }
        }
    }
    "gofmt".into()
}

pub fn same_document_uri(left: &str, right: &str) -> bool {
    diagnostic_uri(left) == diagnostic_uri(right)
}

fn diagnostic_uri(uri: &str) -> String {
    let uri = url::Url::parse(uri).map_or_else(
        |_| uri.into(),
        |uri| {
            uri.to_file_path()
                .ok()
                .and_then(|path| url::Url::from_file_path(path).ok())
                .unwrap_or(uri)
                .to_string()
        },
    );
    if cfg!(windows) && uri.starts_with("file:") {
        uri.to_ascii_lowercase()
    } else {
        uri
    }
}

fn process_path(path: std::path::PathBuf) -> std::path::PathBuf {
    #[cfg(windows)]
    if let Some(value) = path.to_string_lossy().strip_prefix(r"\\?\") {
        return value.into();
    }
    path
}

pub fn file_uri(path: &Path) -> Result<String> {
    #[cfg(windows)]
    let path = {
        let mut path = path.to_path_buf();
        let text = path.to_string_lossy();
        if let Some(value) = text.strip_prefix(r"\\?\") {
            path = value.into();
        }
        path
    };
    #[cfg(not(windows))]
    let path = path.to_path_buf();
    url::Url::from_file_path(&path)
        .map(|uri| uri.to_string())
        .map_err(|_| anyhow!("无法转换文件路径 {}", path.display()))
}

pub fn byte_position(text: &str, offset: usize) -> Position {
    let mut end = offset.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let before = &text[..end];
    let line = before.bytes().filter(|b| *b == b'\n').count() as u32;
    let column = before
        .rsplit('\n')
        .next()
        .unwrap_or("")
        .encode_utf16()
        .count() as u32;
    Position::new(line, column)
}

pub fn position_byte(text: &str, position: Position) -> usize {
    let mut start = 0;
    for _ in 0..position.line {
        match text[start..].find('\n') {
            Some(i) => start += i + 1,
            None => return text.len(),
        }
    }
    let mut units = 0;
    for (offset, ch) in text[start..].char_indices() {
        if units >= position.character || ch == '\n' {
            return start + offset;
        }
        units += ch.len_utf16() as u32;
    }
    text.len()
}

pub fn write_message(writer: &mut impl Write, message: &Value) -> Result<()> {
    let bytes = serde_json::to_vec(message)?;
    write!(writer, "Content-Length: {}\r\n\r\n", bytes.len())?;
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(())
}

pub fn read_message(reader: &mut impl BufRead) -> Result<Value> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            bail!("语言服务输出流已关闭");
        }
        if line.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("Content-Length")
        {
            length = Some(value.trim().parse::<usize>()?);
        }
    }
    let length = length.context("LSP 消息缺少 Content-Length")?;
    if length > 16 * 1024 * 1024 {
        bail!("LSP 消息过大");
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}

pub fn hide_console(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    #[cfg(not(windows))]
    let _ = command;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn java_protocol_maps_import_offsets_without_moving_other_files() {
        assert!(same_document_uri(
            "file:///C:/practice/%23!sandbox.%7Btest%7D/Solution.java",
            "file:/C:/practice/%23%21sandbox.%7btest%7d/Solution.java"
        ));
        let uri = "file:///practice/Solution.java";
        let mut completion = json!({"items":[{"textEdit":{"range":{"start":{"line":4,"character":6},"end":{"line":4,"character":8}},"newText":"next"},"additionalTextEdits":[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}},"newText":"import java.io.File;\n"}]}]});
        unmap_response("textDocument/completion", &mut completion, uri);
        assert_eq!(
            completion["items"][0]["textEdit"]["range"]["start"]["line"],
            2
        );
        assert_eq!(
            completion["items"][0]["additionalTextEdits"][0]["range"]["start"]["line"],
            0
        );
        let mut definitions = json!([
            {"uri":"file:/practice/Solution.java","range":{"start":{"line":5,"character":2},"end":{"line":5,"character":3}}},
            {"uri":"file:///practice/ListNode.java","range":{"start":{"line":0,"character":6},"end":{"line":0,"character":14}}}
        ]);
        unmap_response("textDocument/definition", &mut definitions, uri);
        assert_eq!(definitions[0]["range"]["start"]["line"], 3);
        assert_eq!(definitions[1]["range"]["start"]["character"], 6);
        let mut absent = Value::Null;
        unmap_response("textDocument/completion", &mut absent, uri);
        assert!(absent.is_null());
        assert_eq!(
            configuration(PracticeLanguage::Java, Some("java.completion"))["enabled"],
            true
        );
    }

    #[test]
    fn java_project_supplies_nodes_and_never_rewrites_the_saved_solution() {
        let workspace = tempfile::tempdir().unwrap();
        let source = "/** public class ListNode {} */\nclass Solution {}";
        fs::write(workspace.path().join("Solution.java"), source).unwrap();
        let project = prepare_java_project(workspace.path(), source).unwrap();
        assert!(project.path().join("project/ListNode.java").is_file());
        assert!(project.path().join("project/TreeNode.java").is_file());
        assert_eq!(
            fs::read_to_string(workspace.path().join("Solution.java")).unwrap(),
            source
        );
        let custom =
            prepare_java_project(workspace.path(), "class ListNode {}\nclass Solution {}").unwrap();
        assert!(!custom.path().join("project/ListNode.java").exists());
    }
    #[test]
    fn lsp_coordinates_handle_chinese_and_surrogate_pairs() {
        let text = "// 中文🙂\nfunc abc() {}";
        let end = text.find('\n').unwrap();
        assert_eq!(byte_position(text, end), Position::new(0, 7));
        assert_eq!(position_byte(text, Position::new(0, 7)), end);
        assert_eq!(position_byte(text, Position::new(1, 5)), end + 6);
    }
    #[test]
    fn protocol_lengths_are_utf8_bytes_and_multiple_frames_remain_separate() {
        let first = json!({"result":"你好🙂"});
        let second = json!({"id":2});
        let mut bytes = vec![];
        write_message(&mut bytes, &first).unwrap();
        write_message(&mut bytes, &second).unwrap();
        let mut reader = std::io::Cursor::new(bytes);
        assert_eq!(read_message(&mut reader).unwrap(), first);
        assert_eq!(read_message(&mut reader).unwrap(), second);
        assert!(read_message(&mut reader).is_err());
    }
}
