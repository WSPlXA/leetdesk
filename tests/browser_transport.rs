//! Executes the actual WebView request script with a mocked browser Fetch API.
//! No request reaches LeetCode and no real browser cookie is accessed.
use leetdesk::online::{Request, script};
use serde_json::{Value, json};
use std::process::Command;

#[test]
#[ignore = "requires Node.js"]
fn browser_fetch_preserves_task_ids_and_reports_http_and_validation_errors() {
    let request = Request {
        path: "/problems/two-sum/interpret_solution/".into(),
        body: Some(json!({"lang":"golang","typed_code":"func twoSum() {}"})),
    };
    let generated = serde_json::to_string(&script(7, &request).unwrap()).unwrap();
    let cases = [
        (
            200,
            json!({"interpret_id":"runcode_1627219627.5662382_EI7iasnhLm"}).to_string(),
            false,
            "",
        ),
        (
            401,
            json!({"detail":"Authentication required"}).to_string(),
            false,
            "登录已失效",
        ),
        (403, "<html>Verify</html>".into(), true, "官方验证尚未完成"),
        (
            403,
            json!({"error":"CSRF verification failed"}).to_string(),
            false,
            "CSRF verification failed",
        ),
        (
            429,
            json!({"error":"Too many requests"}).to_string(),
            false,
            "请求过于频繁",
        ),
        (
            502,
            "<html>Bad Gateway</html>".into(),
            false,
            "服务暂时不可用",
        ),
        (
            200,
            "<html>Login</html>".into(),
            false,
            "网站返回了登录或验证页面",
        ),
    ];
    for (status, text, challenge, expected) in cases {
        let response = json!({"status":status,"text":text,"challenge":challenge});
        let driver = format!(
            r#"
const assert=require('node:assert/strict');
const response={response};
global.document={{cookie:'csrftoken=fixture-token; LEETCODE_SESSION=fixture-session'}};
global.location={{origin:'https://leetcode.cn',href:'https://leetcode.cn/accounts/login/'}};
global.fetch=async(path,options)=>{{
    assert.equal(path,'/problems/two-sum/interpret_solution/');
    assert.equal(options.credentials,'include');
    assert.equal(options.referrer,'https://leetcode.cn/problems/two-sum/');
    assert.equal(options.headers['X-CSRFToken'],'fixture-token');
    assert.equal(options.method,'POST');
    return {{status:response.status,ok:response.status>=200&&response.status<300,
        headers:{{get:()=>response.challenge?'challenge':null}},text:async()=>response.text}};
}};
global.window={{ipc:{{postMessage:value=>process.stdout.write(value)}}}};
eval({generated});
"#
        );
        let output = Command::new("node")
            .args(["-e", &driver])
            .output()
            .expect("Node.js is required");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let reply: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(reply["id"], 7);
        if expected.is_empty() {
            assert_eq!(reply["ok"], true);
            assert_eq!(
                reply["data"]["interpret_id"],
                "runcode_1627219627.5662382_EI7iasnhLm"
            );
        } else {
            assert_eq!(reply["ok"], false);
            let message = reply["error"].as_str().unwrap();
            assert!(message.contains(expected), "{message}");
            assert!(message.contains(&format!("HTTP {status}")), "{message}");
        }
        assert!(!reply.to_string().contains("fixture-session"));
        assert!(!reply.to_string().contains("fixture-token"));
    }
}

#[test]
#[ignore = "requires Node.js"]
fn opaque_or_loading_pages_do_not_send_api_requests() {
    let generated =
        serde_json::to_string(&script(8, &leetdesk::online::account_request()).unwrap()).unwrap();
    for (origin, ready, denied) in [
        ("null", "complete", false),
        ("https://leetcode.com", "loading", false),
        ("https://leetcode.com", "complete", true),
    ] {
        let driver = format!(
            r#"
global.location={{origin:{origin:?}}};
global.document={{readyState:{ready:?},get cookie(){{if({denied})throw Error('Access is denied');return '';}}}};
global.fetch=async()=>{{throw Error('Unexpected fetch')}};
global.window={{ipc:{{postMessage:value=>process.stdout.write(value)}}}};
eval({generated});
"#
        );
        let output = Command::new("node").args(["-e", &driver]).output().unwrap();
        assert!(output.status.success());
        let reply: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(reply["ok"], false);
        assert!(
            reply["error"]
                .as_str()
                .unwrap()
                .contains("官方页面尚未就绪")
        );
    }
}
