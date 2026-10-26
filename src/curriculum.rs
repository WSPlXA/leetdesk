//! Original, offline lessons. Courses exist independently of problem catalogues.
use serde::Deserialize;
use std::sync::LazyLock;

#[derive(Deserialize)]
pub struct Course {
    pub id: String,
    pub unit: String,
    pub title: String,
    pub prerequisites: Vec<String>,
    pub goal: String,
    pub concept: String,
    pub trace: String,
    pub code: String,
    pub output: String,
    pub boundary: String,
    pub exercise: String,
    pub answer: String,
    #[serde(default)]
    pub practice: Vec<String>,
}
pub static COURSES: LazyLock<Vec<Course>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../assets/curriculum.json")).expect("valid original courses")
});

pub fn selected(route: &str) -> Option<&'static Course> {
    let id = route.strip_prefix("@course:")?.split(':').next()?;
    COURSES.iter().find(|course| course.id == id)
}

pub fn route_for_link(value: &str) -> Option<String> {
    let url = url::Url::parse(value).ok()?;
    if url.scheme() != "leetdesk" {
        return None;
    }
    let id = url.path().trim_start_matches('/');
    match url.host_str()? {
        "course" if COURSES.iter().any(|c| c.id == id) => Some(format!("@course:{id}")),
        "course-answer" if COURSES.iter().any(|c| c.id == id) => {
            Some(format!("@course:{id}:answers"))
        }
        _ => None,
    }
}

pub fn document(route: &str) -> Option<String> {
    let course = selected(route)?;
    let index = COURSES.iter().position(|c| c.id == course.id)?;
    let mut body = format!(
        "# {}\n\n{} · 第 {} / {} 课 · 原创离线课程\n\n## 学习目标\n\n{}\n\n",
        course.title,
        course.unit,
        index + 1,
        COURSES.len(),
        course.goal
    );
    body.push_str("## 先修知识\n\n");
    if course.prerequisites.is_empty() {
        body.push_str("本课从零开始，不需要先刷题。\n\n");
    }
    for id in &course.prerequisites {
        if let Some(prerequisite) = COURSES.iter().find(|c| c.id == *id) {
            body.push_str(&format!(
                "- [{}](leetdesk://course/{id})\n",
                prerequisite.title
            ));
        }
    }
    body.push_str(&format!("\n## 核心概念\n\n{}\n\n## 过程推演\n\n{}\n\n## Go 实验\n\n先预测输出，再复制到本机 Go 环境运行。实验是独立程序，不会替换题目草稿。\n\n```go\n{}\n```\n\n预期输出：\n\n```text\n{}\n```\n\n## 复杂度与边界\n\n{}\n\n## 自己试一试\n\n{}\n\n", course.concept, course.trace, course.code, course.output, course.boundary, course.exercise));
    if route.ends_with(":answers") {
        body.push_str(&format!(
            "## 核对思路\n\n{}\n\n[收起核对思路](leetdesk://course/{})\n\n",
            course.answer, course.id
        ));
    } else {
        body.push_str(&format!(
            "[完成练习后查看核对思路](leetdesk://course-answer/{})\n\n",
            course.id
        ));
    }
    body.push_str("## 继续学习\n\n");
    if index > 0 {
        let c = &COURSES[index - 1];
        body.push_str(&format!(
            "- 上一课：[{}](leetdesk://course/{})\n",
            c.title, c.id
        ));
    }
    if let Some(c) = COURSES.get(index + 1) {
        body.push_str(&format!(
            "- 下一课：[{}](leetdesk://course/{})\n",
            c.title, c.id
        ));
    }
    if !course.practice.is_empty() {
        body.push_str("\n学完本课后，可在左侧解法目录练习：\n\n");
        for topic in &course.practice {
            body.push_str(&format!("- {topic}\n"));
        }
    }
    Some(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn courses_have_earlier_prerequisites_and_substantive_sections() {
        let mut seen = std::collections::HashSet::new();
        for course in COURSES.iter() {
            assert!(seen.insert(&course.id), "duplicate {}", course.id);
            for prerequisite in &course.prerequisites {
                assert!(
                    seen.contains(prerequisite),
                    "{} needs {} first",
                    course.id,
                    prerequisite
                );
                assert_ne!(prerequisite, &course.id);
            }
            for section in [
                &course.goal,
                &course.concept,
                &course.trace,
                &course.boundary,
                &course.exercise,
                &course.answer,
            ] {
                assert!(!section.trim().is_empty(), "empty section in {}", course.id);
            }
            assert!(course.code.starts_with("package main\n"));
            let doc = document(&format!("@course:{}", course.id)).unwrap();
            assert!(!doc.contains("## 核对思路"));
            assert!(
                document(&format!("@course:{}:answers", course.id))
                    .unwrap()
                    .contains("## 核对思路")
            );
            assert_eq!(
                route_for_link(&format!("leetdesk://course/{}", course.id)),
                Some(format!("@course:{}", course.id))
            );
        }
        assert!(route_for_link("https://example.com").is_none());
        assert!(route_for_link("leetdesk://course/missing").is_none());
    }
}
