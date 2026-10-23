//! Application strings only. Problem statements, code, notes and user tags are
//! kept in their original language; the selected LeetCode site chooses the UI.
use std::{borrow::Cow, collections::HashMap, sync::LazyLock};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UiLanguage {
    #[default]
    Chinese,
    English,
}

impl UiLanguage {
    pub fn for_site(site: crate::online::Site) -> Self {
        match site {
            crate::online::Site::China => Self::Chinese,
            crate::online::Site::Global => Self::English,
        }
    }

    pub fn text<'a>(self, source: &'a str) -> Cow<'a, str> {
        if self == Self::Chinese {
            return Cow::Borrowed(source);
        }
        if let Some(value) = STRINGS.get(source) {
            return Cow::Borrowed(value);
        }
        if !source
            .chars()
            .any(|ch| ('\u{4e00}'..='\u{9fff}').contains(&ch))
        {
            return Cow::Borrowed(source);
        }
        for rule in RULES.iter() {
            if let Some(captures) = rule.pattern.captures(source) {
                let mut result = String::new();
                let mut anonymous = 0;
                let mut at = 0;
                for field in FIELDS.find_iter(&rule.english) {
                    result.push_str(&rule.english[at..field.start()]);
                    let key = &field.as_str()[1..field.as_str().len() - 1];
                    let key = key.split(':').next().unwrap_or(key);
                    let index = if key.is_empty() {
                        let index = rule
                            .fields
                            .iter()
                            .enumerate()
                            .filter(|(_, name)| name.is_empty())
                            .nth(anonymous)
                            .map(|(index, _)| index);
                        anonymous += 1;
                        index
                    } else {
                        rule.fields.iter().position(|name| name == key)
                    };
                    if let Some(index) = index {
                        let value = captures.get(index + 1).unwrap().as_str();
                        result.push_str(&self.text(value));
                    }
                    at = field.end();
                }
                result.push_str(&rule.english[at..]);
                return Cow::Owned(result);
            }
        }
        Cow::Borrowed(source)
    }
}

static STRINGS: LazyLock<HashMap<String, String>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../assets/i18n-en.json"))
        .expect("valid English translations")
});
static FIELDS: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"\{[^{}]*\}").unwrap());
struct Rule {
    pattern: regex::Regex,
    fields: Vec<String>,
    english: String,
}
static RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    let mut entries: Vec<_> = STRINGS
        .iter()
        .filter(|(key, _)| key.contains('{'))
        .collect();
    entries.sort_by(|(a, _), (b, _)| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    entries
        .into_iter()
        .map(|(chinese, english)| {
            let mut pattern = String::from("(?s)^");
            let mut fields = vec![];
            let mut at = 0;
            for field in FIELDS.find_iter(chinese) {
                pattern.push_str(&regex::escape(&chinese[at..field.start()]));
                pattern.push_str("(.*?)");
                let name = &field.as_str()[1..field.as_str().len() - 1];
                fields.push(name.split(':').next().unwrap_or(name).to_string());
                at = field.end();
            }
            pattern.push_str(&regex::escape(&chinese[at..]));
            pattern.push('$');
            Rule {
                pattern: regex::Regex::new(&pattern).unwrap(),
                fields,
                english: english.clone(),
            }
        })
        .collect()
});

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn global_ui_translates_labels_and_messages_without_changing_user_content() {
        let en = UiLanguage::for_site(crate::online::Site::Global);
        assert_eq!(en.text("测试用例"), "Test cases");
        assert_eq!(
            en.text("题目 42 的在线运行已完成"),
            "Online run completed for problem 42"
        );
        assert_eq!(en.text("已切换为米色主题"), "Switched to Beige theme");
        assert_eq!(
            en.text("导入失败：无法读取 F:/我的笔记.md"),
            "Import failed: 无法读取 F:/我的笔记.md"
        );
        assert_eq!(en.text("我的私人笔记内容"), "我的私人笔记内容");
        assert_eq!(UiLanguage::Chinese.text("测试用例"), "测试用例");
    }
}
