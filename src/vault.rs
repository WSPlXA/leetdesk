//! Portable backup of notes, editable solutions, and immutable submission attempts.

use crate::{
    note_bundle,
    store::{Store, atomic_write},
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

pub const MAX_ARCHIVE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_FILES: usize = 20_000;

#[derive(Serialize, Deserialize)]
struct Manifest {
    format: String,
    version: u32,
}

#[derive(Default, Debug, PartialEq, Eq)]
pub struct Summary {
    pub notes: usize,
    pub solutions: usize,
    pub submissions: usize,
    pub conflicts: usize,
}

fn numbered_dirs(root: &Path) -> Result<Vec<(u32, PathBuf)>> {
    if !root.exists() {
        return Ok(vec![]);
    }
    let mut result = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        if let Some(id) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
            && id > 0
            && id.to_string() == entry.file_name().to_string_lossy()
        {
            result.push((id, entry.path()));
        }
    }
    result.sort_by_key(|(id, _)| *id);
    Ok(result)
}

fn add_file(entries: &mut BTreeMap<String, Vec<u8>>, path: &Path, name: String) -> Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() || meta.len() > MAX_FILE_BYTES {
        bail!("备份文件类型或大小无效：{}", path.display());
    }
    entries.insert(name, fs::read(path)?);
    Ok(())
}

pub fn export(store: &Store, destination: &Path) -> Result<Summary> {
    if destination.exists() {
        bail!("备份目标已存在");
    }
    let mut entries = BTreeMap::new();
    let mut summary = Summary::default();
    for (id, dir) in numbered_dirs(&store.root.join("workspaces"))? {
        for filename in ["solution.go", "Solution.java"] {
            let path = dir.join(filename);
            if path.is_file() {
                add_file(&mut entries, &path, format!("workspaces/{id}/{filename}"))?;
                summary.solutions += 1;
            }
        }
        let submissions = dir.join("submissions");
        if submissions.is_dir() {
            for entry in fs::read_dir(submissions)? {
                let path = entry?.path();
                let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                if valid_submission_name(name) {
                    add_file(
                        &mut entries,
                        &path,
                        format!("workspaces/{id}/submissions/{name}"),
                    )?;
                    summary.submissions += 1;
                }
            }
        }
    }
    let has_notes = numbered_dirs(&store.root.join("workspaces"))?
        .into_iter()
        .any(|(_, dir)| {
            fs::read_to_string(dir.join("notes.md")).is_ok_and(|s| !s.trim().is_empty())
        })
        || numbered_dirs(&store.root.join("notes"))?
            .into_iter()
            .any(|(_, dir)| {
                fs::read_dir(dir).is_ok_and(|mut files| {
                    files.any(|f| {
                        f.is_ok_and(|f| {
                            let path = f.path();
                            crate::store::is_markdown(&path)
                                && fs::read_to_string(path).is_ok_and(|s| !s.trim().is_empty())
                        })
                    })
                })
            });
    if has_notes {
        let temp = tempfile::tempdir()?;
        let notes = temp.path().join("notes.zip");
        let notes_summary = note_bundle::export(store, &notes)?;
        summary.notes = notes_summary.personal + notes_summary.explanations;
        add_file(&mut entries, &notes, "notes.zip".into())?;
    }
    if entries.len() > MAX_FILES
        || entries.values().map(|v| v.len() as u64).sum::<u64>() > MAX_ARCHIVE_BYTES
    {
        bail!("备份包超过大小或文件数量限制");
    }
    let parent = destination.parent().context("备份路径无父目录")?;
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    {
        let mut zip = ZipWriter::new(temp.as_file_mut());
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        zip.start_file("manifest.json", options)?;
        zip.write_all(&serde_json::to_vec(&Manifest {
            format: "leetdesk-vault".into(),
            version: 1,
        })?)?;
        for (name, bytes) in entries {
            zip.start_file(name, options)?;
            zip.write_all(&bytes)?;
        }
        zip.finish()?;
    }
    if temp.as_file().metadata()?.len() > MAX_ARCHIVE_BYTES {
        bail!("压缩后备份包过大");
    }
    temp.persist_noclobber(destination)?;
    Ok(summary)
}

enum Entry {
    Notes,
    Solution(u32, String),
    Submission(u32, String),
}

fn valid_submission_name(name: &str) -> bool {
    let Some((stem, ext)) = name.rsplit_once('.') else {
        return false;
    };
    (ext == "go" || ext == "java")
        && !stem.is_empty()
        && stem.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn entry(name: &str) -> Option<Entry> {
    if name == "notes.zip" {
        return Some(Entry::Notes);
    }
    if name.contains('\\') || name.contains(':') {
        return None;
    }
    let parts: Vec<_> = name.split('/').collect();
    if parts.first() != Some(&"workspaces") {
        return None;
    }
    let id: u32 = parts.get(1)?.parse().ok()?;
    if id == 0 || id.to_string() != *parts.get(1)? {
        return None;
    }
    match parts.as_slice() {
        [_, _, "solution.go" | "Solution.java"] => Some(Entry::Solution(id, parts[2].into())),
        [_, _, "submissions", filename] if valid_submission_name(filename) => {
            Some(Entry::Submission(id, (*filename).into()))
        }
        _ => None,
    }
}

pub fn import(store: &Store, source: &Path) -> Result<Summary> {
    if fs::metadata(source)?.len() > MAX_ARCHIVE_BYTES {
        bail!("备份包过大");
    }
    let mut archive = ZipArchive::new(File::open(source)?)?;
    if archive.len() > MAX_FILES + 2 {
        bail!("备份文件数量过多");
    }
    let mut entries = BTreeMap::new();
    let mut total = 0_u64;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let name = file.name().to_string();
        if file.is_dir() || (name != "manifest.json" && entry(&name).is_none()) {
            bail!("无效的备份路径：{name}");
        }
        total = total.saturating_add(file.size());
        if file.size() > MAX_FILE_BYTES || total > MAX_ARCHIVE_BYTES {
            bail!("备份解压后超过大小限制");
        }
        let mut bytes = Vec::new();
        file.by_ref()
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_FILE_BYTES || entries.insert(name.clone(), bytes).is_some() {
            bail!("重复或过大的备份文件：{name}");
        }
    }
    let manifest: Manifest =
        serde_json::from_slice(&entries.remove("manifest.json").context("缺少备份清单")?)?;
    if manifest.format != "leetdesk-vault" || manifest.version != 1 {
        bail!("不支持的备份版本");
    }
    for (name, bytes) in &entries {
        if !matches!(entry(name), Some(Entry::Notes)) {
            std::str::from_utf8(bytes).with_context(|| format!("代码不是 UTF-8：{name}"))?;
        }
    }
    let mut summary = Summary::default();
    for (name, bytes) in entries {
        match entry(&name).expect("validated entry") {
            Entry::Notes => {
                let mut temp = tempfile::NamedTempFile::new()?;
                temp.write_all(&bytes)?;
                let result = note_bundle::import(store, temp.path())?;
                summary.notes += result.personal + result.explanations;
                summary.conflicts += result.merged;
            }
            Entry::Solution(id, filename) | Entry::Submission(id, filename) => {
                let submission = name.contains("/submissions/");
                let dir = if submission {
                    store.workspace(id).join("submissions")
                } else {
                    store.workspace(id)
                };
                fs::create_dir_all(&dir)?;
                let path = dir.join(&filename);
                let target = if path.exists() && fs::read(&path)? != bytes {
                    summary.conflicts += 1;
                    let (stem, ext) = filename.rsplit_once('.').context("文件名无效")?;
                    (1..=10_000)
                        .map(|n| dir.join(format!("{stem}-imported-{n}.{ext}")))
                        .find(|p| !p.exists() || fs::read(p).is_ok_and(|b| b == bytes))
                        .context("冲突文件过多")?
                } else {
                    path
                };
                if !target.exists() {
                    atomic_write(&target, &bytes)?;
                }
                if submission {
                    summary.submissions += 1;
                } else {
                    summary.solutions += 1;
                }
            }
        }
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::PracticeLanguage;

    #[test]
    fn backup_roundtrip_preserves_code_notes_and_each_submission() {
        let temp = tempfile::tempdir().unwrap();
        let origin = Store::open(temp.path().join("origin")).unwrap();
        let dir = origin.workspace(206);
        fs::create_dir_all(&dir).unwrap();
        origin
            .save_source_for(206, PracticeLanguage::Go, "package solution\n")
            .unwrap();
        origin
            .save_source_for(206, PracticeLanguage::Java, "class Solution {}\n")
            .unwrap();
        atomic_write(&dir.join("notes.md"), b"# Reverse List\n").unwrap();
        origin
            .save_submission_snapshot(206, PracticeLanguage::Go, "first")
            .unwrap();
        origin
            .save_submission_snapshot(206, PracticeLanguage::Go, "second")
            .unwrap();
        let path = temp.path().join("backup.zip");
        let exported = export(&origin, &path).unwrap();
        assert_eq!(
            (exported.notes, exported.solutions, exported.submissions),
            (1, 2, 2)
        );
        let restored = Store::open(temp.path().join("restored")).unwrap();
        let imported = import(&restored, &path).unwrap();
        assert_eq!(imported.conflicts, 0);
        assert_eq!(
            fs::read_to_string(restored.workspace(206).join("notes.md")).unwrap(),
            "# Reverse List\n"
        );
        assert_eq!(
            restored.source_for(206, PracticeLanguage::Java).unwrap(),
            "class Solution {}\n"
        );
        let mut snapshots = fs::read_dir(restored.workspace(206).join("submissions"))
            .unwrap()
            .map(|entry| fs::read_to_string(entry.unwrap().path()).unwrap())
            .collect::<Vec<_>>();
        snapshots.sort();
        assert_eq!(snapshots, ["first", "second"]);
        let imported_again = import(&restored, &path).unwrap();
        assert_eq!(imported_again.conflicts, 0);
        assert_eq!(
            fs::read_dir(restored.workspace(206).join("submissions"))
                .unwrap()
                .count(),
            2
        );
    }

    #[test]
    fn conflicting_solution_is_kept_alongside_local_work() {
        let temp = tempfile::tempdir().unwrap();
        let origin = Store::open(temp.path().join("origin")).unwrap();
        fs::create_dir_all(origin.workspace(1)).unwrap();
        origin
            .save_source_for(1, PracticeLanguage::Go, "incoming")
            .unwrap();
        let backup = temp.path().join("backup.zip");
        export(&origin, &backup).unwrap();
        let local = Store::open(temp.path().join("local")).unwrap();
        fs::create_dir_all(local.workspace(1)).unwrap();
        local
            .save_source_for(1, PracticeLanguage::Go, "local")
            .unwrap();
        let summary = import(&local, &backup).unwrap();
        assert_eq!(summary.conflicts, 1);
        assert_eq!(local.source(1).unwrap(), "local");
        assert_eq!(
            fs::read_to_string(local.workspace(1).join("solution-imported-1.go")).unwrap(),
            "incoming"
        );
    }

    #[test]
    fn rejects_archive_path_traversal_before_writing_anything() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("bad.zip");
        let mut writer = ZipWriter::new(File::create(&source).unwrap());
        let options = SimpleFileOptions::default();
        writer.start_file("manifest.json", options).unwrap();
        writer
            .write_all(br#"{"format":"leetdesk-vault","version":1}"#)
            .unwrap();
        writer
            .start_file("workspaces/1/../solution.go", options)
            .unwrap();
        writer.write_all(b"bad").unwrap();
        writer.finish().unwrap();
        let store = Store::open(temp.path().join("data")).unwrap();
        assert!(import(&store, &source).is_err());
        assert!(!store.workspace(1).exists());
    }
}
