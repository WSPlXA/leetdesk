use crate::{
    catalog::{Case, Kind, Problem, validate_cases},
    language::{gofmt_executable, gopls_executable, hide_console, tool_command},
    store::NODE_TYPES,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    io::Read,
    path::Path,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

static NEXT_RUN: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaseResult {
    pub index: usize,
    pub input: Vec<Value>,
    pub expected: Value,
    pub actual: Value,
    pub passed: bool,
    #[serde(default)]
    pub error: String,
}

#[derive(Clone, Debug)]
pub struct RunResult {
    pub passed: usize,
    pub total: usize,
    pub output: String,
    pub cases: Vec<CaseResult>,
}

pub fn run(
    problem: &Problem,
    source: &str,
    cases: &[Case],
    workspace: &Path,
    cancel: Arc<AtomicBool>,
) -> Result<RunResult> {
    validate_cases(cases, &problem.parameters)?;
    let key = format!(
        "{}-{}",
        std::process::id(),
        NEXT_RUN.fetch_add(1, Ordering::Relaxed)
    );
    let run_dir = workspace.join("runs").join(&key);
    fs::create_dir_all(&run_dir)?;
    // Snapshot the source and cases. Editing the current solution cannot change an in-flight run.
    fs::write(
        run_dir.join("go.mod"),
        "module leetdesk.local/run\n\ngo 1.22\n",
    )?;
    fs::write(run_dir.join("solution.go"), source)?;
    fs::write(run_dir.join("types.go"), NODE_TYPES)?;
    fs::write(run_dir.join("leetdesk_test.go"), harness(problem, cases)?)?;
    let cache = workspace.parent().context("工作区无效")?.join(".go-cache");
    fs::create_dir_all(&cache)?;
    let executable = run_dir.join(if cfg!(windows) {
        "cases.test.exe"
    } else {
        "cases.test"
    });
    let mut build = tool_command("go");
    build
        .args(["test", "-c", "-vet=off", "-o"])
        .arg(&executable)
        .arg(".")
        .current_dir(&run_dir)
        .env("GOCACHE", &cache)
        .env("GOTOOLCHAIN", "local");
    let (success, build_output) = execute(build, Duration::from_secs(90), &cancel)?;
    if !success {
        return Ok(RunResult {
            passed: 0,
            total: cases.len(),
            output: format!("编译失败\n\n{build_output}"),
            cases: vec![],
        });
    }
    let mut command = Command::new(&executable);
    command
        .args(["-test.v", "-test.timeout=5s"])
        .current_dir(&run_dir);
    let (success, raw) = execute(command, Duration::from_secs(8), &cancel)?;
    let results: Vec<CaseResult> = raw
        .lines()
        .filter_map(|line| line.strip_prefix("LEETDESK_CASE:"))
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    let passed = results.iter().filter(|case| case.passed).count();
    let mut output = format!("本地用例：{passed}/{} 通过\n", cases.len());
    for case in &results {
        output.push_str(&format!(
            "\n{} 用例 {}\n输入：{}\n预期：{}\n实际：{}\n",
            if case.passed { "✓" } else { "✗" },
            case.index + 1,
            Value::Array(case.input.clone()),
            case.expected,
            case.actual
        ));
        if !case.error.is_empty() {
            output.push_str(&format!("错误：{}\n", case.error));
        }
    }
    if !success && results.len() < cases.len() {
        output.push_str(&format!("\n执行未完成\n{raw}"));
    }
    if raw.lines().any(|line| {
        !line.starts_with("LEETDESK_CASE:")
            && !line.starts_with("===")
            && !line.starts_with("---")
            && !line.trim_start().starts_with("---")
            && line != "PASS"
            && line != "FAIL"
    }) {
        output.push_str("\n程序输出：\n");
        for line in raw
            .lines()
            .filter(|line| !line.starts_with("LEETDESK_CASE:"))
        {
            output.push_str(line);
            output.push('\n');
        }
    }
    // Keep the source snapshot for review; compiled test binaries are disposable.
    let _ = fs::remove_file(executable);
    Ok(RunResult {
        passed,
        total: cases.len(),
        output,
        cases: results,
    })
}

pub fn run_java(
    problem: &Problem,
    source: &str,
    cases: &[Case],
    workspace: &Path,
    cancel: Arc<AtomicBool>,
) -> Result<RunResult> {
    validate_cases(cases, &problem.parameters)?;
    let key = format!(
        "java-{}-{}",
        std::process::id(),
        NEXT_RUN.fetch_add(1, Ordering::Relaxed)
    );
    let run_dir = workspace.join("runs").join(key);
    fs::create_dir_all(&run_dir)?;
    fs::write(
        run_dir.join("Solution.java"),
        format!("import java.util.*;\nimport java.math.*;\n{source}"),
    )?;
    fs::write(
        run_dir.join("LeetDeskHarness.java"),
        java_harness(problem, cases)?,
    )?;
    let mut compile = Command::new("javac");
    compile
        .args([
            "-encoding",
            "UTF-8",
            "Solution.java",
            "LeetDeskHarness.java",
        ])
        .current_dir(&run_dir);
    let (success, output) = execute(compile, Duration::from_secs(45), &cancel)
        .context("无法编译 Java；请安装 JDK 并将 javac 加入 PATH")?;
    if !success {
        return Ok(RunResult {
            passed: 0,
            total: cases.len(),
            output: format!("Java 编译失败\n\n{output}"),
            cases: vec![],
        });
    }
    let mut command = Command::new("java");
    command
        .args(["-Xmx256m", "-cp", ".", "LeetDeskHarness"])
        .current_dir(&run_dir);
    let (success, raw) = execute(command, Duration::from_secs(8), &cancel)
        .context("无法运行 Java；请安装 JDK 并将 java 加入 PATH")?;
    let mut results = Vec::new();
    for line in raw
        .lines()
        .filter_map(|line| line.strip_prefix("LEETDESK_CASE:"))
    {
        let Some((index, actual)) = line.split_once(':') else {
            continue;
        };
        let Ok(index) = index.parse::<usize>() else {
            continue;
        };
        let Some(case) = cases.get(index) else {
            continue;
        };
        if let Ok(actual) = serde_json::from_str::<Value>(actual) {
            let mut expected = case.expected.clone();
            let mut compared = actual.clone();
            if problem.unordered
                && let (Some(left), Some(right)) =
                    (expected.as_array_mut(), compared.as_array_mut())
            {
                left.sort_by_key(Value::to_string);
                right.sort_by_key(Value::to_string);
            }
            results.push(CaseResult {
                index,
                input: case.input.clone(),
                expected: case.expected.clone(),
                actual,
                passed: expected == compared,
                error: String::new(),
            });
        }
    }
    let passed = results.iter().filter(|case| case.passed).count();
    let mut output = format!("本地 Java 用例：{passed}/{} 通过\n", cases.len());
    for case in &results {
        output.push_str(&format!(
            "\n{} 用例 {}\n输入：{}\n预期：{}\n实际：{}\n",
            if case.passed { "✓" } else { "✗" },
            case.index + 1,
            Value::Array(case.input.clone()),
            case.expected,
            case.actual
        ));
    }
    if !success || results.len() < cases.len() {
        output.push_str(&format!("\n执行未完成\n{raw}"));
    }
    Ok(RunResult {
        passed,
        total: cases.len(),
        output,
        cases: results,
    })
}

fn java_harness(problem: &Problem, cases: &[Case]) -> Result<String> {
    let mut calls = String::new();
    for (index, case) in cases.iter().enumerate() {
        let arguments = case
            .input
            .iter()
            .zip(&problem.parameters)
            .map(|(value, parameter)| java_value(value, parameter.kind))
            .collect::<Result<Vec<_>>>()?
            .join(", ");
        let actual = if matches!(problem.returns, Kind::List | Kind::Tree) {
            "actual == null ? java.util.Collections.emptyList() : actual"
        } else {
            "actual"
        };
        calls.push_str(&format!("try {{ Object actual = new Solution().{}({arguments}); System.out.println(\"LEETDESK_CASE:{index}:\" + json({actual})); }} catch (Throwable error) {{ error.printStackTrace(); }}\n", problem.function));
    }
    Ok(format!(
        r#"import java.util.*;
class ListNode {{ int val; ListNode next; ListNode() {{}} ListNode(int val) {{ this.val = val; }} }}
class TreeNode {{ int val; TreeNode left, right; TreeNode() {{}} TreeNode(int val) {{ this.val = val; }} }}
class LeetDeskHarness {{
    static ListNode list(int[] values) {{ ListNode head = null, tail = null; for (int value : values) {{ ListNode node = new ListNode(value); if (tail == null) head = node; else tail.next = node; tail = node; }} return head; }}
    static TreeNode tree(Integer[] values) {{ if (values.length == 0 || values[0] == null) return null; TreeNode root = new TreeNode(values[0]); ArrayDeque<TreeNode> queue = new ArrayDeque<>(); queue.add(root); int i = 1; while (!queue.isEmpty() && i < values.length) {{ TreeNode node = queue.remove(); if (i < values.length && values[i] != null) {{ node.left = new TreeNode(values[i]); queue.add(node.left); }} i++; if (i < values.length && values[i] != null) {{ node.right = new TreeNode(values[i]); queue.add(node.right); }} i++; }} return root; }}
    static String quote(String value) {{ StringBuilder out = new StringBuilder("\""); for (char ch : value.toCharArray()) {{ if (ch == '\\' || ch == '"') out.append('\\'); if (ch == '\n') out.append("\\n"); else if (ch == '\r') out.append("\\r"); else if (ch == '\t') out.append("\\t"); else out.append(ch); }} return out.append('"').toString(); }}
    static String json(Object value) {{
        if (value == null) return "null";
        if (value instanceof String s) return quote(s);
        if (value instanceof Number || value instanceof Boolean) return value.toString();
        if (value instanceof ListNode head) {{ ArrayList<Integer> items = new ArrayList<>(); for (ListNode node = head; node != null && items.size() < 10000; node = node.next) items.add(node.val); return json(items); }}
        if (value instanceof TreeNode root) {{ ArrayList<Integer> items = new ArrayList<>(); LinkedList<TreeNode> queue = new LinkedList<>(); queue.add(root); while (!queue.isEmpty() && items.size() < 10000) {{ TreeNode node = queue.remove(); if (node == null) {{ items.add(null); continue; }} items.add(node.val); queue.add(node.left); queue.add(node.right); }} while (!items.isEmpty() && items.get(items.size()-1) == null) items.remove(items.size()-1); return json(items); }}
        if (value.getClass().isArray()) {{ ArrayList<Object> items = new ArrayList<>(); for (int i=0; i<java.lang.reflect.Array.getLength(value); i++) items.add(java.lang.reflect.Array.get(value,i)); return json(items); }}
        if (value instanceof Iterable<?> iterable) {{ ArrayList<String> items = new ArrayList<>(); for (Object item : iterable) items.add(json(item)); return "[" + String.join(",", items) + "]"; }}
        return quote(value.toString());
    }}
    public static void main(String[] args) {{ {calls} }}
}}
"#
    ))
}

fn java_value(value: &Value, kind: Kind) -> Result<String> {
    Ok(match kind {
        Kind::Int | Kind::Bool => value.to_string(),
        Kind::String => value.to_string(),
        Kind::Ints | Kind::List | Kind::Tree => {
            let array = value.as_array().context("Java 数组参数无效")?;
            let values = array
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            match kind {
                Kind::Ints => format!("new int[]{{{values}}}"),
                Kind::List => format!("list(new int[]{{{values}}})"),
                Kind::Tree => format!("tree(new Integer[]{{{values}}})"),
                _ => unreachable!(),
            }
        }
    })
}

pub fn execute(
    mut command: Command,
    timeout: Duration,
    cancel: &AtomicBool,
) -> Result<(bool, String)> {
    if cancel.load(Ordering::Relaxed) {
        bail!("运行已取消");
    }
    hide_console(&mut command);
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .spawn()
        .context("无法启动工具进程；请确认对应编译器已安装")?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    fn drain(mut reader: impl Read) -> String {
        let mut retained = Vec::new();
        let mut buffer = [0; 8192];
        while let Ok(count) = reader.read(&mut buffer) {
            if count == 0 {
                break;
            }
            let remaining = (1024 * 1024_usize).saturating_sub(retained.len());
            retained.extend_from_slice(&buffer[..count.min(remaining)]);
        }
        String::from_utf8_lossy(&retained).into_owned()
    }
    let out = thread::spawn(move || drain(stdout));
    let err = thread::spawn(move || drain(stderr));
    let started = Instant::now();
    let mut stopped = None;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if cancel.load(Ordering::Relaxed) || started.elapsed() > timeout {
            stopped = Some(if cancel.load(Ordering::Relaxed) {
                "运行已取消"
            } else {
                "运行超时"
            });
            #[cfg(windows)]
            {
                let mut kill = Command::new("taskkill");
                kill.args(["/PID", &child.id().to_string(), "/T", "/F"]);
                hide_console(&mut kill);
                let _ = kill.stdout(Stdio::null()).stderr(Stdio::null()).status();
            }
            let _ = child.kill();
            break child.wait()?;
        }
        thread::sleep(Duration::from_millis(30));
    };
    let output = format!(
        "{}{}",
        out.join().unwrap_or_default(),
        err.join().unwrap_or_default()
    );
    if let Some(reason) = stopped {
        bail!("{reason}\n{output}");
    }
    Ok((status.success(), output))
}

pub fn format_go(source: &str) -> Result<String> {
    let mut command = tool_command(gofmt_executable());
    hide_console(&mut command);
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("无法启动 gofmt；请安装 Go，或设置 LEETDESK_GOFMT")?;
    let mut input = child.stdin.take().unwrap();
    let source = source.to_owned();
    let writer = thread::spawn(move || input.write_all(source.as_bytes()));
    use std::io::Write;
    let output = child.wait_with_output()?;
    writer
        .join()
        .map_err(|_| anyhow::anyhow!("gofmt 输入线程失败"))??;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr));
    }
    Ok(String::from_utf8(output.stdout)?)
}

pub fn format_java(source: &str) -> Result<String> {
    use std::io::Write;
    let tool =
        std::env::var_os("LEETDESK_JAVA_FORMAT").unwrap_or_else(|| "google-java-format".into());
    let mut command = Command::new(tool);
    hide_console(&mut command);
    let mut child = command
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("无法启动 google-java-format；请安装它或设置 LEETDESK_JAVA_FORMAT")?;
    let mut input = child.stdin.take().unwrap();
    let text = source.to_owned();
    let writer = thread::spawn(move || input.write_all(text.as_bytes()));
    let output = child.wait_with_output()?;
    writer
        .join()
        .map_err(|_| anyhow::anyhow!("Java 格式化输入线程失败"))??;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr));
    }
    Ok(String::from_utf8(output.stdout)?)
}

pub fn organize_go_imports(source: &str, workspace: &Path) -> Result<String> {
    let temporary = tempfile::Builder::new()
        .prefix(".leetdesk-imports-")
        .tempdir_in(workspace)?;
    let path = temporary.path().join("solution.go");
    fs::write(&path, source)?;
    let mut command = tool_command(gopls_executable());
    command.arg("imports").arg("-w").arg(&path);
    let (success, output) = execute(command, Duration::from_secs(45), &AtomicBool::new(false))
        .context("无法整理导入；请安装 gopls")?;
    if !success {
        bail!("gopls 整理导入失败：{}", output.trim());
    }
    Ok(fs::read_to_string(path)?)
}

pub fn harness(problem: &Problem, cases: &[Case]) -> Result<String> {
    validate_cases(cases, &problem.parameters)?;
    let cases_literal = serde_json::to_string(&serde_json::to_string(cases)?)?;
    let mut arguments = vec![];
    let mut decode = String::new();
    for (i, parameter) in problem.parameters.iter().enumerate() {
        decode.push_str(&format!("var arg{i} {}\nif err := json.Unmarshal(tc.Input[{i}], &arg{i}); err != nil {{ panic(err) }}\n", parameter.kind.go_type()));
        arguments.push(match parameter.kind {
            Kind::List => format!("leetdeskList(arg{i})"),
            Kind::Tree => format!("leetdeskTree(arg{i})"),
            _ => format!("arg{i}"),
        });
    }
    let call = format!("{}({})", problem.function, arguments.join(", "));
    let result = match problem.returns {
        Kind::List => format!("leetdeskListValues({call})"),
        Kind::Tree => format!("leetdeskTreeValues({call})"),
        _ => call,
    };
    let template = include_str!("../assets/harness.go.txt");
    Ok(template
        .replace("{{CASES}}", &cases_literal)
        .replace("{{DECODE}}", &decode)
        .replace("{{CALL}}", &result)
        .replace(
            "{{UNORDERED}}",
            if problem.unordered { "true" } else { "false" },
        ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn harness_uses_typed_arguments_and_node_adapters() {
        let problems = crate::catalog::builtin().unwrap();
        let linked = problems.iter().find(|p| p.id == 206).unwrap();
        let generated = harness(linked, &linked.cases).unwrap();
        assert!(generated.contains("leetdeskListValues(reverseList(leetdeskList(arg0)))"));
        assert!(!generated.contains("{{"));
    }
    #[test]
    fn java_runner_compiles_and_checks_arrays_lists_and_trees() {
        if Command::new("javac").arg("-version").output().is_err() {
            return;
        }
        let problems = crate::catalog::builtin().unwrap();
        for (id, source, expected) in [
            (
                206,
                "class Solution { public ListNode reverseList(ListNode head) { ListNode prev = null; while (head != null) { ListNode next = head.next; head.next = prev; prev = head; head = next; } return prev; } }",
                3,
            ),
            (
                104,
                "class Solution { public int maxDepth(TreeNode root) { return root == null ? 0 : 1 + Math.max(maxDepth(root.left), maxDepth(root.right)); } }",
                3,
            ),
        ] {
            let problem = problems.iter().find(|problem| problem.id == id).unwrap();
            let temp = tempfile::tempdir().unwrap();
            let workspace = temp.path().join("workspace");
            fs::create_dir_all(&workspace).unwrap();
            let result = run_java(
                problem,
                source,
                &problem.cases,
                &workspace,
                Arc::new(AtomicBool::new(false)),
            )
            .unwrap();
            assert_eq!(result.passed, expected, "{}", result.output);
        }
    }
}
