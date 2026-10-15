//! Portable, offline backup of personal Markdown notes and imported explanations.

use crate::store::{Store, atomic_write, is_markdown};
use anyhow::{Context, Result, bail};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::OnceLock,
};
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

const FORMAT: &str = "leetdesk-notes";
const VERSION: u32 = 1;
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 128 * 1024 * 1024;
const MAX_FILES: usize = 10_000;

#[derive(Serialize, Deserialize)]
struct Manifest {
    format: String,
    version: u32,
}

#[derive(Default, Debug, PartialEq, Eq)]
pub struct ExportSummary {
    pub personal: usize,
    pub explanations: usize,
    pub images: usize,
    pub missing_images: usize,
}

#[derive(Default, Debug, PartialEq, Eq)]
pub struct ImportSummary {
    pub personal: usize,
    pub explanations: usize,
    pub merged: usize,
}

/// Export every personal `notes.md` and every imported explanation in one ZIP.
/// Images referenced with Markdown or HTML image syntax are copied into the ZIP.
pub fn export(store: &Store, destination: &Path) -> Result<ExportSummary> {
    if destination.exists() {
        bail!("目标文件已存在，请选择新的 ZIP 文件名");
    }
    let mut entries = BTreeMap::new();
    let mut summary = ExportSummary::default();

    for (id, dir) in numbered_directories(&store.root.join("workspaces"))? {
        let path = dir.join("notes.md");
        if path.is_file() && !fs::read_to_string(&path)?.trim().is_empty() {
            add_document(
                &path,
                &format!("workspaces/{id}/notes.md"),
                &mut entries,
                &mut summary,
            )?;
            summary.personal += 1;
        }
    }
    for (id, dir) in numbered_directories(&store.root.join("notes"))? {
        let mut paths = fs::read_dir(dir)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::io::Result<Vec<_>>>()?;
        paths.sort();
        for path in paths {
            if path.is_file() && is_markdown(&path) && !fs::read_to_string(&path)?.trim().is_empty()
            {
                let name = path
                    .file_name()
                    .context("讲解文件名无效")?
                    .to_string_lossy();
                add_document(
                    &path,
                    &format!("notes/{id}/{name}"),
                    &mut entries,
                    &mut summary,
                )?;
                summary.explanations += 1;
            }
        }
    }
    if summary.personal + summary.explanations == 0 {
        bail!("还没有可导出的 Markdown 笔记");
    }
    if entries.len() > MAX_FILES
        || entries
            .values()
            .map(|bytes: &Vec<u8>| bytes.len() as u64)
            .sum::<u64>()
            > MAX_TOTAL_BYTES
    {
        bail!("笔记包超过大小或文件数量限制");
    }

    let temporary = destination.with_extension("zip.saving");
    let result = (|| -> Result<()> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .with_context(|| format!("无法创建 {}", temporary.display()))?;
        let mut zip = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        zip.start_file("manifest.json", options)?;
        zip.write_all(&serde_json::to_vec(&Manifest {
            format: FORMAT.into(),
            version: VERSION,
        })?)?;
        for (name, bytes) in entries {
            zip.start_file(name, options)?;
            zip.write_all(&bytes)?;
        }
        zip.finish()?;
        fs::rename(&temporary, destination)
            .with_context(|| format!("无法保存 {}", destination.display()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result?;
    Ok(summary)
}

fn numbered_directories(root: &Path) -> Result<Vec<(u32, PathBuf)>> {
    if !root.exists() {
        return Ok(vec![]);
    }
    let mut directories = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir()
            && let Some(id) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse().ok())
            && id != 0
        {
            directories.push((id, entry.path()));
        }
    }
    directories.sort_by_key(|(id, _)| *id);
    Ok(directories)
}

fn add_document(
    path: &Path,
    archive_name: &str,
    entries: &mut BTreeMap<String, Vec<u8>>,
    summary: &mut ExportSummary,
) -> Result<()> {
    let markdown =
        fs::read_to_string(path).with_context(|| format!("无法读取笔记 {}", path.display()))?;
    if markdown.len() as u64 > MAX_FILE_BYTES {
        bail!("笔记太大：{}", path.display());
    }
    let original = fs::read_to_string(path.with_extension("origin"))
        .ok()
        .map(PathBuf::from)
        .filter(|original| original.is_file());
    let base = original
        .as_deref()
        .unwrap_or(path)
        .parent()
        .context("笔记路径无父目录")?;
    let mut replacements = Vec::new();
    for (start, end, url) in image_links(&markdown) {
        if let Ok(parsed) = url::Url::parse(url) {
            if parsed.scheme() == "file" {
                summary.missing_images += 1;
            }
            continue;
        }
        if url.starts_with('#') {
            continue;
        }
        let relative = Path::new(url);
        if relative.components().next().is_none()
            || !relative
                .components()
                .all(|part| matches!(part, Component::Normal(_) | Component::CurDir))
        {
            summary.missing_images += 1;
            continue;
        }
        let image = base.join(relative);
        let Ok(metadata) = fs::symlink_metadata(&image) else {
            summary.missing_images += 1;
            continue;
        };
        if !metadata.file_type().is_file() {
            summary.missing_images += 1;
            continue;
        }
        if metadata.len() > MAX_FILE_BYTES {
            bail!("图片太大：{}", image.display());
        }
        let name = image
            .file_name()
            .context("图片文件名无效")?
            .to_string_lossy();
        let safe_name: String = name
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                    ch
                } else {
                    '_'
                }
            })
            .collect();
        let asset_name = format!("{:05}-{safe_name}", summary.images + 1);
        let archive_dir = archive_name.rsplit_once('/').unwrap().0;
        let asset_path = format!("{archive_dir}/assets/{asset_name}");
        entries.insert(asset_path, fs::read(&image)?);
        replacements.push((start, end, format!("assets/{asset_name}")));
        summary.images += 1;
    }
    let mut portable = markdown;
    for (start, end, replacement) in replacements.into_iter().rev() {
        portable.replace_range(start..end, &replacement);
    }
    entries.insert(archive_name.into(), portable.into_bytes());
    Ok(())
}

fn image_links(markdown: &str) -> Vec<(usize, usize, &str)> {
    static MD_IMAGE: OnceLock<Regex> = OnceLock::new();
    static HTML_IMAGE: OnceLock<Regex> = OnceLock::new();
    let md = MD_IMAGE.get_or_init(|| {
        Regex::new(r#"!\[[^\]]*\]\(\s*(?:<(?P<bracket>[^>]+)>|(?P<plain>[^\s)]+))(?:\s+["'][^"']*["'])?\s*\)"#)
            .expect("valid image regex")
    });
    let html = HTML_IMAGE.get_or_init(|| {
        Regex::new(r#"(?i)<img\b[^>]*\bsrc\s*=\s*["'](?P<url>[^"']+)["']"#)
            .expect("valid HTML image regex")
    });
    let mut urls = Vec::new();
    for captures in md.captures_iter(markdown) {
        if let Some(url) = captures.name("bracket").or_else(|| captures.name("plain")) {
            urls.push((url.start(), url.end(), url.as_str()));
        }
    }
    for captures in html.captures_iter(markdown) {
        if let Some(url) = captures.name("url") {
            urls.push((url.start(), url.end(), url.as_str()));
        }
    }
    urls.sort_by_key(|(start, _, _)| *start);
    urls
}

enum EntryKind {
    Personal(u32),
    Explanation(u32, String),
    Asset(String),
}

fn entry_kind(name: &str) -> Option<EntryKind> {
    if name.contains('\\') || name.contains(':') || name.starts_with('/') {
        return None;
    }
    let parts: Vec<_> = name.split('/').collect();
    let id: u32 = parts.get(1)?.parse().ok()?;
    if id == 0 || id.to_string() != *parts.get(1)? {
        return None;
    }
    match parts.as_slice() {
        ["workspaces", _, "notes.md"] => Some(EntryKind::Personal(id)),
        ["notes", _, filename] if is_markdown(Path::new(filename)) && valid_name(filename) => {
            Some(EntryKind::Explanation(id, (*filename).into()))
        }
        ["workspaces" | "notes", _, "assets", filename] if valid_name(filename) => {
            Some(EntryKind::Asset(name.into()))
        }
        _ => None,
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains('\0')
}

fn checked_destination(root: &Path, relative: &str) -> Result<PathBuf> {
    let mut path = root.to_path_buf();
    for component in Path::new(relative).components() {
        if !matches!(component, Component::Normal(_)) {
            bail!("笔记包包含无效路径：{relative}");
        }
        path.push(component);
        if fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            bail!("笔记路径指向符号链接：{}", path.display());
        }
    }
    Ok(path)
}

/// Import an exported ZIP without replacing existing personal notes.
pub fn import(store: &Store, source: &Path) -> Result<ImportSummary> {
    if fs::metadata(source)?.len() > MAX_TOTAL_BYTES {
        bail!("笔记包太大");
    }
    let mut zip = ZipArchive::new(File::open(source)?)?;
    if zip.len() > MAX_FILES + 1 {
        bail!("笔记包中的文件过多");
    }
    let mut files = BTreeMap::new();
    let mut total = 0_u64;
    for index in 0..zip.len() {
        let mut file = zip.by_index(index)?;
        let name = file.name().to_string();
        if file.is_dir() || (name != "manifest.json" && entry_kind(&name).is_none()) {
            bail!("笔记包包含无效路径：{name}");
        }
        if file.size() > MAX_FILE_BYTES {
            bail!("笔记包文件太大：{name}");
        }
        total = total.saturating_add(file.size());
        if total > MAX_TOTAL_BYTES {
            bail!("笔记包解压后超过大小限制");
        }
        let mut bytes = Vec::new();
        file.by_ref()
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_FILE_BYTES || files.insert(name.clone(), bytes).is_some() {
            bail!("笔记包包含过大或重复的文件：{name}");
        }
    }
    let manifest: Manifest = serde_json::from_slice(
        files
            .remove("manifest.json")
            .context("这不是 LeetDesk 笔记包")?
            .as_slice(),
    )?;
    if manifest.format != FORMAT || manifest.version != VERSION {
        bail!("不支持的笔记包格式或版本");
    }
    if !files.keys().any(|name| {
        matches!(
            entry_kind(name),
            Some(EntryKind::Personal(_) | EntryKind::Explanation(_, _))
        )
    }) {
        bail!("笔记包中没有 Markdown 文档");
    }
    for (name, bytes) in &files {
        checked_destination(&store.root, name)?;
        if !matches!(entry_kind(name), Some(EntryKind::Asset(_))) {
            std::str::from_utf8(bytes).with_context(|| format!("笔记不是 UTF-8：{name}"))?;
        }
    }
    // Keep images from both machines when their exported asset names collide.
    let mut asset_targets = BTreeMap::new();
    let mut asset_replacements = BTreeMap::new();
    for (name, bytes) in &files {
        if let Some(EntryKind::Asset(relative)) = entry_kind(name) {
            let target = checked_destination(&store.root, &relative)?;
            let selected = if target.exists() && fs::read(&target)? != *bytes {
                matching_or_next_import_name(&target, bytes)?
            } else {
                target.clone()
            };
            if selected != target {
                let (directory, filename) = name.rsplit_once('/').unwrap();
                let document_directory = directory.strip_suffix("/assets").unwrap();
                let new_filename = selected.file_name().unwrap().to_string_lossy();
                asset_replacements.insert(
                    (document_directory.to_string(), format!("assets/{filename}")),
                    format!("assets/{new_filename}"),
                );
            }
            asset_targets.insert(name.clone(), selected);
        }
    }

    let mut summary = ImportSummary::default();
    for (name, bytes) in &files {
        if let Some(target) = asset_targets.get(name) {
            fs::create_dir_all(target.parent().unwrap())?;
            if !target.exists() {
                atomic_write(target, bytes)?;
            }
        }
    }
    for (name, bytes) in files {
        match entry_kind(&name) {
            Some(EntryKind::Personal(id)) => {
                let target = store.workspace(id).join("notes.md");
                fs::create_dir_all(target.parent().unwrap())?;
                let incoming = rewrite_asset_links(
                    String::from_utf8(bytes)?,
                    name.rsplit_once('/').unwrap().0,
                    &asset_replacements,
                );
                let existing = fs::read_to_string(&target).unwrap_or_default();
                if existing.is_empty() {
                    atomic_write(&target, incoming.as_bytes())?;
                    summary.personal += 1;
                } else if !incoming.is_empty() && !existing.contains(&incoming) {
                    let backup = next_import_name(target.parent().unwrap(), "notes-imported", "md");
                    atomic_write(&backup, incoming.as_bytes())?;
                    let merged = format!(
                        "{}\n\n---\n\n## 从笔记包导入\n\n{}",
                        existing.trim_end(),
                        incoming
                    );
                    atomic_write(&target, merged.as_bytes())?;
                    summary.personal += 1;
                    summary.merged += 1;
                }
            }
            Some(EntryKind::Explanation(id, filename)) => {
                let dir = store.root.join("notes").join(id.to_string());
                fs::create_dir_all(&dir)?;
                let mut target = dir.join(filename);
                let bytes = rewrite_asset_links(
                    String::from_utf8(bytes)?,
                    name.rsplit_once('/').unwrap().0,
                    &asset_replacements,
                )
                .into_bytes();
                if target.exists() && fs::read(&target)? != bytes {
                    let stem = target.file_stem().unwrap().to_string_lossy();
                    let extension = target.extension().unwrap().to_string_lossy();
                    target = next_import_name(&dir, &format!("{stem}-imported"), &extension);
                }
                if !target.exists() {
                    atomic_write(&target, &bytes)?;
                    summary.explanations += 1;
                }
                let origin = target.canonicalize()?;
                atomic_write(
                    &target.with_extension("origin"),
                    origin.to_string_lossy().as_bytes(),
                )?;
            }
            Some(EntryKind::Asset(_)) => {}
            None => unreachable!("archive paths validated before writing"),
        }
    }
    Ok(summary)
}

fn matching_or_next_import_name(target: &Path, incoming: &[u8]) -> Result<PathBuf> {
    let stem = target
        .file_stem()
        .context("图片文件名无效")?
        .to_string_lossy();
    let extension = target.extension().map(|value| value.to_string_lossy());
    for number in 1.. {
        let name = match &extension {
            Some(extension) => format!("{stem}-imported-{number}.{extension}"),
            None => format!("{stem}-imported-{number}"),
        };
        let candidate = target.with_file_name(name);
        if !candidate.exists() || fs::read(&candidate)? == incoming {
            return Ok(candidate);
        }
    }
    unreachable!()
}

fn rewrite_asset_links(
    markdown: String,
    directory: &str,
    replacements: &BTreeMap<(String, String), String>,
) -> String {
    let mut edits = Vec::new();
    for (start, end, url) in image_links(&markdown) {
        let normalized = url.strip_prefix("./").unwrap_or(url);
        if let Some(replacement) =
            replacements.get(&(directory.to_string(), normalized.to_string()))
        {
            let prefix = if url.starts_with("./") { "./" } else { "" };
            edits.push((start, end, format!("{prefix}{replacement}")));
        }
    }
    let mut rewritten = markdown;
    for (start, end, replacement) in edits.into_iter().rev() {
        rewritten.replace_range(start..end, &replacement);
    }
    rewritten
}

fn next_import_name(directory: &Path, stem: &str, extension: &str) -> PathBuf {
    let mut number = 1;
    loop {
        let path = directory.join(format!("{stem}-{number}.{extension}"));
        if !path.exists() {
            return path;
        }
        number += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_personal_explanations_and_images() {
        let source_dir = tempfile::tempdir().unwrap();
        let source = Store::open(source_dir.path().join("source")).unwrap();
        let workspace = source.workspace(1);
        fs::create_dir_all(workspace.join("images")).unwrap();
        fs::write(workspace.join("images/pic.png"), b"image 1").unwrap();
        fs::write(
            workspace.join("notes.md"),
            "# 笔记\n\n![图](./images/pic.png)\n\n![远程](https://example.com/image.png)\n",
        )
        .unwrap();
        let external = source_dir.path().join("external");
        fs::create_dir_all(external.join("figures")).unwrap();
        fs::write(external.join("figures/demo.svg"), b"<svg/>").unwrap();
        fs::write(
            external.join("method.md"),
            "# 做法\n\n<img src=\"figures/demo.svg\">\n",
        )
        .unwrap();
        let explanation = source.root.join("notes/1/method.md");
        fs::create_dir_all(explanation.parent().unwrap()).unwrap();
        fs::copy(external.join("method.md"), &explanation).unwrap();
        fs::write(
            explanation.with_extension("origin"),
            external.join("method.md").to_string_lossy().as_bytes(),
        )
        .unwrap();

        let bundle = source_dir.path().join("notes.zip");
        assert_eq!(
            export(&source, &bundle).unwrap(),
            ExportSummary {
                personal: 1,
                explanations: 1,
                images: 2,
                missing_images: 0,
            }
        );
        let target = Store::open(source_dir.path().join("target")).unwrap();
        assert_eq!(
            import(&target, &bundle).unwrap(),
            ImportSummary {
                personal: 1,
                explanations: 1,
                merged: 0,
            }
        );
        let personal = fs::read_to_string(target.workspace(1).join("notes.md")).unwrap();
        assert!(personal.contains("# 笔记") && personal.contains("assets/00001-pic.png"));
        assert!(personal.contains("https://example.com/image.png"));
        assert_eq!(
            fs::read(target.workspace(1).join("assets/00001-pic.png")).unwrap(),
            b"image 1"
        );
        let imported = fs::read_to_string(target.root.join("notes/1/method.md")).unwrap();
        assert!(imported.contains("assets/00002-demo.svg"));
        assert_eq!(
            fs::read(target.root.join("notes/1/assets/00002-demo.svg")).unwrap(),
            b"<svg/>"
        );
        assert_eq!(import(&target, &bundle).unwrap(), ImportSummary::default());
    }

    #[test]
    fn conflicting_personal_notes_keep_both_versions() {
        let temp = tempfile::tempdir().unwrap();
        let source = Store::open(temp.path().join("source")).unwrap();
        fs::create_dir_all(source.workspace(7)).unwrap();
        fs::write(source.workspace(7).join("notes.md"), "# 家里的笔记").unwrap();
        let bundle = temp.path().join("notes.zip");
        export(&source, &bundle).unwrap();
        let target = Store::open(temp.path().join("target")).unwrap();
        fs::create_dir_all(target.workspace(7)).unwrap();
        fs::write(target.workspace(7).join("notes.md"), "# 单位的笔记").unwrap();
        let summary = import(&target, &bundle).unwrap();
        assert_eq!(summary.merged, 1);
        let merged = fs::read_to_string(target.workspace(7).join("notes.md")).unwrap();
        assert!(merged.contains("# 单位的笔记") && merged.contains("# 家里的笔记"));
        assert_eq!(
            fs::read_to_string(target.workspace(7).join("notes-imported-1.md")).unwrap(),
            "# 家里的笔记"
        );
        assert_eq!(import(&target, &bundle).unwrap(), ImportSummary::default());
    }

    #[test]
    fn conflicting_images_keep_both_versions_and_rewrite_links() {
        let temp = tempfile::tempdir().unwrap();
        let source = Store::open(temp.path().join("source")).unwrap();
        let source_workspace = source.workspace(7);
        fs::create_dir_all(source_workspace.join("images")).unwrap();
        fs::write(source_workspace.join("images/pic.png"), b"home image").unwrap();
        fs::write(
            source_workspace.join("notes.md"),
            "# 家里\n\n![图](images/pic.png)",
        )
        .unwrap();
        let bundle = temp.path().join("notes.zip");
        export(&source, &bundle).unwrap();

        let target = Store::open(temp.path().join("target")).unwrap();
        let target_workspace = target.workspace(7);
        fs::create_dir_all(target_workspace.join("assets")).unwrap();
        fs::write(target_workspace.join("notes.md"), "# 单位").unwrap();
        fs::write(
            target_workspace.join("assets/00001-pic.png"),
            b"office image",
        )
        .unwrap();

        assert_eq!(import(&target, &bundle).unwrap().merged, 1);
        let note = fs::read_to_string(target_workspace.join("notes.md")).unwrap();
        assert!(note.contains("assets/00001-pic-imported-1.png"));
        assert_eq!(
            fs::read(target_workspace.join("assets/00001-pic.png")).unwrap(),
            b"office image"
        );
        assert_eq!(
            fs::read(target_workspace.join("assets/00001-pic-imported-1.png")).unwrap(),
            b"home image"
        );
        assert_eq!(import(&target, &bundle).unwrap(), ImportSummary::default());
    }

    #[test]
    fn malicious_archive_path_is_rejected_before_writing() {
        let temp = tempfile::tempdir().unwrap();
        let bundle = temp.path().join("bad.zip");
        let mut zip = ZipWriter::new(File::create(&bundle).unwrap());
        zip.start_file("manifest.json", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(br#"{"format":"leetdesk-notes","version":1}"#)
            .unwrap();
        zip.start_file("notes/1/../../bad.md", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"malicious").unwrap();
        zip.finish().unwrap();
        let target = Store::open(temp.path().join("target")).unwrap();
        assert!(import(&target, &bundle).is_err());
        assert!(!target.root.join("bad.md").exists());
    }
}
