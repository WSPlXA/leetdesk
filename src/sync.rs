//! Manual client for the single-owner backup service.

use crate::{
    store::Store,
    vault::{self, MAX_ARCHIVE_BYTES, Summary},
};
use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use std::{
    fs,
    io::{Read, Write},
    time::Duration,
};

fn config() -> Result<(String, String)> {
    let url = std::env::var("LEETDESK_SYNC_URL").context("请设置 LEETDESK_SYNC_URL")?;
    let token = std::env::var("LEETDESK_SYNC_TOKEN").context("请设置 LEETDESK_SYNC_TOKEN")?;
    if token.len() < 32 {
        bail!("LEETDESK_SYNC_TOKEN 至少需要 32 字符");
    }
    let parsed = url::Url::parse(&url).context("同步地址无效")?;
    if parsed.scheme() != "https"
        && !(parsed.scheme() == "http"
            && matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")))
    {
        bail!("远程同步地址必须使用 HTTPS");
    }
    if parsed.username() != ""
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        bail!("同步地址不能包含账户、查询或片段");
    }
    Ok((url.trim_end_matches('/').into(), token))
}

fn client() -> Result<Client> {
    Ok(Client::builder()
        .timeout(Duration::from_secs(120))
        .build()?)
}

pub fn upload(store: &Store) -> Result<Summary> {
    let (url, token) = config()?;
    let temp = tempfile::tempdir()?;
    let bundle = temp.path().join("backup.zip");
    let summary = vault::export(store, &bundle)?;
    let bytes = fs::read(bundle)?;
    let response = client()?
        .post(format!("{url}/v1/snapshots"))
        .bearer_auth(token)
        .header("Content-Type", "application/zip")
        .body(bytes)
        .send()?;
    if !response.status().is_success() {
        bail!(
            "上传失败（HTTP {}）：{}",
            response.status(),
            response.text()?.chars().take(200).collect::<String>()
        );
    }
    Ok(summary)
}

pub fn download(store: &Store) -> Result<Summary> {
    let (url, token) = config()?;
    let mut response = client()?
        .get(format!("{url}/v1/snapshots/latest"))
        .bearer_auth(token)
        .send()?;
    if !response.status().is_success() {
        bail!(
            "下载失败（HTTP {}）：{}",
            response.status(),
            response.text()?.chars().take(200).collect::<String>()
        );
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_ARCHIVE_BYTES)
    {
        bail!("备份包过大");
    }
    let mut temp = tempfile::NamedTempFile::new()?;
    let copied = std::io::copy(
        &mut response.by_ref().take(MAX_ARCHIVE_BYTES + 1),
        &mut temp,
    )?;
    if copied > MAX_ARCHIVE_BYTES {
        bail!("备份包过大");
    }
    temp.flush()?;
    vault::import(store, temp.path())
}
