//! LeetCode payloads and request planning. Authentication stays in WebView2;
//! requests run in the official origin, so Rust never reads passwords or session cookies.
use crate::catalog::{Kind, Parameter, Problem};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Site {
    #[default]
    China,
    Global,
}

impl Site {
    pub fn origin(self) -> &'static str {
        match self {
            Self::China => "https://leetcode.cn",
            Self::Global => "https://leetcode.com",
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Self::China => "cn",
            Self::Global => "global",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::China => "中国站",
            Self::Global => "国际站",
        }
    }
    pub fn accepts(self, origin: &str) -> bool {
        url::Url::parse(origin).is_ok_and(|url| {
            url.scheme() == "https"
                && url.host_str()
                    == url::Url::parse(self.origin())
                        .ok()
                        .and_then(|origin| origin.host_str().map(str::to_owned))
                        .as_deref()
                && url.port_or_known_default() == Some(443)
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemoteMeta {
    pub site: Site,
    pub frontend_id: String,
    pub question_id: String,
    pub status: Option<String>,
    pub paid_only: bool,
    pub loaded: bool,
    #[serde(default)]
    pub sample_input: String,
    #[serde(default)]
    pub go_supported: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OnlineCache {
    pub site: Site,
    pub username: String,
    pub problems: Vec<Problem>,
    pub synced_at: u64,
}

#[derive(Debug)]
pub struct Page {
    pub total: usize,
    pub problems: Vec<Problem>,
}

#[derive(Clone, Debug)]
pub struct Request {
    pub path: String,
    pub body: Option<Value>,
}

pub const PAGE_SIZE: usize = 100;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StudyPlan {
    pub slug: String,
    pub title: String,
    pub groups: Vec<StudyGroup>,
    pub synced_at: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StudyGroup {
    pub name: String,
    pub questions: Vec<String>,
}

impl StudyPlan {
    pub fn validate(&self) -> Result<()> {
        let mut seen = std::collections::HashSet::new();
        if self.slug != "top-100-liked" {
            bail!("未知题单");
        }
        for slug in self.groups.iter().flat_map(|group| &group.questions) {
            if !valid_slug(slug) || !seen.insert(slug) {
                bail!("题单包含无效或重复题目");
            }
        }
        if seen.len() != 100 {
            bail!("热题 100 列表不完整：{} / 100", seen.len());
        }
        Ok(())
    }
}

pub fn builtin_hot100() -> StudyPlan {
    serde_json::from_str(include_str!("../assets/hot100.json")).expect("内置题单无效")
}

pub fn study_plan_request() -> Request {
    graphql(
        "LeetDeskHot100",
        "query LeetDeskHot100($planSlug:String!) { userStatus { isSignedIn username } studyPlanV2Detail(planSlug:$planSlug) { planSubGroups { name questions { titleSlug } } } }",
        json!({"planSlug":"top-100-liked"}),
    )
}

pub fn parse_study_plan(body: &Value) -> Result<StudyPlan> {
    let groups = checked_data(body)?["studyPlanV2Detail"]["planSubGroups"]
        .as_array()
        .context("热题 100 响应缺少分组")?;
    let mut seen = std::collections::HashSet::new();
    let groups: Vec<StudyGroup> = groups
        .iter()
        .map(|group| {
            let rows = group["questions"].as_array().context("题单分组缺少题目")?;
            let questions: Vec<_> = rows
                .iter()
                .map(|row| {
                    let slug = row["titleSlug"].as_str().context("题单缺少题目标识")?;
                    if !valid_slug(slug) || !seen.insert(slug.to_string()) {
                        bail!("题单包含无效或重复题目");
                    }
                    Ok(slug.to_string())
                })
                .collect::<Result<_>>()?;
            Ok(StudyGroup {
                name: crate::topics::label(group["name"].as_str().unwrap_or("其他")).into(),
                questions,
            })
        })
        .collect::<Result<_>>()?;
    if seen.len() != 100 {
        bail!("热题 100 列表不完整：{} / 100", seen.len());
    }
    Ok(StudyPlan {
        slug: "top-100-liked".into(),
        title: "热题 100".into(),
        groups,
        synced_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs(),
    })
}

pub fn account_request() -> Request {
    graphql(
        "LeetDeskAccount",
        "query LeetDeskAccount { userStatus { isSignedIn username } }",
        json!({}),
    )
}

pub fn list_request(site: Site, skip: usize) -> Request {
    let field = match site {
        Site::China => {
            "problemsetQuestionList(categorySlug:$categorySlug,limit:$limit,skip:$skip,filters:$filters) { total questions { frontendQuestionId title titleCn titleSlug difficulty status paidOnly topicTags { name nameTranslated slug } } }"
        }
        Site::Global => {
            "problemsetQuestionList: questionList(categorySlug:$categorySlug,limit:$limit,skip:$skip,filters:$filters) { total: totalNum questions: data { frontendQuestionId: questionFrontendId title titleSlug difficulty status paidOnly: isPaidOnly topicTags { name slug } } }"
        }
    };
    let query = format!(
        "query LeetDeskQuestions($categorySlug:String,$limit:Int,$skip:Int,$filters:QuestionListFilterInput) {{ userStatus {{ isSignedIn username }} {field} }}"
    );
    graphql(
        "LeetDeskQuestions",
        &query,
        json!({"categorySlug":"","limit":PAGE_SIZE,"skip":skip,"filters":{}}),
    )
}

pub fn legacy_list_request() -> Request {
    Request {
        path: "/api/problems/all/".into(),
        body: None,
    }
}

pub fn detail_request(site: Site, slug: &str) -> Result<Request> {
    if !valid_slug(slug) {
        bail!("题目 slug 无效");
    }
    let translated = if site == Site::China {
        "translatedTitle translatedContent"
    } else {
        ""
    };
    Ok(graphql(
        "LeetDeskQuestion",
        &format!(
            "query LeetDeskQuestion($titleSlug:String!) {{ userStatus {{ isSignedIn username }} question(titleSlug:$titleSlug) {{ questionId questionFrontendId title titleSlug content {translated} difficulty isPaidOnly status sampleTestCase exampleTestcases codeSnippets {{ langSlug code }} metaData topicTags {{ name slug }} }} }}"
        ),
        json!({"titleSlug":slug}),
    ))
}

pub fn run_request(problem: &Problem, source: &str, input: &str) -> Result<Request> {
    let remote = problem.online.as_ref().context("该题不是在线题目")?;
    if !remote.loaded || remote.question_id.is_empty() || !remote.go_supported {
        bail!("请先加载支持 Go 的题目详情");
    }
    Ok(Request {
        path: format!("/problems/{}/interpret_solution/", problem.slug),
        body: Some(
            json!({"lang":"golang","question_id":remote.question_id,"typed_code":submission_source(source),"data_input":input}),
        ),
    })
}

pub fn submit_request(problem: &Problem, source: &str) -> Result<Request> {
    let mut request = run_request(problem, source, "")?;
    request.path = format!("/problems/{}/submit/", problem.slug);
    request
        .body
        .as_mut()
        .unwrap()
        .as_object_mut()
        .unwrap()
        .remove("data_input");
    Ok(request)
}

pub fn judge_id(body: &Value, submit: bool) -> Result<String> {
    let key = if submit {
        "submission_id"
    } else {
        "interpret_id"
    };
    let id = string_value(&body[key]);
    if id.is_empty() {
        if let Some(message) = platform_message(body) {
            bail!("LeetCode 未创建判题任务：{message}");
        }
        let fields = body
            .as_object()
            .map(|object| {
                object
                    .keys()
                    .take(12)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .filter(|fields| !fields.is_empty())
            .unwrap_or_else(|| "空响应或非对象响应".into());
        bail!("平台响应缺少 {key}（返回字段：{fields}）");
    }
    check_request(&id).context("平台返回的任务标识格式不受支持")?;
    Ok(id)
}

fn platform_message(body: &Value) -> Option<String> {
    for key in ["error", "detail", "message", "msg", "error_message"] {
        if let Some(message) = body[key]
            .as_str()
            .filter(|message| !message.trim().is_empty())
        {
            return Some(message.chars().take(500).collect());
        }
        if let Some(error) = body[key].as_object()
            && let Some(message) = error.get("message").and_then(Value::as_str)
        {
            return Some(message.chars().take(500).collect());
        }
    }
    None
}

pub fn judge_finished(body: &Value) -> bool {
    body["state"] == "SUCCESS"
        || body["state"] == "FAILURE"
        || body["status_code"].as_i64().is_some()
}

pub fn submission_accepted(body: &Value) -> bool {
    body["state"] != "FAILURE" && body["status_code"] == 10
}

pub fn check_request(id: &str) -> Result<Request> {
    if id.is_empty()
        || id.len() > 256
        || matches!(id, "." | "..")
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        bail!("判题任务标识无效");
    }
    Ok(Request {
        path: format!("/submissions/detail/{id}/check/"),
        body: None,
    })
}

pub fn submission_source(source: &str) -> String {
    let mut removed = false;
    source
        .lines()
        .filter(|line| {
            if !removed && line.trim_start().starts_with("package ") {
                removed = true;
                false
            } else {
                true
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn graphql(name: &str, query: &str, variables: Value) -> Request {
    Request {
        path: "/graphql/".into(),
        body: Some(json!({"operationName":name,"query":query,"variables":variables})),
    }
}

pub fn checked_data(body: &Value) -> Result<&Value> {
    if let Some(errors) = body["errors"].as_array().filter(|items| !items.is_empty()) {
        bail!(
            "LeetCode 接口返回错误：{}",
            errors
                .iter()
                .filter_map(|e| e["message"].as_str())
                .collect::<Vec<_>>()
                .join("；")
        );
    }
    body.get("data").context("LeetCode 返回了无法识别的响应")
}

pub fn parse_account(body: &Value) -> Result<Option<String>> {
    let status = &checked_data(body)?["userStatus"];
    match status["isSignedIn"].as_bool() {
        Some(true) => Ok(Some(
            status["username"]
                .as_str()
                .filter(|s| !s.is_empty())
                .context("账号响应缺少用户名")?
                .into(),
        )),
        Some(false) => Ok(None),
        None => bail!("无法验证账号登录状态"),
    }
}

pub fn parse_page(site: Site, body: &Value) -> Result<Page> {
    let list = &checked_data(body)?["problemsetQuestionList"];
    let total = list["total"].as_u64().context("题库响应缺少总数")? as usize;
    let rows = list["questions"]
        .as_array()
        .context("题库响应缺少题目列表")?;
    let mut problems = vec![];
    for row in rows {
        let frontend = string_value(&row["frontendQuestionId"]);
        let slug = row["titleSlug"].as_str().context("题目缺少 slug")?;
        let mut problem = stub(
            site,
            &frontend,
            slug,
            row["titleCn"]
                .as_str()
                .filter(|s| !s.is_empty())
                .or(row["title"].as_str())
                .unwrap_or(slug),
        )?;
        problem.difficulty = difficulty(row["difficulty"].as_str().unwrap_or(""));
        problem.tags = tags(&row["topicTags"]);
        let remote = problem.online.as_mut().unwrap();
        remote.status = normalize_status(&row["status"]);
        remote.paid_only = row["paidOnly"].as_bool().unwrap_or(false);
        problems.push(problem);
    }
    Ok(Page { total, problems })
}

pub fn parse_legacy(site: Site, body: &Value) -> Result<Page> {
    let rows = body["stat_status_pairs"]
        .as_array()
        .context("题库接口不可用；请在登录页完成验证后重试")?;
    let mut problems = vec![];
    for row in rows {
        let stat = &row["stat"];
        let frontend = string_value(&stat["frontend_question_id"]);
        let Some(slug) = stat["question__title_slug"].as_str() else {
            continue;
        };
        let mut problem = stub(
            site,
            &frontend,
            slug,
            stat["question__title"].as_str().unwrap_or(slug),
        )?;
        problem.difficulty = match row["difficulty"]["level"].as_u64() {
            Some(1) => "简单",
            Some(2) => "中等",
            Some(3) => "困难",
            _ => "未知",
        }
        .into();
        let remote = problem.online.as_mut().unwrap();
        remote.question_id = string_value(&stat["question_id"]);
        remote.status = normalize_status(&row["status"]);
        remote.paid_only = row["paid_only"].as_bool().unwrap_or(false);
        problems.push(problem);
    }
    Ok(Page {
        total: problems.len(),
        problems,
    })
}

fn stub(site: Site, frontend: &str, slug: &str, title: &str) -> Result<Problem> {
    if !valid_slug(slug) || frontend.is_empty() {
        bail!("无效的在线题目标识");
    }
    // Non-numeric IDs (LCP, LCR, interview problems) use a stable slug hash.
    let id = frontend.parse::<u32>().unwrap_or_else(|_| {
        1_000_000
            + (slug.bytes().fold(2166136261_u32, |hash, c| {
                (hash ^ c as u32).wrapping_mul(16777619)
            }) % 1_000_000_000)
    });
    Ok(Problem {
        id,
        slug: slug.into(),
        title: title.into(),
        difficulty: String::new(),
        tags: vec![],
        description: String::new(),
        starter: "package solution\n".into(),
        function: String::new(),
        parameters: vec![],
        returns: Kind::Int,
        unordered: false,
        cases: vec![],
        explanation: String::new(),
        extra_types: String::new(),
        online: Some(RemoteMeta {
            site,
            frontend_id: frontend.into(),
            question_id: String::new(),
            status: None,
            paid_only: false,
            loaded: false,
            sample_input: String::new(),
            go_supported: false,
        }),
    })
}

pub fn parse_detail(existing: &Problem, body: &Value) -> Result<Problem> {
    let row = &checked_data(body)?["question"];
    if row.is_null() {
        bail!("题目详情为空，可能需要会员权限");
    }
    let mut problem = existing.clone();
    let remote = problem.online.as_mut().context("在线题目元数据缺失")?;
    remote.loaded = true;
    remote.question_id = string_value(&row["questionId"]);
    if let Some(frontend) = row["questionFrontendId"].as_str() {
        remote.frontend_id = frontend.into();
    }
    remote.status = normalize_status(&row["status"]);
    remote.paid_only = row["isPaidOnly"].as_bool().unwrap_or(remote.paid_only);
    remote.sample_input = row["exampleTestcases"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or(row["sampleTestCase"].as_str())
        .unwrap_or("")
        .into();
    problem.title = row["translatedTitle"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or(row["title"].as_str())
        .unwrap_or(&problem.title)
        .into();
    problem.description = row["translatedContent"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or(row["content"].as_str())
        .unwrap_or("")
        .into();
    if problem.description.is_empty() {
        bail!("题面不可用，可能需要会员权限");
    }
    problem.tags = tags(&row["topicTags"]);
    let snippet = row["codeSnippets"]
        .as_array()
        .and_then(|rows| rows.iter().find(|s| s["langSlug"] == "golang"))
        .and_then(|s| s["code"].as_str());
    remote.go_supported = snippet.is_some();
    if let Some(code) = snippet {
        problem.starter = format!("package solution\n\n{code}\n");
        problem.extra_types = extra_types(code);
    } else {
        problem.starter = "package solution\n\n// 这道题目前没有 Go 模板。\n".into();
    }
    if let Some(metadata) = row["metaData"]
        .as_str()
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
    {
        problem.function = metadata["name"].as_str().unwrap_or("").into();
        problem.parameters = metadata["params"]
            .as_array()
            .map(|parameters| {
                parameters
                    .iter()
                    .filter_map(|p| {
                        Some(Parameter {
                            name: p["name"].as_str()?.into(),
                            kind: parse_kind(p["type"].as_str()?)?,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        problem.returns = metadata["return"]["type"]
            .as_str()
            .and_then(parse_kind)
            .unwrap_or(Kind::Int);
    }
    Ok(problem)
}

fn extra_types(code: &str) -> String {
    let mut output = String::new();
    let mut rest = code;
    while let Some(start) = rest.find("/*") {
        rest = &rest[start + 2..];
        let Some(end) = rest.find("*/") else {
            break;
        };
        let comment = rest[..end]
            .lines()
            .map(|s| s.trim_start().strip_prefix("* ").unwrap_or(s.trim_start()))
            .collect::<Vec<_>>()
            .join("\n");
        let mut definitions = comment.as_str();
        while let Some(offset) = definitions.find("type ") {
            definitions = &definitions[offset..];
            let Some(open) = definitions.find('{') else {
                break;
            };
            let Some(close) = definitions[open..].find('}') else {
                break;
            };
            let end = open + close + 1;
            let declaration = &definitions[..end];
            if !declaration.starts_with("type ListNode ")
                && !declaration.starts_with("type TreeNode ")
                && declaration[..open].contains("struct")
            {
                output.push_str(declaration);
                output.push('\n');
            }
            definitions = &definitions[end..];
        }
        rest = &rest[end + 2..];
    }
    output
}

fn parse_kind(value: &str) -> Option<Kind> {
    match value {
        "integer" | "int" => Some(Kind::Int),
        "integer[]" | "int[]" => Some(Kind::Ints),
        "string" => Some(Kind::String),
        "boolean" | "bool" => Some(Kind::Bool),
        "ListNode" => Some(Kind::List),
        "TreeNode" => Some(Kind::Tree),
        _ => None,
    }
}
fn difficulty(value: &str) -> String {
    match value {
        "Easy" | "EASY" => "简单",
        "Medium" | "MEDIUM" => "中等",
        "Hard" | "HARD" => "困难",
        _ => value,
    }
    .into()
}
fn tags(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|xs| {
            xs.iter()
                .filter_map(|tag| {
                    tag["nameTranslated"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .or(tag["name"].as_str())
                })
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}
fn string_value(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_u64().map(|n| n.to_string()))
        .unwrap_or_default()
}
fn normalize_status(value: &Value) -> Option<String> {
    match value.as_str() {
        Some("ac" | "AC" | "SOLVED") => Some("ac".into()),
        Some("notac" | "TRIED") => Some("notac".into()),
        _ => None,
    }
}
pub fn valid_slug(value: &str) -> bool {
    !value.is_empty()
        && value.len() < 200
        && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}
pub fn status_label(problem: &Problem) -> &'static str {
    match problem.online.as_ref().and_then(|m| m.status.as_deref()) {
        Some("ac") => "已通过",
        Some("notac") => "尝试过",
        _ => "未解答",
    }
}

/// Request URLs are fixed same-origin paths, never caller-supplied absolute URLs.
pub fn script(id: u64, request: &Request) -> Result<String> {
    if !request.path.starts_with('/')
        || request.path.starts_with("//")
        || request.path.contains('\\')
    {
        bail!("请求必须属于当前 LeetCode 站点");
    }
    let path = serde_json::to_string(&request.path)?;
    let body = serde_json::to_string(&request.body)?;
    Ok(format!(
        r#"(async()=>{{
const id={id};
try{{
    const body={body};const path={path};
    const question=path.match(/^\/problems\/[^/]+\//);
    const csrf=(document.cookie.split(';').map(v=>v.trim()).find(v=>v.startsWith('csrftoken='))||'').slice(10);
    const response=await fetch(path,{{method:body===null?'GET':'POST',credentials:'include',referrer:question?location.origin+question[0]:location.href,headers:body===null?{{}}:{{'Content-Type':'application/json','X-CSRFToken':decodeURIComponent(csrf),'X-Requested-With':'XMLHttpRequest'}},body:body===null?undefined:JSON.stringify(body)}});
    const text=await response.text();
    let data;try{{data=JSON.parse(text)}}catch{{data=undefined}}
    const message=data&&[data.error,data.detail,data.message,data.msg,data.error_message].map(v=>typeof v==='string'?v:v&&typeof v.message==='string'?v.message:'').find(v=>v.trim());
    const challenge=response.headers.get('cf-mitigated')==='challenge';
    const reason=response.status===401?'登录已失效，请重新登录':response.status===429?'请求过于频繁，请稍后重试':response.status===403?(challenge?'官方验证尚未完成，请在题目网页完成验证':'请求被拒绝，请在题目网页检查登录和访问权限'):response.status>=500?'LeetCode 服务暂时不可用，请稍后重试':data===undefined?'网站返回了登录或验证页面，请在官方题目网页完成验证后重试':'请求未成功';
    if(!response.ok||data===undefined)throw Error('LeetCode HTTP '+response.status+'：'+reason+(message?'；'+message.slice(0,500):''));
    window.ipc.postMessage(JSON.stringify({{kind:'response',id,ok:true,data}}));
}}catch(error){{window.ipc.postMessage(JSON.stringify({{kind:'response',id,ok:false,error:String(error.message||error)}}))}}
}})();"#
    ))
}

pub fn run_output(value: &Value) -> String {
    let mut text = String::new();
    for (key, label) in [
        ("status_msg", "判题状态"),
        ("state", "任务状态"),
        ("submission_id", "提交编号"),
        ("last_testcase", "失败用例"),
        ("total_correct", "通过用例"),
        ("total_testcases", "用例总数"),
        ("status_runtime", "耗时"),
        ("status_memory", "内存"),
        ("compile_error", "编译错误"),
        ("full_compile_error", "编译详情"),
        ("runtime_error", "运行错误"),
        ("full_runtime_error", "运行详情"),
        ("code_output", "实际输出"),
        ("expected_output", "预期输出"),
        ("std_output", "程序输出"),
    ] {
        if let Some(data) = value.get(key).filter(|v| !v.is_null()) {
            text.push_str(&format!(
                "{label}：{}\n",
                data.as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| data.to_string())
            ));
        }
    }
    if text.is_empty() {
        text = "LeetCode 已返回运行结果。".into();
    }
    format!("LeetCode 在线运行\n{text}\n运行样例通过不等于题目已提交通过。")
}

pub fn submission_output(value: &Value) -> String {
    let output = run_output(value);
    format!(
        "LeetCode 提交判题\n{}",
        output
            .strip_prefix("LeetCode 在线运行\n")
            .unwrap_or(&output)
            .replace("\n运行样例通过不等于题目已提交通过。", "")
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn official_hot100_snapshot_is_complete_and_partial_updates_are_rejected() {
        let plan = super::builtin_hot100();
        let slugs: Vec<_> = plan
            .groups
            .iter()
            .flat_map(|g| g.questions.iter())
            .collect();
        assert_eq!(slugs.len(), 100);
        assert_eq!(
            slugs.iter().collect::<std::collections::HashSet<_>>().len(),
            100
        );
        assert_eq!(slugs[0], "two-sum");
        let groups: Vec<_> = plan.groups.iter().map(|g| serde_json::json!({"name":g.name,"questions":g.questions.iter().map(|slug| serde_json::json!({"titleSlug":slug})).collect::<Vec<_>>()})).collect();
        let mut payload =
            serde_json::json!({"data":{"studyPlanV2Detail":{"planSubGroups":groups}}});
        assert_eq!(super::parse_study_plan(&payload).unwrap().groups.len(), 17);
        payload["data"]["studyPlanV2Detail"]["planSubGroups"][0]["questions"]
            .as_array_mut()
            .unwrap()
            .pop();
        assert!(
            super::parse_study_plan(&payload)
                .unwrap_err()
                .to_string()
                .contains("99 / 100")
        );
        assert!(
            super::parse_study_plan(&serde_json::json!({"errors":[{"message":"验证失败"}]}))
                .is_err()
        );
    }
    use super::*;
    #[test]
    fn decimal_test_task_ids_are_polled_and_platform_errors_are_preserved() {
        let id = "runcode_1627219627.5662382_EI7iasnhLm";
        assert_eq!(judge_id(&json!({"interpret_id":id}), false).unwrap(), id);
        let request = check_request(id).unwrap();
        assert_eq!(request.path, format!("/submissions/detail/{id}/check/"));
        assert!(request.body.is_none());
        for invalid in [
            "",
            ".",
            "..",
            "../../secret",
            "id/next",
            "id\\next",
            "id?x=1",
            "id#fragment",
            "%2e%2e",
            "id\n",
        ] {
            assert!(check_request(invalid).is_err(), "{invalid:?}");
        }
        assert!(check_request(&"a".repeat(257)).is_err());
        let error = judge_id(&json!({"error":"Too many requests"}), false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("Too many requests"));
        assert!(!error.contains("缺少"));
        assert!(
            judge_id(&json!({"detail":"Authentication required"}), false)
                .unwrap_err()
                .to_string()
                .contains("Authentication required")
        );
        assert!(
            judge_id(&json!({"success":false}), false)
                .unwrap_err()
                .to_string()
                .contains("缺少 interpret_id")
        );
        assert!(judge_id(&json!({"interpret_id":id}), true).is_err());
    }
    #[test]
    fn tests_and_submissions_use_different_requests_and_judge_results() {
        let mut problem = stub(Site::China, "1", "two-sum", "两数之和").unwrap();
        let source = "package solution\nimport \"fmt\"\nvar example = `\npackage text\n`\nfunc twoSum(nums []int, target int) []int {fmt.Println(example); return nil}";
        assert!(submit_request(&problem, source).is_err());
        let remote = problem.online.as_mut().unwrap();
        remote.loaded = true;
        remote.go_supported = true;
        remote.question_id = "1".into();
        let test = run_request(&problem, source, "[2,7]\n9").unwrap();
        let submit = submit_request(&problem, source).unwrap();
        assert!(test.path.ends_with("/interpret_solution/"));
        assert!(submit.path.ends_with("/submit/"));
        let payload = submit.body.unwrap();
        assert_eq!(payload["lang"], "golang");
        assert_eq!(payload["question_id"], "1");
        assert!(payload.get("data_input").is_none());
        assert!(
            payload["typed_code"]
                .as_str()
                .unwrap()
                .contains("package text")
        );
        assert!(
            !payload["typed_code"]
                .as_str()
                .unwrap()
                .contains("package solution")
        );
        assert_eq!(
            judge_id(&json!({"submission_id":123}), true).unwrap(),
            "123"
        );
        assert!(judge_id(&json!({"interpret_id":"test-1"}), true).is_err());
        assert!(!judge_finished(&json!({"state":"PENDING"})));
        assert!(judge_finished(&json!({"state":"FAILURE"})));
        assert!(!submission_accepted(
            &json!({"state":"SUCCESS","status_code":11})
        ));
        assert!(submission_accepted(
            &json!({"state":"SUCCESS","status_code":10})
        ));
        assert!(submission_output(&json!({"state":"SUCCESS","status_code":10,"status_msg":"Accepted","submission_id":123})).contains("Accepted"));
    }
    #[test]
    fn statuses_are_server_values_and_non_numeric_ids_are_stable() {
        let value = json!({"data":{"problemsetQuestionList":{"total":3,"questions":[{"frontendQuestionId":"1","title":"Two Sum","titleCn":"两数之和","titleSlug":"two-sum","status":"ac","difficulty":"Easy","topicTags":[{"name":"Array","nameTranslated":"数组"}]},{"frontendQuestionId":"LCP 01","titleSlug":"guess-numbers","title":"猜数字","status":null},{"frontendQuestionId":"2","titleSlug":"add-two-numbers","title":"两数相加","status":"notac"}]}}});
        let page = parse_page(Site::China, &value).unwrap();
        assert_eq!(page.problems[0].tags, vec!["数组"]);
        assert_eq!(status_label(&page.problems[0]), "已通过");
        assert_eq!(status_label(&page.problems[1]), "未解答");
        assert_eq!(status_label(&page.problems[2]), "尝试过");
        assert!(page.problems[1].id > 1_000_000);
        assert_eq!(
            page.problems[1].id,
            parse_page(Site::China, &value).unwrap().problems[1].id
        );
    }
    #[test]
    fn origin_and_requests_are_restricted_without_reading_cookies() {
        assert!(Site::China.accepts("https://leetcode.cn/accounts/login/"));
        assert!(!Site::China.accepts("https://leetcode.cn.attacker.test/"));
        assert!(!Site::China.accepts("http://leetcode.cn/"));
        assert!(
            script(1, &account_request())
                .unwrap()
                .contains("credentials:'include'")
        );
        assert!(
            script(
                1,
                &Request {
                    path: "//attacker.test/".into(),
                    body: None
                }
            )
            .is_err()
        );
        assert!(check_request("../../secret").is_err());
        assert!(
            parse_account(&json!({"data":{"userStatus":{"isSignedIn":false}}}))
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn details_use_official_go_templates_and_keep_extra_node_definitions() {
        let original = stub(Site::China, "133", "clone-graph", "克隆图").unwrap();
        let body = json!({"data":{"question":{"questionId":"133","content":"<p>Clone graph</p>","translatedContent":"<p>克隆图</p>","status":"ac","exampleTestcases":"[[2],[1]]","codeSnippets":[{"langSlug":"golang","code":"/**\n * type Node struct {\n *     Val int\n *     Neighbors []*Node\n * }\n */\nfunc cloneGraph(node *Node) *Node { return nil }"}],"topicTags":[{"name":"Graph"}]}}});
        let detail = parse_detail(&original, &body).unwrap();
        assert!(detail.extra_types.contains("Neighbors []*Node"));
        assert!(detail.starter.starts_with("package solution"));
        assert_eq!(detail.description, "<p>克隆图</p>");
        assert!(
            run_request(&detail, &detail.starter, "[[2],[1]]")
                .unwrap()
                .body
                .unwrap()["typed_code"]
                .as_str()
                .unwrap()
                .starts_with("\n")
        );
    }
}
