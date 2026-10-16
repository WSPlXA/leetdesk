use crate::catalog::{self, PracticeLanguage, Problem};
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static SUBMISSION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub const NODE_TYPES: &str = "package solution\n\n// LeetCode helpers; do not redeclare these types in solution.go.\ntype ListNode struct {\n\tVal int\n\tNext *ListNode\n}\n\ntype TreeNode struct {\n\tVal int\n\tLeft *TreeNode\n\tRight *TreeNode\n}\n";

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Progress {
    pub favorite: bool,
    pub state: String,
    pub tags: Vec<String>,
}

pub struct Store {
    pub root: PathBuf,
    connection: Connection,
}

impl Store {
    pub fn open(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(&root)?;
        let connection = Connection::open(root.join("leetdesk.sqlite3"))?;
        connection.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS progress (problem_id INTEGER PRIMARY KEY, favorite INTEGER NOT NULL DEFAULT 0, state TEXT NOT NULL DEFAULT '', tags TEXT NOT NULL DEFAULT '[]'); CREATE TABLE IF NOT EXISTS runs (id INTEGER PRIMARY KEY, problem_id INTEGER NOT NULL, created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP, passed INTEGER NOT NULL, total INTEGER NOT NULL, output TEXT NOT NULL, source TEXT NOT NULL);")?;
        Ok(Self { root, connection })
    }

    pub fn workspace(&self, id: u32) -> PathBuf {
        self.root.join("workspaces").join(id.to_string())
    }

    pub fn prepare(&self, problem: &Problem) -> Result<PathBuf> {
        let dir = self.workspace(problem.id);
        fs::create_dir_all(&dir)?;
        atomic_write(
            &dir.join("go.mod"),
            format!("module leetdesk.local/p{}\n\ngo 1.22\n", problem.id).as_bytes(),
        )?;
        atomic_write(
            &dir.join("types.go"),
            format!("{NODE_TYPES}\n{}", problem.extra_types).as_bytes(),
        )?;
        if problem.online.as_ref().is_some_and(|remote| !remote.loaded) {
            return Ok(dir);
        }
        let placeholder = fs::read_to_string(dir.join("solution.go"))
            .is_ok_and(|s| s.trim() == "package solution");
        if !dir.join("solution.go").exists() || placeholder {
            fs::write(dir.join("solution.go"), &problem.starter)?;
        }
        let java_placeholder = fs::read_to_string(dir.join("Solution.java"))
            .is_ok_and(|s| s.trim() == "// 这道题目前没有 Java 模板。");
        if !dir.join("Solution.java").exists()
            || (java_placeholder && !problem.java_starter.is_empty())
        {
            fs::write(
                dir.join("Solution.java"),
                problem.starter_for(PracticeLanguage::Java),
            )?;
        }
        if let Some(remote) = &problem.online
            && !dir.join("online-input.txt").exists()
        {
            fs::write(dir.join("online-input.txt"), &remote.sample_input)?;
        }
        if !dir.join("cases.json").exists() {
            fs::write(
                dir.join("cases.json"),
                serde_json::to_string_pretty(&problem.cases)?,
            )?;
        }
        Ok(dir)
    }

    pub fn source(&self, id: u32) -> Result<String> {
        self.source_for(id, PracticeLanguage::Go)
    }
    pub fn save_source(&self, id: u32, source: &str) -> Result<()> {
        self.save_source_for(id, PracticeLanguage::Go, source)
    }
    pub fn source_for(&self, id: u32, language: PracticeLanguage) -> Result<String> {
        Ok(fs::read_to_string(
            self.workspace(id).join(language.filename()),
        )?)
    }
    pub fn save_source_for(&self, id: u32, language: PracticeLanguage, source: &str) -> Result<()> {
        atomic_write(
            &self.workspace(id).join(language.filename()),
            source.as_bytes(),
        )
    }

    /// Keep the exact source sent by each online submission attempt.
    pub fn save_submission_snapshot(
        &self,
        id: u32,
        language: PracticeLanguage,
        source: &str,
    ) -> Result<PathBuf> {
        let dir = self.workspace(id).join("submissions");
        fs::create_dir_all(&dir)?;
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
        let ext = match language {
            PracticeLanguage::Go => "go",
            PracticeLanguage::Java => "java",
        };
        for _ in 0..100 {
            let sequence = SUBMISSION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = dir.join(format!("{timestamp}-{sequence}.{ext}"));
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    use std::io::Write;
                    file.write_all(source.as_bytes())?;
                    file.sync_all()?;
                    return Ok(path);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        bail!("无法生成唯一的提交快照文件名")
    }
    pub fn language(&self) -> PracticeLanguage {
        fs::read_to_string(self.root.join("language.json"))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }
    pub fn save_language(&self, language: PracticeLanguage) -> Result<()> {
        atomic_write(
            &self.root.join("language.json"),
            &serde_json::to_vec(&language)?,
        )
    }
    pub fn cases(&self, id: u32) -> Result<String> {
        Ok(fs::read_to_string(self.workspace(id).join("cases.json"))?)
    }
    pub fn save_cases(&self, id: u32, text: &str) -> Result<()> {
        atomic_write(&self.workspace(id).join("cases.json"), text.as_bytes())
    }

    pub fn progress(&self, id: u32) -> Progress {
        self.connection
            .query_row(
                "SELECT favorite, state, tags FROM progress WHERE problem_id = ?",
                [id],
                |row| {
                    let tags: String = row.get(2)?;
                    Ok(Progress {
                        favorite: row.get(0)?,
                        state: row.get(1)?,
                        tags: serde_json::from_str(&tags).unwrap_or_default(),
                    })
                },
            )
            .unwrap_or_default()
    }

    pub fn all_progress(&self) -> std::collections::HashMap<u32, Progress> {
        let Ok(mut statement) = self
            .connection
            .prepare("SELECT problem_id, favorite, state, tags FROM progress")
        else {
            return Default::default();
        };
        let Ok(rows) = statement.query_map([], |row| {
            let text: String = row.get(3)?;
            Ok((
                row.get(0)?,
                Progress {
                    favorite: row.get(1)?,
                    state: row.get(2)?,
                    tags: serde_json::from_str(&text).unwrap_or_default(),
                },
            ))
        }) else {
            return Default::default();
        };
        rows.filter_map(Result::ok).collect()
    }

    pub fn save_progress(&self, id: u32, progress: &Progress) -> Result<()> {
        self.connection.execute("INSERT INTO progress(problem_id, favorite, state, tags) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(problem_id) DO UPDATE SET favorite=excluded.favorite, state=excluded.state, tags=excluded.tags WHERE favorite != excluded.favorite OR state != excluded.state OR tags != excluded.tags", params![id, progress.favorite, progress.state, serde_json::to_string(&progress.tags)?])?;
        Ok(())
    }

    pub fn save_run(
        &self,
        id: u32,
        source: &str,
        passed: usize,
        total: usize,
        output: &str,
    ) -> Result<()> {
        self.connection.execute("INSERT INTO runs(problem_id, passed, total, output, source) VALUES (?1, ?2, ?3, ?4, ?5)", params![id, passed as i64, total as i64, output, source])?;
        Ok(())
    }

    pub fn latest_run(&self, id: u32) -> String {
        self.connection
            .query_row(
                "SELECT output FROM runs WHERE problem_id = ? ORDER BY id DESC LIMIT 1",
                [id],
                |r| r.get(0),
            )
            .unwrap_or_else(|_| {
                "运行样例后，结果会显示在这里。\n本地通过只代表这些用例通过。".into()
            })
    }

    pub fn notes(&self, problem: &Problem) -> Result<Vec<(String, String)>> {
        let mut notes = vec![("内置讲解".into(), problem.explanation.clone())];
        let dir = self.root.join("notes").join(problem.id.to_string());
        if dir.exists() {
            let mut paths: Vec<_> = fs::read_dir(dir)?
                .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                .collect();
            paths.sort();
            for path in paths {
                if is_markdown(&path) {
                    notes.push((
                        path.file_name().unwrap().to_string_lossy().into_owned(),
                        fs::read_to_string(path)?,
                    ));
                }
            }
        }
        Ok(notes)
    }

    pub fn import_note(
        &self,
        path: &Path,
        fallback: u32,
        problems: &[Problem],
    ) -> Result<(u32, PathBuf)> {
        if !is_markdown(path) {
            bail!("请选择 .md 或 .markdown 文件");
        }
        let content =
            fs::read_to_string(path).with_context(|| format!("无法读取 {}", path.display()))?;
        let filename = path.file_name().context("文件名无效")?.to_string_lossy();
        let explicit = catalog::note_metadata(&content).0;
        if explicit.is_some_and(|id| !problems.iter().any(|p| p.id == id)) {
            bail!(
                "笔记中的题号 {} 尚未导入题库，请先导入题目 JSON",
                explicit.unwrap()
            );
        }
        let id = catalog::associate_note(&filename, &content, problems).unwrap_or(fallback);
        let dir = self.root.join("notes").join(id.to_string());
        fs::create_dir_all(&dir)?;
        let stem = path.file_stem().unwrap().to_string_lossy();
        let mut destination = dir.join(filename.as_ref());
        let mut i = 2;
        while destination.exists() {
            destination = dir.join(format!("{stem}-{i}.md"));
            i += 1;
        }
        fs::write(&destination, &content)?;
        // Preserve relative image/link resolution in the original document.
        fs::write(
            destination.with_extension("origin"),
            path.canonicalize()?.to_string_lossy().as_bytes(),
        )?;
        let mut progress = self.progress(id);
        for tag in catalog::note_metadata(&content).1 {
            if !progress.tags.contains(&tag) {
                progress.tags.push(tag);
            }
        }
        self.save_progress(id, &progress)?;
        Ok((id, destination))
    }

    pub fn load_catalog(&self) -> Result<Vec<Problem>> {
        if let Ok(text) = fs::read_to_string(self.root.join("online-active.json"))
            && let Ok(site) = serde_json::from_str::<crate::online::Site>(&text)
            && let Some(cache) = self.online_cache(site)?
            && !cache.problems.is_empty()
        {
            return Ok(cache.problems);
        }
        let mut problems = catalog::builtin()?;
        let path = self.root.join("catalog.json");
        if path.exists() {
            let imported: Vec<Problem> = serde_json::from_str(&fs::read_to_string(path)?)?;
            for problem in imported {
                catalog::validate(&problem)?;
                if let Some(existing) = problems.iter_mut().find(|p| p.id == problem.id) {
                    *existing = problem;
                } else {
                    problems.push(problem);
                }
            }
        }
        problems.sort_by_key(|p| p.id);
        Ok(problems)
    }

    pub fn online_cache(
        &self,
        site: crate::online::Site,
    ) -> Result<Option<crate::online::OnlineCache>> {
        let path = self
            .root
            .join("leetcode")
            .join(site.key())
            .join("index.json");
        if !path.exists() {
            return Ok(None);
        }
        let mut cache = serde_json::from_str(&fs::read_to_string(path)?)?;
        self.hydrate_online_details(&mut cache)?;
        Ok(Some(cache))
    }

    pub fn study_plan(&self, site: crate::online::Site) -> crate::online::StudyPlan {
        fs::read_to_string(
            self.root
                .join("leetcode")
                .join(site.key())
                .join("hot100.json"),
        )
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .filter(|plan: &crate::online::StudyPlan| plan.validate().is_ok())
        .unwrap_or_else(crate::online::builtin_hot100)
    }

    pub fn save_study_plan(
        &self,
        site: crate::online::Site,
        plan: &crate::online::StudyPlan,
    ) -> Result<()> {
        plan.validate()?;
        let dir = self.root.join("leetcode").join(site.key());
        fs::create_dir_all(&dir)?;
        atomic_write(&dir.join("hot100.json"), &serde_json::to_vec(plan)?)
    }

    pub fn save_online_cache(&self, cache: &mut crate::online::OnlineCache) -> Result<()> {
        let dir = self.root.join("leetcode").join(cache.site.key());
        fs::create_dir_all(dir.join("details"))?;
        self.hydrate_online_details(cache)?;
        cache.problems.sort_by_key(|a| a.id);
        atomic_write(
            &dir.join("index.json"),
            serde_json::to_vec(cache)?.as_slice(),
        )?;
        atomic_write(
            &self.root.join("online-active.json"),
            serde_json::to_vec(&cache.site)?.as_slice(),
        )?;
        Ok(())
    }

    fn hydrate_online_details(&self, cache: &mut crate::online::OnlineCache) -> Result<()> {
        let dir = self
            .root
            .join("leetcode")
            .join(cache.site.key())
            .join("details");
        let mut details = std::collections::HashMap::new();
        if dir.exists() {
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                if entry.path().extension().is_some_and(|e| e == "json")
                    && let Ok(text) = fs::read_to_string(entry.path())
                    && let Ok(detail) = serde_json::from_str::<Problem>(&text)
                {
                    details.insert(detail.slug.clone(), detail);
                }
            }
        }
        for problem in &mut cache.problems {
            if !crate::online::valid_slug(&problem.slug) {
                bail!("在线题目 slug 无效");
            }
            if let Some(mut detail) = details.remove(&problem.slug)
                && detail.id == problem.id
                && detail
                    .online
                    .as_ref()
                    .is_some_and(|m| m.site == cache.site && m.loaded)
            {
                let latest = problem.online.as_ref().context("在线题目元数据缺失")?;
                let remote = detail.online.as_mut().unwrap();
                remote.status = latest.status.clone();
                remote.paid_only = latest.paid_only;
                if !problem.tags.is_empty() {
                    detail.tags = problem.tags.clone();
                }
                *problem = detail;
            }
        }
        Ok(())
    }

    pub fn save_online_detail(&self, problem: &Problem) -> Result<()> {
        let remote = problem.online.as_ref().context("在线元数据缺失")?;
        if !crate::online::valid_slug(&problem.slug) {
            bail!("题目 slug 无效");
        }
        let dir = self
            .root
            .join("leetcode")
            .join(remote.site.key())
            .join("details");
        fs::create_dir_all(&dir)?;
        // Completion status is account data; details on disk deliberately omit it.
        let mut content = problem.clone();
        content.online.as_mut().unwrap().status = None;
        atomic_write(
            &dir.join(format!("{}.json", problem.slug)),
            serde_json::to_vec(&content)?.as_slice(),
        )
    }

    pub fn online_input(&self, problem: &Problem) -> String {
        fs::read_to_string(self.workspace(problem.id).join("online-input.txt")).unwrap_or_else(
            |_| {
                problem
                    .online
                    .as_ref()
                    .map(|r| r.sample_input.clone())
                    .unwrap_or_default()
            },
        )
    }

    pub fn save_online_input(&self, id: u32, input: &str) -> Result<()> {
        atomic_write(
            &self.workspace(id).join("online-input.txt"),
            input.as_bytes(),
        )
    }

    pub fn import_catalog(&self, path: &Path) -> Result<usize> {
        let text = fs::read_to_string(path)?;
        let imported: Vec<Problem> = if text.trim_start().starts_with('[') {
            serde_json::from_str(&text)?
        } else {
            vec![serde_json::from_str(&text)?]
        };
        for problem in &imported {
            catalog::validate(problem)?;
        }
        let count = imported.len();
        let mut problems = self.load_catalog()?;
        for problem in imported {
            if let Some(existing) = problems.iter_mut().find(|p| p.id == problem.id) {
                *existing = problem;
            } else {
                problems.push(problem);
            }
        }
        atomic_write(
            &self.root.join("catalog.json"),
            serde_json::to_string_pretty(&problems)?.as_bytes(),
        )?;
        Ok(count)
    }
}

pub fn is_markdown(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown"))
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    if fs::read(path).is_ok_and(|current| current == bytes) {
        return Ok(());
    }
    let temporary = path.with_extension("saving");
    fs::write(&temporary, bytes)?;
    fs::rename(&temporary, path).context("无法保存文件")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn study_plan_cache_survives_restart_and_keeps_valid_snapshot_on_bad_update() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("data");
        let store = Store::open(root.clone()).unwrap();
        let mut plan = crate::online::builtin_hot100();
        plan.synced_at = 42;
        store
            .save_study_plan(crate::online::Site::China, &plan)
            .unwrap();
        let mut partial = plan.clone();
        partial.groups[0].questions.pop();
        assert!(
            store
                .save_study_plan(crate::online::Site::China, &partial)
                .is_err()
        );
        drop(store);
        let reopened = Store::open(root).unwrap();
        assert_eq!(
            reopened.study_plan(crate::online::Site::China).synced_at,
            42
        );
        let path = reopened.root.join("leetcode/cn/hot100.json");
        fs::write(path, "{ broken cache").unwrap();
        assert_eq!(
            reopened
                .study_plan(crate::online::Site::China)
                .groups
                .iter()
                .map(|g| g.questions.len())
                .sum::<usize>(),
            100
        );
    }
    #[test]
    fn unchanged_saves_do_not_rewrite_files_or_progress() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("data")).unwrap();
        let problem = crate::catalog::builtin().unwrap().remove(0);
        let workspace = store.prepare(&problem).unwrap();
        let source = workspace.join("solution.go");
        let before = fs::metadata(&source).unwrap().modified().unwrap();
        store.save_source(problem.id, &problem.starter).unwrap();
        assert_eq!(fs::metadata(&source).unwrap().modified().unwrap(), before);
        let progress = Progress {
            favorite: true,
            ..Default::default()
        };
        store.save_progress(problem.id, &progress).unwrap();
        store.save_progress(problem.id, &progress).unwrap();
        assert_eq!(store.connection.changes(), 0);
        store.save_source(problem.id, "changed").unwrap();
        assert_eq!(store.source(problem.id).unwrap(), "changed");
        assert!(!source.with_extension("saving").exists());
    }
    #[test]
    fn cached_details_survive_restart_without_reusing_another_accounts_status() {
        use crate::online::{self, OnlineCache, Site};
        use serde_json::json;
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("data")).unwrap();
        let mut page = online::parse_page(Site::China, &json!({"data":{"problemsetQuestionList":{"total":1,"questions":[{"frontendQuestionId":"1","titleSlug":"two-sum","title":"Two Sum","status":"ac","difficulty":"Easy"}]}}})).unwrap();
        let mut detail = page.problems[0].clone();
        detail.online.as_mut().unwrap().loaded = true;
        detail.online.as_mut().unwrap().go_supported = true;
        detail.starter =
            "package solution\nfunc twoSum(nums []int, target int) []int { return nil }".into();
        let mut cache = OnlineCache {
            site: Site::China,
            username: "alice".into(),
            problems: page.problems.clone(),
            synced_at: 1,
        };
        store.save_online_cache(&mut cache).unwrap();
        store.save_online_detail(&detail).unwrap();
        let restored = store.online_cache(Site::China).unwrap().unwrap();
        assert!(restored.problems[0].online.as_ref().unwrap().loaded);
        assert_eq!(online::status_label(&restored.problems[0]), "已通过");
        store.prepare(&restored.problems[0]).unwrap();
        store
            .save_source(1, "package solution\n// my own draft")
            .unwrap();
        // A new login must replace the status while keeping downloaded content and code.
        page.problems[0].online.as_mut().unwrap().status = None;
        cache.username = "bob".into();
        cache.problems = page.problems;
        store.save_online_cache(&mut cache).unwrap();
        let restored = store.online_cache(Site::China).unwrap().unwrap();
        assert_eq!(restored.username, "bob");
        assert_eq!(online::status_label(&restored.problems[0]), "未解答");
        assert!(restored.problems[0].starter.contains("twoSum"));
        store.prepare(&restored.problems[0]).unwrap();
        assert!(store.source(1).unwrap().contains("my own draft"));
        assert!(store.online_cache(Site::Global).unwrap().is_none());
    }
    #[test]
    fn practice_and_import_survive_restart_without_overwriting_notes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("data");
        let problems = catalog::builtin().unwrap();
        let store = Store::open(root.clone()).unwrap();
        store.prepare(&problems[0]).unwrap();
        store.save_source(1, "package solution\n// saved").unwrap();
        let note = temp.path().join("1.md");
        fs::write(&note, "---\nproblem_id: 1\ntags: [复习]\n---\n# 原文").unwrap();
        let first = store.import_note(&note, 20, &problems).unwrap().1;
        let second = store.import_note(&note, 20, &problems).unwrap().1;
        assert_ne!(first, second);
        drop(store);
        let reopened = Store::open(root).unwrap();
        reopened.prepare(&problems[0]).unwrap();
        assert!(reopened.source(1).unwrap().contains("saved"));
        assert_eq!(reopened.progress(1).tags, vec!["复习"]);
        assert_eq!(reopened.notes(&problems[0]).unwrap().len(), 3);
    }
    #[test]
    fn go_and_java_drafts_are_separate_and_language_is_persistent() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("data");
        let problem = catalog::builtin().unwrap().remove(0);
        let store = Store::open(root.clone()).unwrap();
        store.prepare(&problem).unwrap();
        assert!(
            store
                .source_for(problem.id, PracticeLanguage::Java)
                .unwrap()
                .contains("class Solution")
        );
        store
            .save_source_for(problem.id, PracticeLanguage::Java, "class Solution { }\n")
            .unwrap();
        store.save_language(PracticeLanguage::Java).unwrap();
        drop(store);
        let store = Store::open(root).unwrap();
        store.prepare(&problem).unwrap();
        assert_eq!(store.language(), PracticeLanguage::Java);
        assert_eq!(
            store
                .source_for(problem.id, PracticeLanguage::Java)
                .unwrap(),
            "class Solution { }\n"
        );
        assert!(
            store
                .source_for(problem.id, PracticeLanguage::Go)
                .unwrap()
                .starts_with("package solution")
        );
    }
}
