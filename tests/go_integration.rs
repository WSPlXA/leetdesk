//! Real tool integration checks, not mocked language intelligence.
//! Run: cargo test --test go_integration -- --ignored --test-threads=1
use leetdesk::{
    catalog,
    language::{self, GoLanguage},
    runner,
    store::Store,
};
use serde_json::json;
use std::{
    fs,
    sync::{Arc, atomic::AtomicBool},
    thread,
    time::{Duration, Instant},
};

#[test]
#[ignore = "requires gopls"]
fn goimports_adds_missing_imports_from_an_unsaved_draft() {
    let temp = tempfile::tempdir().unwrap();
    let source = "package solution\n\nfunc answer() { fmt.Println(1) }\n";
    let organized = runner::organize_go_imports(source, temp.path()).unwrap();
    assert!(organized.contains("import \"fmt\""), "{organized}");
    assert!(organized.contains("fmt.Println(1)"));
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
}

fn wait_diagnostics(service: &GoLanguage, expected_error: bool) {
    let start = Instant::now();
    loop {
        let snapshot = service.diagnostics();
        if snapshot.generation > 0
            && snapshot.version == Some(service.version())
            && (snapshot.items.is_empty() != expected_error)
        {
            return;
        }
        if start.elapsed() > Duration::from_secs(45) {
            panic!(
                "diagnostics did not converge: {snapshot:?}, error={:?}",
                service.error()
            );
        }
        thread::sleep(Duration::from_millis(100));
    }
}

#[test]
#[ignore = "requires Go and gopls"]
fn gopls_diagnostics_completion_hover_and_definition_work_with_node_types() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("data")).unwrap();
    let problems = catalog::builtin().unwrap();
    let problem = problems.iter().find(|p| p.id == 206).unwrap();
    let path = store.prepare(problem).unwrap();
    let good = "package solution\n\nfunc reverseList(head *ListNode) *ListNode { return head }\n";
    fs::write(path.join("solution.go"), good).unwrap();
    let service = GoLanguage::start(&path, good).unwrap();
    wait_diagnostics(&service, false);
    let generation = service.diagnostics().generation;
    assert!(service.diagnostics_since(generation).is_none());
    assert_eq!(service.diagnostics_since(0).unwrap().generation, generation);
    assert!(
        service.diagnostics().items.iter().all(|d| {
            !d.message.contains("package comment") && !d.message.contains("is unused")
        }),
        "刷题入口不应产生包注释和未调用函数的提示"
    );
    let hover = service
        .request(
            "textDocument/hover",
            service.at(good, good.find("ListNode").unwrap() + 2),
        )
        .unwrap();
    assert!(hover.to_string().contains("ListNode"));
    let definition = service
        .request(
            "textDocument/definition",
            service.at(good, good.find("ListNode").unwrap() + 2),
        )
        .unwrap();
    assert!(definition.to_string().contains("types.go"));
    let bad =
        "package solution\n\nfunc reverseList(head *ListNode) *ListNode { return missingName }\n";
    service.sync(bad).unwrap();
    wait_diagnostics(&service, true);
    assert!(
        service
            .diagnostics()
            .items
            .iter()
            .any(|d| d.message.contains("missingName"))
    );
    service.sync(good).unwrap();
    wait_diagnostics(&service, false);
    let completion_source =
        "package solution\n\nfunc reverseList(head *ListNode) *ListNode { head.N; return head }\n";
    service.sync(completion_source).unwrap();
    let response = service
        .request(
            "textDocument/completion",
            service.at(
                completion_source,
                completion_source.find("head.N").unwrap() + 6,
            ),
        )
        .unwrap();
    assert!(response.to_string().contains("Next"), "{response}");
    let standard = "package solution\n\nimport \"fmt\"\n\nfunc reverseList(head *ListNode) *ListNode { fmt.Pr; return head }\n";
    service.sync(standard).unwrap();
    // Loading a standard-library import may require the first Go cache build.
    wait_diagnostics(&service, true);
    let response = service
        .request(
            "textDocument/completion",
            service.at(standard, standard.find("fmt.Pr").unwrap() + 6),
        )
        .unwrap();
    assert!(response.to_string().contains("Println"), "{response}");
    let unimported = "package solution\n\nfunc reverseList(head *ListNode) *ListNode { fmt.Println(head); return head }\n";
    service.sync(unimported).unwrap();
    let action = service.request("textDocument/codeAction",json!({"textDocument":{"uri":service.uri()},"range":{"start":{"line":0,"character":0},"end":language::byte_position(unimported,unimported.len())},"context":{"diagnostics":[],"only":["source.organizeImports"]}})).unwrap();
    assert!(action.to_string().contains("fmt"), "{action}");
    // TreeNode is in the same real package and has no undefined-type diagnostics.
    let tree = "package solution\nfunc maxDepth(root *TreeNode) int { if root == nil { return 0 }; return max(maxDepth(root.Left), maxDepth(root.Right)) + 1 }\n";
    service.sync(tree).unwrap();
    wait_diagnostics(&service, false);
}

#[test]
#[ignore = "requires Go"]
fn go_runner_executes_arrays_strings_lists_and_trees_and_detects_wrong_answers() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("data")).unwrap();
    let problems = catalog::builtin().unwrap();
    let solutions = [
        (
            1,
            "func twoSum(nums []int, target int) []int { seen := map[int]int{}; for i, n := range nums { if j, ok := seen[target-n]; ok { return []int{i,j} }; seen[n]=i }; return nil }",
        ),
        (
            20,
            "func isValid(s string) bool { stack := []byte{}; pairs := map[byte]byte{')':'(', ']':'[', '}':'{'}; for i:=0; i<len(s); i++ { if left,ok := pairs[s[i]]; ok { if len(stack)==0 || stack[len(stack)-1]!=left { return false }; stack=stack[:len(stack)-1] } else { stack=append(stack,s[i]) } }; return len(stack)==0 }",
        ),
        (
            206,
            "func reverseList(head *ListNode) *ListNode { var prev *ListNode; for head!=nil { next:=head.Next; head.Next=prev; prev=head; head=next }; return prev }",
        ),
        (
            104,
            "func maxDepth(root *TreeNode) int { if root==nil { return 0 }; return max(maxDepth(root.Left), maxDepth(root.Right)) + 1 }",
        ),
        (
            704,
            "func search(nums []int, target int) int { lo,hi:=0,len(nums)-1; for lo<=hi { mid:=lo+(hi-lo)/2; if nums[mid]==target { return mid }; if nums[mid]<target { lo=mid+1 } else { hi=mid-1 } }; return -1 }",
        ),
        (
            53,
            "func maxSubArray(nums []int) int { ending,best:=nums[0],nums[0]; for _,n:=range nums[1:] { ending=max(n,ending+n); best=max(best,ending) }; return best }",
        ),
    ];
    for (id, solution) in solutions {
        let problem = problems.iter().find(|p| p.id == id).unwrap();
        let path = store.prepare(problem).unwrap();
        let result = runner::run(
            problem,
            &format!("package solution\n{solution}\n"),
            &problem.cases,
            &path,
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert_eq!(
            result.passed, result.total,
            "problem {id}: {}",
            result.output
        );
        assert_eq!(result.cases.len(), problem.cases.len());
    }
    let problem = &problems[0];
    let path = store.prepare(problem).unwrap();
    let result = runner::run(
        problem,
        &problem.starter,
        &problem.cases,
        &path,
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(result.passed, 0);
    assert!(result.output.contains("预期"));
    let broken = runner::run(
        problem,
        "package solution\nfunc broken( {",
        &problem.cases,
        &path,
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert!(broken.output.contains("编译失败"));
}

#[test]
#[ignore = "requires Go"]
fn go_runner_bounds_infinite_loops_and_handles_panics() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("data")).unwrap();
    let problem = catalog::builtin().unwrap().remove(0);
    let path = store.prepare(&problem).unwrap();
    let source =
        "package solution\nfunc twoSum(nums []int, target int) []int { panic(\"test panic\") }";
    let result = runner::run(
        &problem,
        source,
        &problem.cases,
        &path,
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(result.passed, 0);
    assert!(result.output.contains("test panic"));
    let source = "package solution\nfunc twoSum(nums []int, target int) []int { for {} }";
    let started = Instant::now();
    let result = runner::run(
        &problem,
        source,
        &problem.cases,
        &path,
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(result.passed, 0);
    assert!(started.elapsed() < Duration::from_secs(30));
    assert!(result.output.contains("执行未完成"));
}
