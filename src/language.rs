//! A small stdio LSP client. The desktop layer supplies GPUI editor providers.
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
pub struct GoLanguage {
    inner: Arc<Inner>,
}

impl GoLanguage {
    pub fn start(workspace: &Path, source: &str) -> Result<Self> {
        let workspace = workspace.canonicalize()?;
        let uri = file_uri(&workspace.join("solution.go"))?;
        let cache = workspace
            .parent()
            .context("工作区路径无效")?
            .join(".go-cache");
        fs::create_dir_all(&cache)?;
        let log = fs::File::create(workspace.join("gopls.log"))?;
        let mut command = Command::new(gopls_executable());
        command
            .arg("serve")
            .current_dir(&workspace)
            .env("GOCACHE", &cache)
            .env("GOTOOLCHAIN", "local")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(log));
        hide_console(&mut command);
        let mut process = command
            .spawn()
            .context("无法启动 gopls；请安装 Go 和 gopls，并把它们加入 PATH")?;
        let writer = Arc::new(Mutex::new(process.stdin.take().context("gopls stdin")?));
        let stdout = process.stdout.take().context("gopls stdout")?;
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
                            let mut values = thread_diagnostics.lock().unwrap();
                            let previous = values.get(uri).map_or(0, |d| d.generation);
                            values.insert(
                                uri.into(),
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
                                        items.iter().map(|_| settings()).collect()
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
                            Err(anyhow!("gopls: {error}"))
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
                .unwrap_or_else(|| "gopls 已退出".into());
            *thread_error.lock().unwrap() = Some(message.clone());
            for (_, sender) in thread_pending.lock().unwrap().drain() {
                let _ = sender.send(Err(anyhow!(message.clone())));
            }
        });
        let service = Self {
            inner: Arc::new(Inner {
                process: Mutex::new(process),
                writer,
                pending,
                diagnostics,
                error,
                next_id: AtomicU64::new(1),
                document: Mutex::new(Document::default()),
                uri,
            }),
        };
        let root_uri = file_uri(&workspace)?;
        service.request_timeout("initialize", json!({
            "processId":std::process::id(), "clientInfo":{"name":"LeetDesk","version":"0.1.0"},
            "rootUri":root_uri, "workspaceFolders":[{"uri":root_uri,"name":"Go practice"}],
            "initializationOptions":settings(),
            "capabilities":{
                "general":{"positionEncodings":["utf-16"]},
                "workspace":{"configuration":true,"workspaceFolders":true},
                "textDocument":{
                    "synchronization":{"didSave":true},
                    "publishDiagnostics":{"versionSupport":true},
                    "completion":{"completionItem":{"snippetSupport":false,"documentationFormat":["markdown","plaintext"],"insertReplaceSupport":true}},
                    "hover":{"contentFormat":["markdown","plaintext"]},
                    "inlayHint":{},
                    "definition":{"linkSupport":true}
                }
            }
        }), Duration::from_secs(45))?;
        service.notify("initialized", json!({}))?;
        {
            let mut doc = service.inner.document.lock().unwrap();
            doc.text = source.into();
            doc.version = 1;
        }
        service.notify("textDocument/didOpen", json!({"textDocument":{"uri":service.uri(),"languageId":"go","version":1,"text":source}}))?;
        Ok(service)
    }

    pub fn uri(&self) -> &str {
        &self.inner.uri
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
            .get(self.uri())
            .cloned()
            .unwrap_or_default()
    }

    /// Poll without cloning diagnostic messages that have already been delivered.
    pub fn diagnostics_since(&self, generation: u64) -> Option<Diagnostics> {
        let snapshots = self.inner.diagnostics.lock().unwrap();
        let snapshot = snapshots.get(self.uri())?;
        (snapshot.generation != generation).then(|| snapshot.clone())
    }

    pub fn sync(&self, source: &str) -> Result<()> {
        let mut doc = self.inner.document.lock().unwrap();
        if doc.text == source {
            return Ok(());
        }
        doc.version += 1;
        self.notify("textDocument/didChange", json!({"textDocument":{"uri":self.uri(),"version":doc.version},"contentChanges":[{"text":source}]}))?;
        doc.text = source.into();
        Ok(())
    }

    pub fn saved(&self) -> Result<()> {
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
            receiver
                .recv_timeout(timeout)
                .with_context(|| format!("gopls 请求 {method} 超时"))?
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
        json!({"textDocument":{"uri":self.uri()},"position":byte_position(source, offset)})
    }

    /// The caller synchronizes the snapshot before requesting its annotations.
    pub fn type_hints(&self, source: &str) -> Result<Vec<(usize, String)>> {
        let value = self.request(
            "textDocument/inlayHint",
            json!({
                "textDocument":{"uri":self.uri()},
                "range":{"start":Position::new(0,0),"end":byte_position(source, source.len())}
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
                hint.kind
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
                Some((position_byte(source, hint.position), format!(": {label}")))
            })
            .collect())
    }
}

fn settings() -> Value {
    json!({"gofumpt":false,"staticcheck":false,"analyses":{"ST1000":false,"U1000":false,"unusedparams":false},"completeUnimported":false,"usePlaceholders":false,"diagnosticsDelay":"200ms", "hints":{"assignVariableTypes":true,"rangeVariableTypes":true}})
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

pub fn file_uri(path: &Path) -> Result<String> {
    let mut path = path.to_path_buf();
    #[cfg(windows)]
    {
        let text = path.to_string_lossy();
        if let Some(value) = text.strip_prefix(r"\\?\") {
            path = value.into();
        }
    }
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
            bail!("gopls 输出流已关闭");
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
