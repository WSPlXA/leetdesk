//! Search and progress snapshots shared by sidebar redraws.
use crate::{catalog::Problem, learning, online, store::Progress, topics};
use std::{
    cell::RefCell,
    collections::{BTreeSet, HashMap},
    rc::Rc,
};

struct Row {
    id: u32,
    slug: String,
    search: String,
    local_search: String,
    tags: Vec<String>,
    difficulty: String,
    status: &'static str,
    hot_rank: Option<usize>,
    hot_topic: Option<String>,
    patterns: Vec<String>,
}

type FilterCache = (String, String, String, String, String, Rc<[usize]>);
type RouteCounts = HashMap<String, ((usize, usize), (usize, usize))>;

pub struct CatalogView {
    rows: Vec<Row>,
    progress: HashMap<u32, Progress>,
    categories: Vec<String>,
    filtered: RefCell<Option<FilterCache>>,
    plan: online::StudyPlan,
    topic_categories: Vec<String>,
    custom_categories: Vec<String>,
    hot_progress: (usize, usize),
    route_counts: RouteCounts,
    route_rows: HashMap<String, Vec<usize>>,
}

impl CatalogView {
    pub fn new(problems: &[Problem], progress: HashMap<u32, Progress>) -> Self {
        let mut view = Self {
            rows: vec![],
            progress,
            categories: vec![],
            filtered: RefCell::new(None),
            plan: online::builtin_hot100(),
            topic_categories: vec![],
            custom_categories: vec![],
            hot_progress: (0, 0),
            route_counts: HashMap::new(),
            route_rows: HashMap::new(),
        };
        view.replace(problems);
        view
    }

    pub fn replace(&mut self, problems: &[Problem]) {
        self.rows = problems
            .iter()
            .map(|p| Row {
                id: p.id,
                slug: p.slug.clone(),
                search: format!(
                    "{} {} {} {} {} {} {}",
                    p.online
                        .as_ref()
                        .map(|m| m.frontend_id.clone())
                        .unwrap_or_else(|| p.id.to_string()),
                    p.title,
                    p.slug,
                    p.tags.join(" "),
                    p.tags
                        .iter()
                        .map(|tag| topics::label(tag))
                        .collect::<Vec<_>>()
                        .join(" "),
                    p.tags
                        .iter()
                        .map(|tag| crate::i18n::UiLanguage::English
                            .text(topics::label(tag))
                            .into_owned())
                        .collect::<Vec<_>>()
                        .join(" "),
                    learning::search_terms(&p.slug)
                )
                .to_lowercase(),
                local_search: String::new(),
                tags: p
                    .tags
                    .iter()
                    .map(|tag| topics::label(tag).to_string())
                    .collect(),
                difficulty: topics::difficulty(&p.difficulty).into(),
                status: online::status_label(p),
                hot_rank: None,
                hot_topic: None,
                patterns: learning::memberships(&p.slug),
            })
            .collect();
        self.rebuild_plan();
        self.rebuild_categories();
    }

    pub fn update_progress(&mut self, id: u32, progress: &Progress) {
        if self.progress.get(&id) == Some(progress)
            || (progress == &Progress::default() && !self.progress.contains_key(&id))
        {
            return;
        }
        self.progress.insert(id, progress.clone());
        self.rebuild_categories();
    }

    pub fn replace_progress(&mut self, progress: HashMap<u32, Progress>) {
        self.progress = progress;
        self.rebuild_categories();
    }

    pub fn set_plan(&mut self, plan: online::StudyPlan) {
        self.plan = plan;
        self.rebuild_plan();
        self.rebuild_categories();
    }

    fn rebuild_plan(&mut self) {
        let ranks: HashMap<_, _> = self
            .plan
            .groups
            .iter()
            .flat_map(|g| {
                g.questions
                    .iter()
                    .map(move |slug| (slug, topics::label(&g.name)))
            })
            .enumerate()
            .map(|(rank, (slug, topic))| (slug.as_str(), (rank, topic)))
            .collect();
        for row in &mut self.rows {
            let entry = ranks.get(row.slug.as_str());
            row.hot_rank = entry.map(|(rank, _)| *rank);
            row.hot_topic = entry.map(|(_, topic)| topic.to_string());
        }
        self.hot_progress = (
            self.rows
                .iter()
                .filter(|r| r.hot_rank.is_some() && r.status == "已通过")
                .count(),
            self.rows.iter().filter(|r| r.hot_rank.is_some()).count(),
        );
        self.route_counts.clear();
        self.route_rows.clear();
        for (index, row) in self.rows.iter().enumerate() {
            if row.patterns.is_empty() {
                continue;
            }
            let mut filters: Vec<_> = row
                .patterns
                .iter()
                .map(|id| format!("@pattern:{id}"))
                .collect();
            filters.extend(
                learning::CHAPTERS
                    .iter()
                    .filter(|chapter| {
                        chapter
                            .patterns
                            .iter()
                            .any(|pattern| row.patterns.contains(&pattern.id))
                    })
                    .map(|chapter| format!("@chapter:{}", chapter.id)),
            );
            for filter in filters {
                self.route_rows
                    .entry(filter.clone())
                    .or_default()
                    .push(index);
                let (all, hot) = self.route_counts.entry(filter).or_default();
                all.1 += 1;
                all.0 += usize::from(row.status == "已通过");
                if row.hot_rank.is_some() {
                    hot.1 += 1;
                    hot.0 += usize::from(row.status == "已通过");
                }
            }
        }
    }

    pub fn topics(&self) -> &[String] {
        &self.topic_categories
    }
    pub fn learning_indices(&self, filter: &str) -> &[usize] {
        self.route_rows
            .get(filter)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }
    pub fn custom_tags(&self) -> &[String] {
        &self.custom_categories
    }
    pub fn hot_progress(&self) -> (usize, usize) {
        self.hot_progress
    }

    pub fn learning_progress(&self, filter: &str, book: &str) -> (usize, usize) {
        self.route_counts
            .get(filter)
            .map_or((0, 0), |(all, hot)| match book {
                "全部题目" => *all,
                "热题 100" => *hot,
                _ => (0, 0),
            })
    }

    pub fn progress(&self, id: u32) -> Option<&Progress> {
        self.progress.get(&id)
    }
    pub fn categories(&self) -> &[String] {
        &self.categories
    }

    fn rebuild_categories(&mut self) {
        for row in &mut self.rows {
            row.local_search = self
                .progress
                .get(&row.id)
                .map(|progress| progress.tags.join(" ").to_lowercase())
                .unwrap_or_default();
        }
        let mut official: BTreeSet<_> = self
            .rows
            .iter()
            .flat_map(|r| r.tags.iter().chain(r.hot_topic.iter()))
            .cloned()
            .collect();
        let mut common: Vec<_> = topics::COMMON
            .iter()
            .filter_map(|t| official.take(*t))
            .collect();
        common.extend(official);
        self.topic_categories = common;
        self.custom_categories = self
            .progress
            .values()
            .flat_map(|p| p.tags.iter())
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let tags: BTreeSet<_> = self
            .rows
            .iter()
            .flat_map(|r| r.tags.iter())
            .chain(self.progress.values().flat_map(|p| p.tags.iter()))
            .cloned()
            .collect();
        self.categories = ["全部题目", "已通过", "未解答", "尝试过", "收藏", "待复习"]
            .into_iter()
            .map(str::to_string)
            .chain(tags)
            .collect();
        *self.filtered.get_mut() = None;
    }

    pub fn filter(&self, category: &str, difficulty: &str, query: &str) -> Rc<[usize]> {
        self.filter_in(category, difficulty, query, "全部题目", "全部")
    }

    pub fn filter_in(
        &self,
        category: &str,
        difficulty: &str,
        query: &str,
        book: &str,
        status: &str,
    ) -> Rc<[usize]> {
        if let Some((c, d, q, b, s, result)) = self.filtered.borrow().as_ref()
            && c == category
            && d == difficulty
            && q == query
            && b == book
            && s == status
        {
            return result.clone();
        }
        let lower = query.to_lowercase();
        let mut indices: Vec<usize> = self
            .rows
            .iter()
            .enumerate()
            .filter_map(|(i, row)| {
                let local = self.progress.get(&row.id);
                let custom = category.strip_prefix("@custom:");
                let category = topics::label(category);
                let category_matches = if let Some(tag) = custom {
                    local.is_some_and(|p| p.tags.iter().any(|t| t == tag))
                } else {
                    match category {
                        "全部题目" => true,
                        "@unclassified" => row.patterns.is_empty(),
                        route
                            if route.starts_with("@chapter:") || route.starts_with("@pattern:") =>
                        {
                            learning::matches(route, &row.patterns)
                        }
                        "收藏" => local.is_some_and(|p| p.favorite),
                        "待复习" => local.is_some_and(|p| p.state == "待复习"),
                        "已通过" | "未解答" | "尝试过" => row.status == category,
                        tag => {
                            row.tags.iter().any(|t| t == tag)
                                || row.hot_topic.as_deref() == Some(tag)
                                || local.is_some_and(|p| p.tags.iter().any(|t| t == tag))
                        }
                    }
                };
                let status_matches = match status {
                    "全部" => true,
                    "收藏" => local.is_some_and(|p| p.favorite),
                    "待复习" => local.is_some_and(|p| p.state == "待复习"),
                    state => row.status == state,
                };
                (category_matches
                    && status_matches
                    && (book == "全部题目" || (book == "热题 100" && row.hot_rank.is_some()))
                    && (difficulty == "全部" || row.difficulty == difficulty)
                    && (lower.is_empty()
                        || row.search.contains(&lower)
                        || row.local_search.contains(&lower)))
                .then_some(i)
            })
            .collect();
        if book == "热题 100" {
            indices.sort_by_key(|i| self.rows[*i].hot_rank);
        }
        let indices: Rc<[usize]> = indices.into();
        *self.filtered.borrow_mut() = Some((
            category.into(),
            difficulty.into(),
            query.into(),
            book.into(),
            status.into(),
            indices.clone(),
        ));
        indices
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn solution_groups_use_curated_membership_and_intersect_existing_filters() {
        let problems = online::parse_page(online::Site::China, &serde_json::json!({"data":{"problemsetQuestionList":{"total":5,"questions":[
            {"frontendQuestionId":"200","titleSlug":"number-of-islands","title":"岛屿数量","difficulty":"Medium","status":"ac","topicTags":[{"name":"Graph"}]},
            {"frontendQuestionId":"994","titleSlug":"rotting-oranges","title":"腐烂的橘子","difficulty":"Medium","status":"ac","topicTags":[{"name":"Graph"}]},
            {"frontendQuestionId":"207","titleSlug":"course-schedule","title":"课程表","difficulty":"Medium","topicTags":[{"name":"Graph"}]},
            {"frontendQuestionId":"547","titleSlug":"number-of-provinces","title":"省份数量","difficulty":"Medium","status":"ac","topicTags":[{"name":"Graph"}]},
            {"frontendQuestionId":"9999","titleSlug":"new-unclassified-graph","title":"未整理图题","difficulty":"Easy","topicTags":[{"name":"Graph"}]}
        ]}}})).unwrap().problems;
        let view = CatalogView::new(&problems, HashMap::new());
        assert_eq!(
            view.filter("@chapter:graph", "全部", "").as_ref(),
            &[0, 1, 2, 3]
        );
        assert_eq!(
            view.filter("@pattern:bfs-distance", "全部", "").as_ref(),
            &[1]
        );
        assert_eq!(
            view.filter_in("@chapter:graph", "中等", "", "热题 100", "未解答")
                .as_ref(),
            &[2]
        );
        assert_eq!(view.learning_progress("@chapter:graph", "全部题目"), (3, 4));
        assert_eq!(view.learning_progress("@chapter:graph", "热题 100"), (2, 3));
        assert_eq!(view.filter("@unclassified", "全部", "").as_ref(), &[4]);
        assert_eq!(view.filter("@chapter:graph", "全部", "BFS").as_ref(), &[1]);
        assert!(view.filter("@pattern:missing", "全部", "").is_empty());
    }
    pub(crate) fn online_fixture() -> Vec<Problem> {
        crate::online::parse_page(crate::online::Site::China, &serde_json::json!({"data":{"problemsetQuestionList":{"total":5,"questions":[
            {"frontendQuestionId":"203","titleSlug":"remove-linked-list-elements","title":"移除链表元素","difficulty":"Easy","topicTags":[{"name":"Linked List"}]},
            {"frontendQuestionId":"21","titleSlug":"merge-two-sorted-lists","title":"合并两个有序链表","difficulty":"Easy","topicTags":[{"name":"Linked List"}]},
            {"frontendQuestionId":"206","titleSlug":"reverse-linked-list","title":"反转链表","difficulty":"MEDIUM","status":"ac","topicTags":[{"name":"Linked List"},{"name":"Two Pointers"}]},
            {"frontendQuestionId":"3","titleSlug":"longest-substring-without-repeating-characters","title":"无重复字符的最长子串","difficulty":"Hard","topicTags":[{"name":"String"}]},
            {"frontendQuestionId":"133","titleSlug":"clone-graph","title":"克隆图","difficulty":"Medium","status":"notac","topicTags":[{"name":"Graph"}]}
        ]}}})).unwrap().problems
    }

    #[test]
    fn hot100_topics_status_and_difficulty_intersect_and_keep_server_ac() {
        let problems = online_fixture();
        let mut view = CatalogView::new(&problems, HashMap::new());
        assert_eq!(
            view.filter_in("链表", "简单", "", "热题 100", "未解答")
                .as_ref(),
            &[1]
        );
        assert_eq!(
            view.filter_in("链表", "简单", "", "全部题目", "未解答")
                .as_ref(),
            &[0, 1]
        );
        assert_eq!(
            view.filter_in("双指针", "中等", "", "热题 100", "已通过")
                .as_ref(),
            &[2]
        );
        assert_eq!(view.filter("图", "全部", "").as_ref(), &[4]);
        assert_eq!(view.filter("全部题目", "全部", "字符串").as_ref(), &[3]);
        assert_eq!(view.filter("全部题目", "全部", "String").as_ref(), &[3]);
        assert_eq!(
            view.filter("全部题目", "全部", "Linked List").as_ref(),
            &[0, 1, 2]
        );
        assert!(view.topics().iter().any(|t| t == "链表"));
        assert!(!view.topics().iter().any(|t| t == "Linked List"));
        assert_eq!(view.hot_progress(), (1, 3));
        let progress = Progress {
            favorite: true,
            state: "本地通过".into(),
            tags: vec!["链表".into(), "错题🙂".into()],
        };
        view.update_progress(21, &progress);
        assert_eq!(view.filter("@custom:链表", "全部", "").as_ref(), &[1]);
        assert_eq!(view.filter("全部题目", "全部", "错题🙂").as_ref(), &[1]);
        assert_eq!(
            view.filter_in("链表", "全部", "", "热题 100", "收藏")
                .as_ref(),
            &[1]
        );
        assert_eq!(
            view.filter_in("链表", "全部", "", "热题 100", "已通过")
                .as_ref(),
            &[2]
        );
        let first = view.filter_in("链表", "全部", "", "热题 100", "全部");
        assert_eq!(first.as_ref(), &[2, 1], "按照官方题单顺序");
        assert!(Rc::ptr_eq(
            &first,
            &view.filter_in("链表", "全部", "", "热题 100", "全部")
        ));
    }
    #[test]
    fn redraw_reuses_results_and_progress_invalidates_categories_and_filter() {
        let mut problems = crate::catalog::builtin().unwrap();
        problems[0].title = "中文 Go SEARCH".into();
        let id = problems[0].id;
        let mut view = CatalogView::new(&problems, HashMap::new());
        let first = view.filter("全部题目", "全部", "中文 go search");
        assert_eq!(first.as_ref(), &[0]);
        assert!(Rc::ptr_eq(
            &first,
            &view.filter("全部题目", "全部", "中文 go search")
        ));
        assert!(view.filter("收藏", "全部", "").is_empty());
        let progress = Progress {
            favorite: true,
            state: "待复习".into(),
            tags: vec!["易错".into()],
        };
        view.update_progress(id, &progress);
        assert_eq!(view.filter("收藏", "全部", "").as_ref(), &[0]);
        assert_eq!(view.filter("待复习", "全部", "").as_ref(), &[0]);
        assert_eq!(view.filter("易错", "全部", "").as_ref(), &[0]);
        assert!(view.categories().iter().any(|t| t == "易错"));
        let cached = view.filter("易错", "全部", "");
        view.update_progress(id, &progress);
        assert!(Rc::ptr_eq(&cached, &view.filter("易错", "全部", "")));
        problems.reverse();
        view.replace(&problems);
        assert_eq!(
            view.filter("易错", "全部", "").as_ref(),
            &[problems.len() - 1]
        );
    }
}
