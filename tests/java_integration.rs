//! Real Java language-server checks. Install with scripts/install-java-lsp.ps1.
//! cargo +stable-x86_64-pc-windows-msvc test --test java_integration -- --ignored --nocapture
use leetdesk::{
    catalog::PracticeLanguage,
    language::{self, LanguageService},
    snippets,
};
use lsp_types::{CompletionResponse, CompletionTextEdit, DiagnosticSeverity};
use std::{
    fs, thread,
    time::{Duration, Instant},
};

fn wait_for(
    service: &LanguageService,
    predicate: impl Fn(&language::Diagnostics) -> bool,
) -> language::Diagnostics {
    let start = Instant::now();
    loop {
        let snapshot = service.diagnostics();
        if snapshot.generation > 0
            && snapshot.version.is_none_or(|v| v == service.version())
            && predicate(&snapshot)
        {
            return snapshot;
        }
        assert!(
            start.elapsed() < Duration::from_secs(90),
            "diagnostics={snapshot:?}, error={:?}",
            service.error()
        );
        thread::sleep(Duration::from_millis(100));
    }
}

#[test]
#[ignore = "requires Java 25+ and JDT LS"]
fn java_diagnostics_completion_hover_and_definition_use_the_unsaved_draft() {
    let temp = tempfile::tempdir_in("target").unwrap();
    // The commented LeetCode node declaration must not hide the provided type.
    let good = "/** class ListNode { } */\nclass Solution {\n    public ListNode solve(ListNode head) {\n        List<Integer> values = new ArrayList<>();\n        values.add(1);\n        return head;\n    }\n}\n";
    fs::write(temp.path().join("Solution.java"), good).unwrap();
    let service = LanguageService::start_for(temp.path(), good, PracticeLanguage::Java)
        .unwrap_or_else(|error| {
            panic!(
                "{error:#}\n{}",
                String::from_utf8_lossy(
                    &fs::read(temp.path().join("JDT LS.log")).unwrap_or_default()
                )
            )
        });
    wait_for(&service, |d| {
        d.items
            .iter()
            .all(|d| d.severity != Some(DiagnosticSeverity::ERROR))
    });
    let hover = service
        .request(
            "textDocument/hover",
            service.at(good, good.find("ListNode head").unwrap() + 2),
        )
        .unwrap();
    assert!(hover.to_string().contains("ListNode"), "{hover}");
    let definition = service
        .request(
            "textDocument/definition",
            service.at(good, good.find("ListNode head").unwrap() + 2),
        )
        .unwrap();
    assert!(
        definition.to_string().contains("ListNode.java"),
        "{definition}"
    );
    // Local definitions need the two implicit import lines removed from their coordinates.
    let definition = service
        .request(
            "textDocument/definition",
            service.at(good, good.find("return head").unwrap() + 8),
        )
        .unwrap();
    assert_eq!(definition[0]["range"]["start"]["line"], 2, "{definition}");

    let bad = good.replace("return head", "return missingName");
    service.sync(&bad).unwrap();
    let snapshot = wait_for(&service, |d| {
        d.items.iter().any(|d| d.message.contains("missingName"))
    });
    let diagnostic = snapshot
        .items
        .iter()
        .find(|d| d.message.contains("missingName"))
        .unwrap();
    assert_eq!(
        diagnostic.range.start,
        language::byte_position(&bad, bad.find("missingName").unwrap())
    );
    service.sync(good).unwrap();
    wait_for(&service, |d| {
        d.items
            .iter()
            .all(|d| d.severity != Some(DiagnosticSeverity::ERROR))
    });
    let warning = good.replace("return head;", "int unused = 1;\n        return head;");
    service.sync(&warning).unwrap();
    wait_for(&service, |d| {
        d.items.iter().any(|d| {
            d.severity == Some(DiagnosticSeverity::WARNING) && d.message.contains("unused")
        })
    });

    for (draft, prefix, expected) in [
        (
            good.replace("return head;", "head.ne; return head;"),
            "head.ne",
            "next",
        ),
        (
            good.replace("return head;", "values.ad; return head;"),
            "values.ad",
            "add",
        ),
    ] {
        service.sync(&draft).unwrap();
        let offset = draft.rfind(prefix).unwrap() + prefix.len();
        let value = service
            .request("textDocument/completion", service.at(&draft, offset))
            .unwrap();
        let response: CompletionResponse = serde_json::from_value(value).unwrap();
        let remote = match response {
            CompletionResponse::Array(v) => v,
            CompletionResponse::List(v) => v.items,
        };
        let items = snippets::merge_java_completions(vec![], remote);
        let item = items
            .iter()
            .find(|i| i.label.starts_with(expected))
            .unwrap_or_else(|| panic!("missing {expected}: {items:?}"));
        let edit = match item.text_edit.as_ref().unwrap() {
            CompletionTextEdit::Edit(e) => e.clone(),
            CompletionTextEdit::InsertAndReplace(e) => lsp_types::TextEdit {
                range: e.replace,
                new_text: e.new_text.clone(),
            },
        };
        assert_eq!(edit.range.end, language::byte_position(&draft, offset));
        let inserted = language::apply_edits(&draft, &[edit]).unwrap();
        assert!(
            inserted.contains(&format!(
                "{}.{}",
                prefix.split('.').next().unwrap(),
                expected
            )),
            "{inserted}"
        );
        assert!(!inserted.contains("${"), "{inserted}");
    }
    // Language tooling must never write its synthetic imports or helpers into the draft.
    let formatted = service
        .format_java("class Solution{int solve(){return 1;}}")
        .unwrap();
    assert!(formatted.contains("class Solution {"), "{formatted}");
    assert!(!formatted.contains("import java.util"), "{formatted}");
    assert!(!formatted.contains("import java.math"), "{formatted}");
    assert_eq!(
        fs::read_to_string(temp.path().join("Solution.java")).unwrap(),
        good
    );
    drop(service);
    assert!(!fs::read_dir(temp.path()).unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("leetdesk-java-")
    }));
}
