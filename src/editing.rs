//! Snapshot diffs for formatter edits. No display wrapping enters source text.
use similar::{ChangeTag, TextDiff};
use std::ops::Range;
pub type Edits = Vec<(Range<usize>, String)>;

pub fn diff(before: &str, after: &str) -> Edits {
    let mut edits = Vec::new();
    let mut offset = 0;
    let mut pending: Option<(Range<usize>, String)> = None;
    for change in TextDiff::from_chars(before, after).iter_all_changes() {
        let value = change.value();
        match change.tag() {
            ChangeTag::Equal => {
                if let Some(edit) = pending.take() {
                    edits.push(edit);
                }
                offset += value.len();
            }
            ChangeTag::Delete => {
                let edit = pending.get_or_insert_with(|| (offset..offset, String::new()));
                offset += value.len();
                edit.0.end = offset;
            }
            ChangeTag::Insert => {
                pending
                    .get_or_insert_with(|| (offset..offset, String::new()))
                    .1
                    .push_str(value);
            }
        }
    }
    if let Some(edit) = pending {
        edits.push(edit);
    }
    edits
}

#[cfg(test)]
mod tests {
    #[test]
    fn unicode_and_disjoint_edits_recreate_the_formatter_output() {
        let old = "package solution\nfunc f( )string{ return \"中文🙂\" }\n";
        let new = "package solution\n\nfunc f() string { return \"中文🙂\" }\n";
        let edits = super::diff(old, new);
        assert!(edits.len() > 1);
        let mut text = old.to_string();
        for (range, replacement) in edits.iter().rev() {
            text.replace_range(range.clone(), replacement);
        }
        assert_eq!(text, new);
        assert!(super::diff(new, new).is_empty());
    }
}
