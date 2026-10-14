use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Problem {
    pub id: u32,
    pub slug: String,
    pub title: String,
    pub difficulty: String,
    pub tags: Vec<String>,
    pub description: String,
    pub starter: String,
    pub function: String,
    pub parameters: Vec<Parameter>,
    pub returns: Kind,
    #[serde(default)]
    pub unordered: bool,
    pub cases: Vec<Case>,
    #[serde(default)]
    pub explanation: String,
    #[serde(default)]
    pub online: Option<crate::online::RemoteMeta>,
    #[serde(default)]
    pub extra_types: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Parameter {
    pub name: String,
    pub kind: Kind,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Int,
    Ints,
    String,
    Bool,
    List,
    Tree,
}

impl Kind {
    pub fn go_type(self) -> &'static str {
        match self {
            Self::Int => "int",
            Self::Ints | Self::List => "[]int",
            Self::String => "string",
            Self::Bool => "bool",
            Self::Tree => "[]*int",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Case {
    pub input: Vec<Value>,
    pub expected: Value,
}

pub fn builtin() -> Result<Vec<Problem>> {
    let problems: Vec<Problem> = serde_json::from_str(include_str!("../assets/problems.json"))?;
    for problem in &problems {
        validate(problem)?;
    }
    Ok(problems)
}

pub fn validate(problem: &Problem) -> Result<()> {
    if problem.online.is_some() {
        if problem.id == 0 || !crate::online::valid_slug(&problem.slug) {
            bail!("在线题目标识无效");
        }
        return Ok(());
    }
    if problem.id == 0 || !is_identifier(&problem.function) {
        bail!("题号或 Go 函数名无效");
    }
    for parameter in &problem.parameters {
        if !is_identifier(&parameter.name) {
            bail!("无效的参数名：{}", parameter.name);
        }
    }
    validate_cases(&problem.cases, &problem.parameters).context("题目测试用例无效")
}

fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some('a'..='z' | 'A'..='Z' | '_'))
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub fn validate_cases(cases: &[Case], parameters: &[Parameter]) -> Result<()> {
    if cases.is_empty() {
        bail!("至少需要一个测试用例");
    }
    for (i, case) in cases.iter().enumerate() {
        if case.input.len() != parameters.len() {
            bail!(
                "用例 {}：需要 {} 个参数，实际提供 {} 个",
                i + 1,
                parameters.len(),
                case.input.len()
            );
        }
        for (argument, parameter) in case.input.iter().zip(parameters) {
            let valid = match parameter.kind {
                Kind::Int => argument.as_i64().is_some(),
                Kind::Ints | Kind::List => argument
                    .as_array()
                    .is_some_and(|xs| xs.iter().all(|x| x.as_i64().is_some())),
                Kind::Tree => argument
                    .as_array()
                    .is_some_and(|xs| xs.iter().all(|x| x.is_null() || x.as_i64().is_some())),
                Kind::String => argument.is_string(),
                Kind::Bool => argument.is_boolean(),
            };
            if !valid {
                bail!("用例 {}：参数 {} 的类型不匹配", i + 1, parameter.name);
            }
        }
    }
    Ok(())
}

/// Association is explicit metadata first, then an exact LeetCode slug or file prefix.
/// An unassociated note is attached to the problem chosen in the import UI.
pub fn note_metadata(text: &str) -> (Option<u32>, Vec<String>) {
    let mut id = None;
    let mut tags = Vec::new();
    let normalized = text.trim_start_matches('\u{feff}');
    if normalized.lines().next() == Some("---") {
        for line in normalized.lines().skip(1).take_while(|line| *line != "---") {
            if let Some(value) = line
                .strip_prefix("problem_id:")
                .or_else(|| line.strip_prefix("leetcode_id:"))
            {
                id = value.trim().trim_matches(['\'', '"']).parse().ok();
            }
            if let Some(value) = line.strip_prefix("tags:") {
                tags = value
                    .trim()
                    .trim_matches(['[', ']'])
                    .split(',')
                    .map(|tag| tag.trim().trim_matches(['\'', '"']).to_string())
                    .filter(|tag| !tag.is_empty())
                    .collect();
            }
        }
    }
    (id, tags)
}

pub fn markdown_body(text: &str) -> &str {
    let text = text.trim_start_matches('\u{feff}');
    if text.starts_with("---\n") || text.starts_with("---\r\n") {
        let mut offset = text.find('\n').unwrap() + 1;
        for line in text[offset..].split_inclusive('\n') {
            offset += line.len();
            if line.trim_end() == "---" {
                return &text[offset..];
            }
        }
    }
    text
}

pub fn associate_note(filename: &str, text: &str, problems: &[Problem]) -> Option<u32> {
    if let Some(id) = note_metadata(text).0 {
        return problems.iter().any(|p| p.id == id).then_some(id);
    }
    let prefix: String = filename
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if let Ok(id) = prefix.parse::<u32>() {
        return problems.iter().any(|p| p.id == id).then_some(id);
    }
    let matches: Vec<_> = problems
        .iter()
        .filter(|p| {
            let needle = format!("/problems/{}", p.slug);
            text.match_indices(&needle).any(|(offset, _)| {
                text[offset + needle.len()..]
                    .chars()
                    .next()
                    .is_none_or(|c| !c.is_ascii_alphanumeric() && c != '-')
            })
        })
        .collect();
    (matches.len() == 1).then(|| matches[0].id)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_and_association_preserve_multi_tag_notes() {
        let text = "---\r\nproblem_id: 1\r\ntags: [哈希表, \"面试复习\"]\r\n---\r\n# 思路\r\n正文";
        assert_eq!(
            note_metadata(text),
            (Some(1), vec!["哈希表".into(), "面试复习".into()])
        );
        assert_eq!(markdown_body(text), "# 思路\r\n正文");
        let problems = builtin().unwrap();
        assert_eq!(associate_note("notes.md", text, &problems), Some(1));
        assert_eq!(
            associate_note(
                "notes.md",
                "https://leetcode.cn/problems/two-sum/",
                &problems
            ),
            Some(1)
        );
        assert_eq!(
            associate_note(
                "notes.md",
                "https://leetcode.cn/problems/two-sum-ii/",
                &problems
            ),
            None
        );
        assert_eq!(associate_note("notes.md", "普通笔记", &problems), None);
    }
    #[test]
    fn malformed_cases_are_rejected_before_execution() {
        let problem = builtin().unwrap().remove(0);
        assert!(
            validate_cases(
                &[Case {
                    input: vec![],
                    expected: Value::Null
                }],
                &problem.parameters
            )
            .is_err()
        );
        assert!(
            validate_cases(
                &[Case {
                    input: vec![serde_json::json!("wrong"), serde_json::json!(9)],
                    expected: Value::Null
                }],
                &problem.parameters
            )
            .is_err()
        );
    }
}
