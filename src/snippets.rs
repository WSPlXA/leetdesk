//! Local Go completion templates. These stay available while gopls is starting.
use crate::language::{byte_position, position_byte};
use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionTextEdit, InsertTextFormat, TextEdit,
};
use serde_json::json;
use std::{iter::Peekable, ops::Range, str::Chars, sync::OnceLock};

struct Template {
    prefix: &'static str,
    label: &'static str,
    detail: &'static str,
    body: &'static str,
}

const TEMPLATES: &[Template] = &[
    Template {
        prefix: "for",
        label: "for · 计数循环",
        detail: "for i := 0; i < n; i++",
        body: "for i := 0; i < ${1:n}; i++ {\n\t$0\n}",
    },
    Template {
        prefix: "forr",
        label: "forr · range 循环",
        detail: "遍历切片、映射或字符串",
        body: "for ${1:_, value} := range ${2:values} {\n\t$0\n}",
    },
    Template {
        prefix: "forc",
        label: "forc · 条件循环",
        detail: "for condition",
        body: "for ${1:condition} {\n\t$0\n}",
    },
    Template {
        prefix: "make",
        label: "make · 切片",
        detail: "make([]T, n)",
        body: "make([]${1:int}, ${2:n})$0",
    },
    Template {
        prefix: "makemap",
        label: "makemap · 映射",
        detail: "make(map[K]V)",
        body: "make(map[${1:string}]${2:int})$0",
    },
    Template {
        prefix: "makechan",
        label: "makechan · 通道",
        detail: "make(chan T)",
        body: "make(chan ${1:int})$0",
    },
    Template {
        prefix: "append",
        label: "append · 追加元素",
        detail: "append(slice, value)",
        body: "append(${1:slice}, ${2:value})$0",
    },
    Template {
        prefix: "if",
        label: "if · 条件分支",
        detail: "if condition",
        body: "if ${1:condition} {\n\t$0\n}",
    },
    Template {
        prefix: "switch",
        label: "switch · 分支",
        detail: "switch value",
        body: "switch ${1:value} {\ncase ${2:caseValue}:\n\t$0\n}",
    },
];

const JAVA_TEMPLATES: &[Template] = &[
    Template {
        prefix: "for",
        label: "for · 计数循环",
        detail: "for (int i = 0; i < n; i++)",
        body: "for (int ${1:i} = 0; ${1:i} < ${2:n}; ${1:i}++) {\n    $0\n}",
    },
    Template {
        prefix: "foreach",
        label: "foreach · 遍历数组",
        detail: "for (int value : values)",
        body: "for (int ${1:value} : ${2:values}) {\n    $0\n}",
    },
    Template {
        prefix: "if",
        label: "if · 条件分支",
        detail: "if (condition)",
        body: "if (${1:condition}) {\n    $0\n}",
    },
    Template {
        prefix: "while",
        label: "while · 循环",
        detail: "while (condition)",
        body: "while (${1:condition}) {\n    $0\n}",
    },
    Template {
        prefix: "newmap",
        label: "newmap · HashMap",
        detail: "new HashMap<>()",
        body: "new java.util.HashMap<${1:Integer}, ${2:Integer}>()$0",
    },
    Template {
        prefix: "newlist",
        label: "newlist · ArrayList",
        detail: "new ArrayList<>()",
        body: "new java.util.ArrayList<${1:Integer}>()$0",
    },
];

/// Render local and gopls snippets (numeric stops, nested defaults and escapes)
/// into text plus UTF-8 byte ranges understood by the editor's snippet session.
fn expand(template: &str, indent: &str) -> (String, Vec<Range<usize>>) {
    let mut result = String::new();
    let mut fields = Vec::new();
    render(
        &mut template.chars().peekable(),
        indent,
        &mut result,
        &mut fields,
        false,
    );
    fields.sort_by_key(|(number, _)| (*number == 0, *number));
    let has_final = fields.iter().any(|(number, _)| *number == 0);
    let mut ranges: Vec<_> = fields.into_iter().map(|(_, range)| range).collect();
    if !has_final {
        ranges.push(result.len()..result.len());
    }
    (result, ranges)
}

fn render(
    chars: &mut Peekable<Chars<'_>>,
    indent: &str,
    result: &mut String,
    fields: &mut Vec<(usize, Range<usize>)>,
    nested: bool,
) {
    while let Some(ch) = chars.next() {
        if ch == '}' && nested {
            return;
        }
        if ch == '\\'
            && chars
                .peek()
                .is_some_and(|ch| matches!(ch, '\\' | '$' | '}'))
        {
            result.push(chars.next().unwrap());
            continue;
        }
        if ch == '$' {
            let mut lookahead = chars.clone();
            let braced = lookahead.next_if_eq(&'{').is_some();
            let mut digits = String::new();
            while let Some(digit) = lookahead.next_if(char::is_ascii_digit) {
                digits.push(digit);
            }
            if let Ok(number) = digits.parse::<usize>()
                && (!braced || matches!(lookahead.peek(), Some(':' | '}')))
            {
                *chars = lookahead;
                let start = result.len();
                if braced && chars.next() == Some(':') {
                    render(chars, indent, result, fields, true);
                }
                fields.push((number, start..result.len()));
                continue;
            }
        }
        result.push(ch);
        if ch == '\n' {
            result.push_str(indent);
        }
    }
}

/// Convert server snippets before passing them to the editor, which accepts
/// ordinary text and explicit stops. Keep server ranking and additional edits.
pub fn merge_go_completions(
    source: &str,
    mut local: Vec<CompletionItem>,
    remote: Vec<CompletionItem>,
) -> Vec<CompletionItem> {
    let local_append = local
        .iter()
        .any(|item| item.filter_text.as_deref() == Some("append"));
    for mut item in remote {
        if local_append && item.label == "append" {
            continue;
        }
        // gopls may offer a func-typed variable only as a value. Add a call
        // using its resolved signature and keep the value for callback uses.
        let call = callable_variable(source, &item);
        if let Some((call, true)) = &call {
            local.push(call.clone());
        }
        if item.insert_text_format == Some(InsertTextFormat::SNIPPET) {
            let text = match item.text_edit.as_mut() {
                Some(CompletionTextEdit::Edit(edit)) => &mut edit.new_text,
                Some(CompletionTextEdit::InsertAndReplace(edit)) => &mut edit.new_text,
                None => item.insert_text.get_or_insert_with(|| item.label.clone()),
            };
            let (body, fields) = expand(text, "");
            *text = body;
            item.insert_text_format = Some(InsertTextFormat::PLAIN_TEXT);
            let mut data = item
                .data
                .take()
                .filter(|value| value.is_object())
                .unwrap_or_else(|| json!({}));
            data["leetdesk_tabstops"] = json!(
                fields
                    .iter()
                    .map(|range| [range.start, range.end])
                    .collect::<Vec<_>>()
            );
            item.data = Some(data);
        }
        local.push(item);
        if let Some((call, false)) = call {
            local.push(call);
        }
    }
    local
}

fn callable_variable(source: &str, item: &CompletionItem) -> Option<(CompletionItem, bool)> {
    if !matches!(
        item.kind,
        Some(CompletionItemKind::VARIABLE | CompletionItemKind::FIELD)
    ) {
        return None;
    }
    let signature = item.detail.as_deref()?.strip_prefix("func(")?;
    let mut depth = 0usize;
    let mut start = 0;
    let mut params = Vec::new();
    for (at, ch) in signature.char_indices() {
        match ch {
            ')' if depth == 0 => {
                if at > start {
                    params.push(signature[start..at].trim());
                }
                break;
            }
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.checked_sub(1)?,
            ',' if depth == 0 => {
                params.push(signature[start..at].trim());
                start = at + 1;
            }
            _ => {}
        }
    }
    let mut call = item.clone();
    let (text, range) = match call.text_edit.as_mut()? {
        CompletionTextEdit::Edit(edit) => (&mut edit.new_text, edit.range),
        CompletionTextEdit::InsertAndReplace(edit) => (&mut edit.new_text, edit.replace),
    };
    if text != &item.label
        || source
            .get(position_byte(source, range.end)..)?
            .trim_start()
            .starts_with('(')
    {
        return None;
    }
    let mut fields = Vec::new();
    text.push('(');
    for (index, param) in params.iter().enumerate() {
        if index > 0 {
            text.push_str(", ");
        }
        let param = param.split_once(' ').map_or(*param, |(name, _)| {
            if !matches!(name, "chan" | "struct" | "interface" | "func" | "map")
                && name.chars().all(|ch| ch.is_alphanumeric() || ch == '_')
            {
                name
            } else {
                param
            }
        });
        let start = text.len();
        text.push_str(param);
        fields.push([start, text.len()]);
    }
    text.push(')');
    fields.push([text.len(), text.len()]);
    call.label = format!("{}(…)", item.label);
    call.kind = Some(CompletionItemKind::FUNCTION);
    call.filter_text = Some(item.label.clone());
    call.insert_text_format = Some(InsertTextFormat::PLAIN_TEXT);
    let mut data = item
        .data
        .clone()
        .filter(|data| data.is_object())
        .unwrap_or_else(|| json!({}));
    data["leetdesk_tabstops"] = json!(fields);
    call.data = Some(data);
    let before = &source[..position_byte(source, range.start)];
    let statement = before.rsplit(['\n', ';', '{', '}']).next()?.trim();
    Some((call, matches!(statement, "" | "go" | "defer")))
}

fn append_target(before_word: &str) -> Option<&str> {
    // Only reuse a simple assignment destination. Restrict indexes to names or
    // integers so completing append never duplicates a call with side effects.
    static ASSIGNMENT: OnceLock<regex::Regex> = OnceLock::new();
    let pattern = ASSIGNMENT.get_or_init(|| regex::Regex::new(
        r"(?:^|[\n;{}])\s*(?P<target>[\p{L}_][\p{L}\p{N}_]*(?:\s*\.\s*[\p{L}_][\p{L}\p{N}_]*|\s*\[\s*[\p{L}\p{N}_]+\s*\])*)\s*=\s*$"
    ).expect("valid assignment pattern"));
    pattern
        .captures(before_word)?
        .name("target")
        .map(|target| target.as_str())
}

pub fn completions(source: &str, offset: usize) -> Vec<CompletionItem> {
    complete(source, offset, TEMPLATES)
}

pub fn java_completions(source: &str, offset: usize) -> Vec<CompletionItem> {
    complete(source, offset, JAVA_TEMPLATES)
}

fn complete(source: &str, offset: usize, templates: &[Template]) -> Vec<CompletionItem> {
    let Some(prefix_source) = source.get(..offset) else {
        return vec![];
    };
    let word_bytes: usize = prefix_source
        .chars()
        .rev()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .map(char::len_utf8)
        .sum();
    let start = offset - word_bytes;
    if start > 0 && source.as_bytes()[start - 1] == b'.' {
        return vec![];
    }
    let query = &source[start..offset];
    let line_start = prefix_source.rfind('\n').map_or(0, |at| at + 1);
    let line = &source[line_start..start];
    let indent = line
        .chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .collect::<String>();
    templates.iter().enumerate().filter(|(_, template)| {
        (query.is_empty() || template.prefix.starts_with(query))
            && !(template.prefix == "append" && source[offset..].trim_start().starts_with('('))
    }).map(|(rank, template)| {
        let target = (template.prefix == "append").then(|| append_target(&source[..start])).flatten();
        let (body, fields) = if let Some(target) = target {
            let body = format!("append({target}, value)");
            let value = body.len() - 6..body.len() - 1;
            let end = body.len();
            (body, vec![value, end..end])
        } else {
            expand(template.body, &indent)
        };
        CompletionItem {
            label: template.label.into(),
            kind: Some(CompletionItemKind::SNIPPET),
            detail: Some(target.map_or_else(|| template.detail.into(), |target| format!("append({target}, value)"))),
            filter_text: Some(template.prefix.into()),
            sort_text: Some(format!("!{rank:02}")),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                range: lsp_types::Range { start: byte_position(source, start), end: byte_position(source, offset) },
                new_text: body,
            })),
            data: Some(json!({"leetdesk_tabstops": fields.iter().map(|range| [range.start, range.end]).collect::<Vec<_>>() })),
            ..Default::default()
        }
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_reuses_assignment_target_and_starts_at_value() {
        for target in [
            "q",
            "queue",
            "队列",
            "state.queue",
            "queues[i]",
            "queues[0]",
        ] {
            let source = format!("func solve() {{\n\t{target} = app");
            let items = completions(&source, source.len());
            let item = &items[0];
            let CompletionTextEdit::Edit(edit) = item.text_edit.as_ref().unwrap() else {
                panic!("expected edit")
            };
            assert_eq!(edit.new_text, format!("append({target}, value)"));
            let stops = &item.data.as_ref().unwrap()["leetdesk_tabstops"];
            let start = stops[0][0].as_u64().unwrap() as usize;
            let end = stops[0][1].as_u64().unwrap() as usize;
            assert_eq!(&edit.new_text[start..end], "value");
            assert_eq!(stops.as_array().unwrap().len(), 2);
        }
        for before in ["q =\n\t", "q=", "if ok { q = ", "f(); q = "] {
            let source = format!("{before}app");
            let CompletionTextEdit::Edit(edit) = completions(&source, source.len())[0]
                .text_edit
                .clone()
                .unwrap()
            else {
                panic!("expected edit")
            };
            assert_eq!(edit.new_text, "append(q, value)");
        }
        for source in [
            "return app",
            "other(q, app",
            "q, err = app",
            "q == app",
            "q := app",
            "queues[next()] = app",
        ] {
            let CompletionTextEdit::Edit(edit) = completions(source, source.len())[0]
                .text_edit
                .clone()
                .unwrap()
            else {
                panic!("expected edit")
            };
            assert_eq!(edit.new_text, "append(slice, value)");
        }
        assert!(completions("q = app(q, 1)", 7).is_empty());
    }

    #[test]
    fn server_snippets_expand_function_arguments_escapes_and_nested_defaults() {
        let (text, fields) = expand(r"xxx(${1:int}, ${2:struct{ Value string \}})$0", "");
        assert_eq!(text, "xxx(int, struct{ Value string })");
        assert_eq!(&text[fields[0].clone()], "int");
        assert_eq!(&text[fields[1].clone()], "struct{ Value string }");
        assert_eq!(fields[2], text.len()..text.len());
        assert_eq!(expand("xxx(${1:})", ""), ("xxx()".into(), vec![4..4, 5..5]));
        assert_eq!(expand("xxx($1, ${2})", "").0, "xxx(, )");
        assert_eq!(
            expand(r"\$literal \\ ${1:中文🙂} ${2:${3:int}}", "").0,
            "$literal \\ 中文🙂 int"
        );
    }

    #[test]
    fn merging_keeps_callable_variables_and_avoids_duplicate_append() {
        let remote = vec![CompletionItem {
            label: "xxx".into(),
            kind: Some(CompletionItemKind::VARIABLE),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                range: Default::default(),
                new_text: "xxx(${1:int})".into(),
            })),
            data: Some(json!({"server": "preserved"})),
            ..Default::default()
        }];
        let merged = merge_go_completions("", vec![], remote);
        let item = merged.iter().find(|item| item.label == "xxx").unwrap();
        assert_eq!(item.data.as_ref().unwrap()["server"], "preserved");
        assert_eq!(
            item.data.as_ref().unwrap()["leetdesk_tabstops"],
            json!([[4, 7], [8, 8]])
        );
        let local = completions("q = app", 7);
        let merged = merge_go_completions(
            "q = app",
            local,
            vec![CompletionItem {
                label: "append".into(),
                ..Default::default()
            }],
        );
        assert_eq!(merged.len(), 1);
    }
    #[test]
    fn go_templates_have_real_tab_stops_and_keep_indentation() {
        let source = "func solve() {\n\tfo";
        let items = completions(source, source.len());
        let item = items
            .iter()
            .find(|item| item.label.starts_with("for ·"))
            .unwrap();
        let CompletionTextEdit::Edit(edit) = item.text_edit.as_ref().unwrap() else {
            panic!("expected edit")
        };
        assert_eq!(edit.range.start, lsp_types::Position::new(1, 1));
        assert_eq!(edit.new_text, "for i := 0; i < n; i++ {\n\t\t\n\t}");
        assert_eq!(
            item.data.as_ref().unwrap()["leetdesk_tabstops"],
            json!([[16, 17], [27, 27]])
        );
        let make = completions("make", 4);
        assert!(make.iter().any(|item| item.label == "make · 切片"));
        assert!(make.iter().any(|item| item.label == "makemap · 映射"));
        let append = completions("append", 6);
        assert_eq!(append.len(), 1);
        assert_eq!(
            append[0].data.as_ref().unwrap()["leetdesk_tabstops"],
            json!([[7, 12], [14, 19], [20, 20]])
        );
        assert!(completions("fmt.app", 7).is_empty());
    }
    #[test]
    fn java_loop_and_collections_complete_with_tab_stops() {
        let source = "class Solution {\n    fo";
        let items = java_completions(source, source.len());
        let loop_item = items
            .iter()
            .find(|item| item.label.starts_with("for ·"))
            .unwrap();
        let CompletionTextEdit::Edit(edit) = loop_item.text_edit.as_ref().unwrap() else {
            panic!("expected edit")
        };
        assert!(edit.new_text.contains("for (int i = 0; i < n; i++)"));
        assert!(
            loop_item.data.as_ref().unwrap()["leetdesk_tabstops"]
                .as_array()
                .unwrap()
                .len()
                >= 2
        );
        assert!(
            java_completions("newmap", 6)
                .iter()
                .any(|item| item.label.starts_with("newmap"))
        );
    }
}
