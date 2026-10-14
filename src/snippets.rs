//! Local Go completion templates. These stay available while gopls is starting.
use crate::language::byte_position;
use lsp_types::{CompletionItem, CompletionItemKind, CompletionTextEdit, TextEdit};
use serde_json::json;
use std::ops::Range;

struct Template {
    prefix: &'static str,
    label: &'static str,
    detail: &'static str,
    body: &'static str,
}

const TEMPLATES: &[Template] = &[
    Template { prefix: "for", label: "for · 计数循环", detail: "for i := 0; i < n; i++", body: "for i := 0; i < ${1:n}; i++ {\n\t$0\n}" },
    Template { prefix: "forr", label: "forr · range 循环", detail: "遍历切片、映射或字符串", body: "for ${1:_, value} := range ${2:values} {\n\t$0\n}" },
    Template { prefix: "forc", label: "forc · 条件循环", detail: "for condition", body: "for ${1:condition} {\n\t$0\n}" },
    Template { prefix: "make", label: "make · 切片", detail: "make([]T, n)", body: "make([]${1:int}, ${2:n})$0" },
    Template { prefix: "makemap", label: "makemap · 映射", detail: "make(map[K]V)", body: "make(map[${1:string}]${2:int})$0" },
    Template { prefix: "makechan", label: "makechan · 通道", detail: "make(chan T)", body: "make(chan ${1:int})$0" },
    Template { prefix: "append", label: "append · 追加元素", detail: "append(slice, value)", body: "append(${1:slice}, ${2:value})$0" },
    Template { prefix: "if", label: "if · 条件分支", detail: "if condition", body: "if ${1:condition} {\n\t$0\n}" },
    Template { prefix: "switch", label: "switch · 分支", detail: "switch value", body: "switch ${1:value} {\ncase ${2:caseValue}:\n\t$0\n}" },
];

/// Render the subset of snippet syntax used by our templates into ordinary Go
/// text and byte ranges for editable fields. Byte ranges remain UTF-8 safe.
fn expand(template: &str, indent: &str) -> (String, Vec<Range<usize>>) {
    let mut result = String::new();
    let mut fields = Vec::new();
    let mut final_cursor = None;
    let mut chars = template.char_indices().peekable();
    while let Some((_, ch)) = chars.next() {
        if ch == '$' {
            if let Some(&(_, '{')) = chars.peek() {
                chars.next();
                let start = chars.peek().map_or(template.len(), |(index, _)| *index);
                if let Some(end) = template[start..].find('}') {
                    let inside = &template[start..start + end];
                    if let Some((number, default)) = inside.split_once(':')
                        && let Ok(number) = number.parse::<usize>()
                    {
                        let begin = result.len();
                        result.push_str(default);
                        fields.push((number, begin..result.len()));
                        while chars.peek().is_some_and(|(index, _)| *index <= start + end) {
                            chars.next();
                        }
                        continue;
                    }
                }
                result.push_str("${");
                continue;
            }
            if let Some(&(_, '0')) = chars.peek() {
                chars.next();
                final_cursor = Some(result.len());
                continue;
            }
        }
        result.push(ch);
        if ch == '\n' {
            result.push_str(indent);
        }
    }
    fields.sort_by_key(|(number, _)| *number);
    let mut ranges: Vec<_> = fields.into_iter().map(|(_, range)| range).collect();
    if let Some(at) = final_cursor {
        ranges.push(at..at);
    }
    (result, ranges)
}

pub fn completions(source: &str, offset: usize) -> Vec<CompletionItem> {
    let Some(prefix_source) = source.get(..offset) else { return vec![] };
    let word_bytes: usize = prefix_source.chars().rev()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .map(char::len_utf8).sum();
    let start = offset - word_bytes;
    if start > 0 && source.as_bytes()[start - 1] == b'.' { return vec![]; }
    let query = &source[start..offset];
    let line_start = prefix_source.rfind('\n').map_or(0, |at| at + 1);
    let line = &source[line_start..start];
    let indent = line.chars().take_while(|ch| *ch == ' ' || *ch == '\t').collect::<String>();
    TEMPLATES.iter().enumerate().filter(|(_, template)| {
        query.is_empty() || template.prefix.starts_with(query)
    }).map(|(rank, template)| {
        let (body, fields) = expand(template.body, &indent);
        CompletionItem {
            label: template.label.into(),
            kind: Some(CompletionItemKind::SNIPPET),
            detail: Some(template.detail.into()),
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
    fn go_templates_have_real_tab_stops_and_keep_indentation() {
        let source = "func solve() {\n\tfo";
        let items = completions(source, source.len());
        let item = items.iter().find(|item| item.label.starts_with("for ·")).unwrap();
        let CompletionTextEdit::Edit(edit) = item.text_edit.as_ref().unwrap() else { panic!("expected edit") };
        assert_eq!(edit.range.start, lsp_types::Position::new(1, 1));
        assert_eq!(edit.new_text, "for i := 0; i < n; i++ {\n\t\t\n\t}");
        assert_eq!(item.data.as_ref().unwrap()["leetdesk_tabstops"], json!([[16,17],[27,27]]));
        let make = completions("make", 4);
        assert!(make.iter().any(|item| item.label == "make · 切片"));
        assert!(make.iter().any(|item| item.label == "makemap · 映射"));
        let append = completions("append", 6);
        assert_eq!(append.len(), 1);
        assert_eq!(append[0].data.as_ref().unwrap()["leetdesk_tabstops"], json!([[7,12],[14,19],[20,20]]));
        assert!(completions("fmt.app", 7).is_empty());
    }
}
