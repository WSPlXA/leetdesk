#![cfg_attr(windows, windows_subsystem = "windows")]

extern crate gpui_kit as gpui;
mod auth;
mod desktop;
mod markdown;
mod theme;
mod ui;

use anyhow::{Context, Result};
use leetdesk::store::Store;
use std::{path::PathBuf, process::Command};

fn main() {
    // WebView2 is a child HWND; DirectComposition would cover it on Windows.
    #[cfg(windows)]
    unsafe {
        std::env::set_var("GPUI_DISABLE_DIRECT_COMPOSITION", "true");
    }
    if let Err(error) = run() {
        eprintln!("LeetDesk: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!(
            "LeetDesk — GPUI + Rust / Go、Java 刷题工作区\n\n启动：leetdesk [--data-dir PATH]\n检查工具：leetdesk --doctor\n导入题目：leetdesk --import-catalog PATH\n\n快捷键：Ctrl+S 保存 · Ctrl+Enter 运行 · Ctrl+Shift+F 格式化\nCtrl+Shift+O 整理 Go 导入 · Ctrl+Space 补全 · F12 定义 · Ctrl+I 导入 MD"
        );
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--doctor") {
        for (label, tool, argument) in [
            ("Go", std::ffi::OsString::from("go"), "version"),
            ("gopls", leetdesk::language::gopls_executable(), "version"),
            ("gofmt", leetdesk::language::gofmt_executable(), "-h"),
            ("Java", std::ffi::OsString::from("java"), "-version"),
            ("javac", std::ffi::OsString::from("javac"), "-version"),
            (
                "google-java-format",
                std::env::var_os("LEETDESK_JAVA_FORMAT")
                    .unwrap_or_else(|| "google-java-format".into()),
                "--version",
            ),
        ] {
            let mut command = Command::new(tool);
            leetdesk::language::hide_console(&mut command);
            match command.arg(argument).output() {
                Ok(output) => println!(
                    "{label}: {}",
                    String::from_utf8_lossy(if output.stdout.is_empty() {
                        &output.stderr
                    } else {
                        &output.stdout
                    })
                    .trim()
                ),
                Err(error) => println!("{label}: 不可用 ({error})"),
            }
        }
        return Ok(());
    }
    let root = if let Some(index) = args.iter().position(|arg| arg == "--data-dir") {
        PathBuf::from(args.get(index + 1).context("--data-dir 需要路径")?)
    } else if let Some(path) = std::env::var_os("LEETDESK_DATA") {
        PathBuf::from(path)
    } else {
        default_data_dir()?
    };
    let store = Store::open(root)?;
    if let Some(index) = args.iter().position(|arg| arg == "--import-catalog") {
        let path = PathBuf::from(
            args.get(index + 1)
                .context("--import-catalog 需要 JSON 路径")?,
        );
        println!("已导入 {} 道题目", store.import_catalog(&path)?);
        return Ok(());
    }
    let smoke = args.iter().any(|arg| arg == "--smoke-test");
    if let Some(index) = args.iter().position(|arg| arg == "--design-capture") {
        let path = PathBuf::from(
            args.get(index + 1)
                .context("--design-capture needs a PNG path")?,
        );
        let dimension = |flag: &str, fallback: f32| -> Result<f32> {
            let value = args
                .iter()
                .position(|arg| arg == flag)
                .map(|i| {
                    args.get(i + 1)
                        .context("missing capture dimension")?
                        .parse::<f32>()
                        .context("invalid dimension")
                })
                .transpose()?
                .unwrap_or(fallback);
            anyhow::ensure!(
                value.is_finite() && (650.0..=3000.0).contains(&value),
                "capture dimension out of range"
            );
            Ok(value)
        };
        return desktop::capture(
            store,
            path,
            dimension("--width", 1488.)?,
            dimension("--height", 1057.)?,
        );
    }
    if smoke || args.iter().any(|arg| arg == "--offline") {
        desktop::launch(store, smoke)
    } else {
        let plan_probe = args.iter().any(|arg| arg == "--study-plan-check");
        auth::launch(
            store,
            plan_probe || args.iter().any(|arg| arg == "--online-check"),
            plan_probe,
        )
    }
}

fn default_data_dir() -> Result<PathBuf> {
    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME") {
        return Ok(PathBuf::from(home).join("Library/Application Support/LeetDesk"));
    }
    Ok(std::env::current_dir()?.join("data"))
}
