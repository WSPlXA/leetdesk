//! Original practice routes. Membership is curated by solution, not inferred from broad tags.
use crate::{i18n::UiLanguage, online::Site};
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    sync::LazyLock,
};

#[derive(Deserialize)]
pub struct Text {
    pub zh: String,
    pub en: String,
}
impl Text {
    pub fn get(&self, language: UiLanguage) -> &str {
        match language {
            UiLanguage::Chinese => &self.zh,
            UiLanguage::English => &self.en,
        }
    }
}
#[derive(Deserialize)]
pub struct Pattern {
    pub id: String,
    pub title: Text,
    pub summary: Text,
    pub questions: Vec<String>,
}
#[derive(Deserialize)]
pub struct Chapter {
    pub id: String,
    pub title: Text,
    pub overview: Text,
    pub approach: Text,
    pub pitfalls: Text,
    pub complexity: Text,
    pub patterns: Vec<Pattern>,
}
pub static CHAPTERS: LazyLock<Vec<Chapter>> = LazyLock::new(|| {
    let chapters: Vec<Chapter> = serde_json::from_str(include_str!("../assets/learning-path.json"))
        .expect("valid practice routes");
    validate(&chapters).expect("valid curated practice membership");
    chapters
});

#[derive(Deserialize)]
pub struct ProblemLesson {
    #[serde(flatten)]
    pub primary: Walkthrough,
    #[serde(default)]
    pub alternatives: HashMap<String, Walkthrough>,
}

#[derive(Deserialize)]
pub struct Walkthrough {
    pub solution: Text,
    pub cost: Text,
    pub pitfall: Text,
}

pub static LESSONS: LazyLock<HashMap<String, ProblemLesson>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../assets/problem-lessons.json"))
        .expect("valid original problem walkthroughs")
});

#[derive(Deserialize)]
pub struct LessonDetails {
    pub mapping: Text,
    pub why: Text,
    pub trace: Text,
    pub go: String,
    pub nodes: String,
    #[cfg(test)]
    pub checks: Vec<String>,
}

pub static DETAILS: LazyLock<HashMap<String, LessonDetails>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../assets/lesson-details.json"))
        .expect("valid detailed Go walkthroughs")
});

fn detail(slug: &str, pattern: Option<&Pattern>) -> Option<&'static LessonDetails> {
    pattern
        .and_then(|p| DETAILS.get(&format!("{slug}@{}", p.id)))
        .or_else(|| DETAILS.get(slug))
}

fn append_details(body: &mut String, details: &LessonDetails, steps: &str, language: UiLanguage) {
    let chinese = language == UiLanguage::Chinese;
    let pick = |zh: &str, en: &str| if chinese { zh } else { en }.to_owned();
    body.push_str(&format!(
        "### {}\n\n{}\n\n### {}\n\n{}\n\n### {}\n\n{}\n\n### {}\n\n{}\n\n",
        pick("从大类方法到本题", "Applying the general method"),
        details.mapping.get(language),
        pick("解题步骤与依据", "Steps and reasoning"),
        steps,
        pick("为什么这样做是对的", "Why this is correct"),
        details.why.get(language),
        pick("Go 参考解法", "Go reference solution"),
        pick(
            "以下代码使用 Go 1.21+，包含所需的标准库导入。",
            "The code uses Go 1.21+ and includes required standard-library imports."
        )
    ));
    let definition = match details.nodes.as_str() {
        "list" => Some("type ListNode struct { Val int; Next *ListNode }"),
        "tree" => Some("type TreeNode struct { Val int; Left, Right *TreeNode }"),
        "random" => Some("type Node struct { Val int; Next, Random *Node }"),
        "graph" => Some("type Node struct { Val int; Neighbors []*Node }"),
        _ => None,
    };
    if let Some(definition) = definition {
        body.push_str(&format!(
            "{}\n\n```go\n{definition}\n```\n\n",
            pick(
                "节点类型由题目平台提供，结构如下，提交时不要重复声明：",
                "The platform supplies this node type; do not redeclare it when submitting:"
            )
        ));
    }
    body.push_str(&format!(
        "```go\n{}\n```\n\n### {}\n\n{}\n\n",
        details.go,
        pick("示例逐步推演", "Step-by-step example"),
        details.trace.get(language)
    ));
}

/// Compose category context and the current problem in one document. Imported material
/// remains a separate source on disk and retains the caller's image/link origin.
pub fn lesson(
    filter: &str,
    problem: &crate::catalog::Problem,
    language: UiLanguage,
    site: Site,
    problems: &[crate::catalog::Problem],
    supplement: Option<(&str, &str)>,
) -> String {
    let chinese = language == UiLanguage::Chinese;
    let pick = |zh: &str, en: &str| if chinese { zh } else { en }.to_owned();
    let member = memberships(&problem.slug);
    let inferred = member.first().map(|id| format!("@pattern:{id}"));
    let context = if find(filter).is_some() {
        filter
    } else {
        inferred.as_deref().unwrap_or(filter)
    };
    let mut body = guide(context, language, site, problems);
    let mut current = format!(
        "## {} · {}\n\n",
        pick("当前题目", "Current problem"),
        escape_label(&problem.title)
    );
    if find(context).is_some() && !matches(context, &member) {
        current.push_str(&pick(
            "> 当前题目不属于上面的分组。选择左侧本组题目后，这里的解析会同步切换。\n\n",
            "> The current problem is outside the group above. Choose a problem from this group to update the walkthrough here.\n\n",
        ));
    }
    for chapter in CHAPTERS.iter() {
        for pattern in chapter.patterns.iter().filter(|p| member.contains(&p.id)) {
            current.push_str(&format!(
                "- {} → {}：{}\n",
                chapter.title.get(language),
                pattern.title.get(language),
                pattern.summary.get(language)
            ));
        }
    }
    current.push('\n');
    if let Some(lesson) = LESSONS.get(&problem.slug) {
        let selected_pattern = find(context).and_then(|(chapter, pattern)| {
            pattern.or_else(|| chapter.patterns.iter().find(|p| member.contains(&p.id)))
        });
        let lesson = selected_pattern
            .and_then(|pattern| lesson.alternatives.get(&pattern.id))
            .unwrap_or(&lesson.primary);
        if let Some(details) = detail(&problem.slug, selected_pattern) {
            append_details(
                &mut current,
                details,
                lesson.solution.get(language),
                language,
            );
        } else {
            current.push_str(&format!(
                "### {}\n\n{}\n\n",
                pick("解题步骤与依据", "Steps and reasoning"),
                lesson.solution.get(language)
            ));
        }
        current.push_str(&format!(
            "**{}** {}\n\n**{}** {}\n\n",
            pick("时间 / 额外空间：", "Time / auxiliary space:"),
            lesson.cost.get(language),
            pick("本题易错点：", "Problem-specific pitfall:"),
            lesson.pitfall.get(language)
        ));
    } else {
        current.push_str(&pick(
            "这道题尚未收录原创逐题解析。下方会展示已有的内置或导入讲解；也可以导入自己的 Markdown 资料。\n\n",
            "An original walkthrough has not been written for this problem yet. Existing built-in or imported material is shown below; you can also import Markdown material.\n\n",
        ));
    }
    if let Some((name, markdown)) = supplement.filter(|(_, markdown)| !markdown.trim().is_empty()) {
        current.push_str(&format!(
            "### {} · {}\n\n{}\n\n",
            pick("补充讲解", "Supplementary explanation"),
            escape_label(name),
            markdown
        ));
    }
    let marker = format!(
        "## {}",
        pick("做法拆分与练习顺序", "Techniques and practice order")
    );
    if let Some(position) = body.find(&marker) {
        body.insert_str(position, &current);
    } else {
        body.push_str("\n\n");
        body.push_str(&current);
    }
    body
}

fn escape_label(label: &str) -> String {
    label
        .replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace(['\r', '\n'], " ")
}

/// The reader starts with the selected problem, with the general method in the
/// same document below it. Choosing a chapter explicitly opens its overview.
pub fn reading_lesson(
    overview: bool,
    filter: &str,
    problem: &crate::catalog::Problem,
    language: UiLanguage,
    site: Site,
    problems: &[crate::catalog::Problem],
    supplement: Option<(&str, &str)>,
) -> String {
    let pick = |zh: &str, en: &str| {
        if language == UiLanguage::Chinese {
            zh
        } else {
            en
        }
        .to_owned()
    };
    let member = memberships(&problem.slug);
    let context = if matches(filter, &member) || overview {
        filter.to_owned()
    } else {
        member
            .first()
            .map(|p| format!("@pattern:{p}"))
            .unwrap_or_else(|| filter.to_owned())
    };
    if overview {
        let guide = guide(&context, language, site, problems);
        return guide
            .split_once("\n\n")
            .map_or(guide.clone(), |(_, rest)| rest.to_owned());
    }
    let pattern = find(&context).and_then(|(chapter, pattern)| {
        pattern.or_else(|| chapter.patterns.iter().find(|p| member.contains(&p.id)))
    });
    let mut body = String::new();
    if let Some(lesson) = LESSONS.get(&problem.slug) {
        let lesson = pattern
            .and_then(|p| lesson.alternatives.get(&p.id))
            .unwrap_or(&lesson.primary);
        if let Some(details) = detail(&problem.slug, pattern) {
            let (mapping, steps, why) = if problem.slug == "rotting-oranges" {
                (
                    pick(
                        "把每个橘子所在的格子看成状态，上下左右的感染是一次转移。所有已经腐烂的橘子同时作为起点，初始距离为 0。",
                        "Treat each occupied cell as a state and infection in the four directions as one transition. Every initially rotten orange is a source at distance zero.",
                    ),
                    pick(
                        "用队列 `q` 进行多源 BFS，在入队时就把格子改为腐烂，并减少剩余新鲜橘子的数量 `fresh`，避免同一格重复入队。",
                        "Run multi-source BFS with queue `q`. Mark a fresh cell rotten when enqueuing it and decrement `fresh`, so two neighbors cannot enqueue the same cell.",
                    ),
                    pick(
                        "BFS 按层遍历，队列中的节点按距离从近到远出队。每次转移的代价都是 1 分钟，因此第一次到达某个格子的时间，一定是从最近的腐烂橘子出发所需的最少时间。\n\n每轮先固定 `size := len(q)`，只处理本轮已有的格子；新入队的格子留到下一分钟。用 `minutes` 记录完成的层数。队列耗尽后若 `fresh > 0`，说明有橘子被空格隔开，返回 -1。",
                        "BFS removes states in increasing distance order. Each transition takes one minute, so the first arrival at a cell is its minimum distance from any initially rotten orange.\n\nFix `size := len(q)` at the start of each round: newly enqueued cells belong to the next minute. `minutes` counts completed layers. If the queue empties while `fresh > 0`, empty cells have isolated some oranges; return -1.",
                    ),
                )
            } else {
                (
                    details.mapping.get(language).to_owned(),
                    lesson.solution.get(language).to_owned(),
                    details.why.get(language).to_owned(),
                )
            };
            body.push_str(&format!(
                "## {}\n\n{}\n\n{}\n\n## {}\n\n{}\n\n## {}\n\n",
                pick("从大类方法到本题", "Applying the general method"),
                mapping,
                steps,
                pick("为什么这样做是对的", "Why this is correct"),
                why,
                pick("用一个例子走一遍", "Step-by-step example")
            ));
            if problem.slug == "rotting-oranges" {
                body.push_str(&format!("{} `grid = [[2,1,1],[1,1,0],[0,1,1]]`\n\n| {} | {} | {} |\n| --- | --- | --- |\n| 0 | (0, 0) | 6 |\n| 1 | (0, 1), (1, 0) | 4 |\n| 2 | (0, 2), (1, 1) | 2 |\n| 3 | (2, 1) | 1 |\n| 4 | (2, 2) | 0 |\n\n**{}**\n\n",
                    pick("输入：","Input:"),pick("分钟","Minute"),pick("本轮新感染","Newly infected"),pick("剩余新鲜","Fresh left"),pick("答案：4 分钟","Answer: 4 minutes")));
            } else {
                body.push_str(details.trace.get(language));
                body.push_str("\n\n");
            }
            body.push_str(&format!(
                "## {}\n\n",
                pick("Go 参考代码", "Go reference code")
            ));
            let definition = match details.nodes.as_str() {
                "list" => Some("type ListNode struct { Val int; Next *ListNode }"),
                "tree" => Some("type TreeNode struct { Val int; Left, Right *TreeNode }"),
                "random" => Some("type Node struct { Val int; Next, Random *Node }"),
                "graph" => Some("type Node struct { Val int; Neighbors []*Node }"),
                _ => None,
            };
            if let Some(definition) = definition {
                body.push_str(&format!(
                    "{}\n\n```go\n{definition}\n```\n\n",
                    pick(
                        "节点类型由平台提供，提交时不要重复声明。",
                        "The platform provides the node type; omit its declaration when submitting."
                    )
                ));
            }
            body.push_str(&format!("```go\n{}\n```\n\n", details.go));
        } else {
            body.push_str(lesson.solution.get(language));
        }
        body.push_str(&format!(
            "## {}\n\n**{}** {}\n\n**{}** {}\n\n",
            pick("复杂度与易错点", "Complexity and pitfalls"),
            pick("时间 / 额外空间：", "Time / auxiliary space:"),
            lesson.cost.get(language),
            pick("易错点：", "Pitfall:"),
            lesson.pitfall.get(language)
        ));
    } else {
        body.push_str(&pick("这道题尚未收录原创逐题解析。可阅读下方已有资料，或导入自己的 Markdown 讲解。\n\n","An original walkthrough is not available yet. Read existing material below, or import your Markdown explanation.\n\n"));
    }
    if let Some((name, markdown)) = supplement.filter(|(_, body)| !body.trim().is_empty()) {
        body.push_str(&format!(
            "## {} · {}\n\n{}\n\n",
            pick("补充讲解", "Supplementary explanation"),
            escape_label(name),
            markdown
        ));
    }
    body.push_str(&format!(
        "---\n\n## {}\n\n",
        pick("大类方法与同类练习", "General method and related practice")
    ));
    // Demote the guide's title so it fits inside the problem's article.
    body.push_str(&guide(&context, language, site, problems).replacen("# ", "### ", 1));
    body
}

pub fn validate(chapters: &[Chapter]) -> Result<(), String> {
    let mut ids = HashSet::new();
    for chapter in chapters {
        if !ids.insert(chapter.id.as_str()) || chapter.patterns.is_empty() {
            return Err("duplicate or empty chapter".into());
        }
        for text in [
            &chapter.title,
            &chapter.overview,
            &chapter.approach,
            &chapter.pitfalls,
            &chapter.complexity,
        ] {
            if text.zh.trim().is_empty() || text.en.trim().is_empty() {
                return Err("missing chapter explanation".into());
            }
        }
        for pattern in &chapter.patterns {
            if !ids.insert(pattern.id.as_str()) || pattern.questions.is_empty() {
                return Err("duplicate or empty pattern".into());
            }
            if pattern.title.zh.is_empty()
                || pattern.title.en.is_empty()
                || pattern.summary.zh.is_empty()
                || pattern.summary.en.is_empty()
            {
                return Err("missing pattern explanation".into());
            }
            let mut questions = HashSet::new();
            for slug in &pattern.questions {
                if !crate::online::valid_slug(slug) || !questions.insert(slug) {
                    return Err(format!("invalid or duplicate slug: {slug}"));
                }
            }
        }
    }
    Ok(())
}

pub fn find(filter: &str) -> Option<(&'static Chapter, Option<&'static Pattern>)> {
    if let Some(id) = filter.strip_prefix("@chapter:") {
        return CHAPTERS
            .iter()
            .find(|chapter| chapter.id == id)
            .map(|chapter| (chapter, None));
    }
    let id = filter.strip_prefix("@pattern:")?;
    CHAPTERS.iter().find_map(|chapter| {
        chapter
            .patterns
            .iter()
            .find(|pattern| pattern.id == id)
            .map(|pattern| (chapter, Some(pattern)))
    })
}

pub fn memberships(slug: &str) -> Vec<String> {
    CHAPTERS
        .iter()
        .flat_map(|chapter| {
            chapter
                .patterns
                .iter()
                .filter(move |pattern| pattern.questions.iter().any(|question| question == slug))
                .map(|pattern| pattern.id.clone())
        })
        .collect()
}

pub fn search_terms(slug: &str) -> String {
    CHAPTERS
        .iter()
        .flat_map(|chapter| {
            chapter
                .patterns
                .iter()
                .filter(move |pattern| pattern.questions.iter().any(|question| question == slug))
                .map(move |pattern| {
                    format!(
                        "{} {} {} {}",
                        chapter.title.zh, chapter.title.en, pattern.title.zh, pattern.title.en
                    )
                })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn matches(filter: &str, patterns: &[String]) -> bool {
    find(filter).is_some_and(|(chapter, pattern)| {
        if let Some(pattern) = pattern {
            patterns.contains(&pattern.id)
        } else {
            chapter
                .patterns
                .iter()
                .any(|pattern| patterns.contains(&pattern.id))
        }
    })
}

pub fn title(filter: &str, language: UiLanguage) -> Option<&'static str> {
    find(filter).map(|(chapter, pattern)| {
        pattern
            .map_or(&chapter.title, |pattern| &pattern.title)
            .get(language)
    })
}

pub fn guide(
    filter: &str,
    language: UiLanguage,
    site: Site,
    problems: &[crate::catalog::Problem],
) -> String {
    let Some((chapter, pattern)) = find(filter) else {
        return match language {
            UiLanguage::Chinese => "# 按解法学习\n\n从左侧选择一个大类，先阅读适用条件与易错点，再选择具体做法练习。题目按解法清单编排，可归入多个分组；题数只统计当前题库已载入的题目。\n\n编排为应用内原创整理，供复习与迁移使用，并非对题目唯一解法的判定。".into(),
            UiLanguage::English => "# Practice by approach\n\nChoose a chapter on the left to read its decision rules and pitfalls, then practice a specific technique. Curated membership permits multiple approaches per problem. Counts reflect problems loaded in the current catalog.\n\nThese original practice routes are a learning aid, not a claim that each problem has only one solution.".into(),
        };
    };
    let pick = |zh: &str, en: &str| {
        if language == UiLanguage::Chinese {
            zh.to_owned()
        } else {
            en.to_owned()
        }
    };
    let mut body = if let Some(pattern) = pattern {
        format!(
            "# {} · {}\n\n{}\n\n{}\n\n",
            chapter.title.get(language),
            pattern.title.get(language),
            chapter.overview.get(language),
            pattern.summary.get(language)
        )
    } else {
        format!(
            "# {}\n\n{}\n\n",
            chapter.title.get(language),
            chapter.overview.get(language)
        )
    };
    let planned: HashSet<_> = chapter
        .patterns
        .iter()
        .filter(|candidate| pattern.is_none_or(|selected| selected.id == candidate.id))
        .flat_map(|pattern| pattern.questions.iter().map(String::as_str))
        .collect();
    let loaded = problems
        .iter()
        .filter(|problem| planned.contains(problem.slug.as_str()))
        .count();
    body.push_str(&format!("> {}\n\n", if language == UiLanguage::Chinese {format!("本组已载入 {loaded} / {} 道编排题目。左侧题目列表仍受题单、难度、状态和搜索条件筛选。", planned.len())} else {format!("Loaded {loaded} / {} curated problems. The problem list still respects plan, difficulty, status, and search filters.", planned.len())}));
    for (heading, text) in [
        (
            pick("如何判断与推进", "Decision process"),
            &chapter.approach,
        ),
        (
            pick("复杂度与前提", "Cost and assumptions"),
            &chapter.complexity,
        ),
        (pick("易错点", "Pitfalls"), &chapter.pitfalls),
    ] {
        body.push_str(&format!("## {heading}\n\n{}\n\n", text.get(language)));
    }
    let patterns: Vec<_> = pattern.map_or_else(
        || chapter.patterns.iter().collect(),
        |pattern| vec![pattern],
    );
    body.push_str(&format!(
        "## {}\n\n",
        pick("做法拆分与练习顺序", "Techniques and practice order")
    ));
    for pattern in patterns {
        body.push_str(&format!(
            "### {}\n\n{}\n\n",
            pattern.title.get(language),
            pattern.summary.get(language)
        ));
        for (index, slug) in pattern.questions.iter().enumerate() {
            let label = problems
                .iter()
                .find(|problem| &problem.slug == slug)
                .map_or_else(
                    || slug.replace('-', " "),
                    |problem| {
                        let id = problem.online.as_ref().map_or_else(
                            || problem.id.to_string(),
                            |remote| remote.frontend_id.clone(),
                        );
                        format!("{id}. {}", problem.title)
                    },
                )
                .replace('\\', "\\\\")
                .replace('[', "\\[")
                .replace(']', "\\]");
            body.push_str(&format!(
                "{}. [{label}]({}/problems/{slug}/)\n",
                index + 1,
                site.origin()
            ));
        }
        body.push('\n');
    }
    if chapter.id == "graph" {
        body.push_str(&format!("## {}\n\n[{}](https://labuladong.online/zh/algo/data-structure-basic/graph-traverse-basic/)\n\n", pick("参考阅读", "Further reading"), pick("labuladong：图结构的 DFS/BFS 遍历", "labuladong: graph DFS/BFS traversal")));
    }
    body.push_str(&pick("---\n说明与解法分组为应用内原创整理。练习链接指向官方站点；尚未载入的题目需先同步题库。", "---\nOriginal explanations and curated practice groups. Links open the official site; sync the catalog to practice problems not loaded locally."));
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reader_keeps_problem_first_and_preserves_variants_and_imports() {
        let problems = crate::catalog::builtin().unwrap();
        let mut problem = problems[0].clone();
        problem.slug = "rotting-oranges".into();
        let body = reading_lesson(
            false,
            "@pattern:bfs-distance",
            &problem,
            UiLanguage::Chinese,
            Site::China,
            &problems,
            Some(("自有笔记", "![图](./figure.png)")),
        );
        assert!(body.starts_with("## 从大类方法到本题"));
        assert!(body.find("## Go 参考代码").unwrap() < body.find("## 大类方法与同类练习").unwrap());
        assert!(body.contains("| 4 | (2, 2) | 0 |"));
        assert!(body.contains("size := len(q)"));
        assert!(body.contains("![图](./figure.png)"));
        problem.slug = "trapping-rain-water".into();
        let stack = reading_lesson(
            false,
            "@pattern:monotone-stack",
            &problem,
            UiLanguage::English,
            Site::Global,
            &problems,
            None,
        );
        assert!(stack.contains("height[bottom]"));
        assert!(!stack.contains("lm - height[l]"));
        let guide = reading_lesson(
            true,
            "@chapter:graph",
            &problem,
            UiLanguage::English,
            Site::Global,
            &problems,
            None,
        );
        assert!(guide.contains("Cost and assumptions"));
        assert!(!guide.contains("func trap"));
    }
    #[test]
    fn every_curated_problem_has_an_original_bilingual_walkthrough() {
        let expected: HashSet<_> = CHAPTERS
            .iter()
            .flat_map(|c| &c.patterns)
            .flat_map(|p| &p.questions)
            .collect();
        assert_eq!(expected.len(), 166);
        assert_eq!(LESSONS.len(), expected.len());
        assert_eq!(DETAILS.len(), 170);
        for slug in expected {
            let lesson = &LESSONS[slug];
            let keys = std::iter::once(slug.clone())
                .chain(lesson.alternatives.keys().map(|p| format!("{slug}@{p}")));
            for key in keys {
                let details = &DETAILS[&key];
                for text in [&details.mapping, &details.why, &details.trace] {
                    assert!(!text.zh.trim().is_empty(), "{key}");
                    assert!(!text.en.trim().is_empty(), "{key}");
                }
                assert!(details.go.contains("func "), "{key}");
                assert!(!details.go.contains("TODO"), "{key}");
                assert!(!details.checks.is_empty(), "{key}");
                assert!(
                    details.trace.zh.contains("1.") && details.trace.zh.contains("3."),
                    "{key}"
                );
            }
            for walkthrough in std::iter::once(&lesson.primary).chain(lesson.alternatives.values())
            {
                for text in [
                    &walkthrough.solution,
                    &walkthrough.cost,
                    &walkthrough.pitfall,
                ] {
                    assert!(!text.zh.trim().is_empty(), "{slug}");
                    assert!(!text.en.trim().is_empty(), "{slug}");
                }
            }
            for pattern in lesson.alternatives.keys() {
                assert!(memberships(slug).contains(pattern), "{slug}: {pattern}");
            }
        }
    }

    #[test]
    fn combined_lessons_follow_the_problem_and_selected_technique_without_guessing() {
        let problems = crate::catalog::builtin().unwrap();
        let mut problem = problems[0].clone();
        problem.slug = "trapping-rain-water".into();
        let cn = lesson(
            "@pattern:monotone-stack",
            &problem,
            UiLanguage::Chinese,
            Site::China,
            &problems,
            Some(("参考.md", "![图](./figure.png)\n\n自有讲解")),
        );
        assert!(cn.contains("栈、单调结构与堆"));
        assert!(cn.contains("弹出洼底"));
        assert!(!cn.contains("较小边界一侧的蓄水量"));
        assert!(cn.contains("![图](./figure.png)"));
        assert!(cn.contains("从大类方法到本题"));
        assert!(cn.contains("为什么这样做是对的"));
        assert!(cn.contains("```go\nfunc trap"));
        assert!(cn.contains("height[bottom]"));
        assert!(cn.contains("water2"));
        assert!(!cn.contains("lm-height[l]"));
        assert!(cn.find("当前题目").unwrap() < cn.find("做法拆分与练习顺序").unwrap());
        let en = lesson(
            "全部题目",
            &problem,
            UiLanguage::English,
            Site::Global,
            &problems,
            None,
        );
        assert!(en.contains("Pointers and windows"));
        assert!(en.contains("lower maximum"));
        assert!(en.contains("Time / auxiliary space"));
        assert!(en.contains("Applying the general method"));
        assert!(en.contains("Why this is correct"));
        assert!(en.contains("Step-by-step example"));
        assert!(en.contains("lm - height[l]"));
        assert!(!en.contains("较小边界"));
        let wrong_group = lesson(
            "@chapter:graph",
            &problem,
            UiLanguage::English,
            Site::Global,
            &problems,
            None,
        );
        assert!(wrong_group.contains("outside the group above"));
        problem.slug = "unclassified-custom-problem".into();
        let unknown = lesson(
            "全部题目",
            &problem,
            UiLanguage::Chinese,
            Site::China,
            &problems,
            Some(("自写", "已有分析")),
        );
        assert!(unknown.contains("尚未收录原创逐题解析"));
        assert!(unknown.contains("已有分析"));
        assert!(!unknown.contains("弹出洼底"));
    }

    #[test]
    fn curated_routes_cover_hot100_and_permit_multiple_approaches() {
        validate(&CHAPTERS).unwrap();
        assert_eq!(CHAPTERS.len(), 12);
        for slug in crate::online::builtin_hot100()
            .groups
            .iter()
            .flat_map(|group| &group.questions)
        {
            assert!(
                !memberships(slug).is_empty(),
                "unclassified Hot 100 problem: {slug}"
            );
        }
        let water = memberships("trapping-rain-water");
        assert!(water.contains(&"opposing".into()));
        assert!(water.contains(&"monotone-stack".into()));
        assert!(memberships("unknown-imported-problem").is_empty());
        assert!(!matches(
            "@pattern:graph-paths",
            &memberships("course-schedule")
        ));
    }

    #[test]
    fn guides_are_bilingual_offline_and_link_to_the_selected_official_site() {
        let problems = crate::catalog::builtin().unwrap();
        for chapter in CHAPTERS.iter() {
            for language in [UiLanguage::Chinese, UiLanguage::English] {
                let body = guide(
                    &format!("@chapter:{}", chapter.id),
                    language,
                    Site::Global,
                    &problems,
                );
                assert!(body.contains(chapter.title.get(language)));
                assert!(body.contains(chapter.overview.get(language)));
                assert!(body.contains("https://leetcode.com/problems/"));
                assert!(!body.contains("https://leetcode.cn/problems/"));
            }
        }
        let body = guide(
            "@pattern:hash-complement",
            UiLanguage::Chinese,
            Site::China,
            &problems,
        );
        assert!(body.contains("1. 两数之和"));
        assert!(body.contains("https://leetcode.cn/problems/two-sum/"));
        let graph = guide(
            "@chapter:graph",
            UiLanguage::Chinese,
            Site::China,
            &problems,
        );
        assert!(graph.contains("graph-traverse-basic"));
        assert!(graph.contains("路径枚举"));
    }
}
