//! Reproducible microbenchmarks. Synthetic data; no personal account files.
use leetdesk::{
    catalog::{self, Problem},
    catalog_view::CatalogView,
    store::{self, Progress, Store},
};
use std::{fs, hint::black_box, time::Instant};

fn measure(mut operation: impl FnMut(usize), iterations: usize) -> f64 {
    let mut samples = vec![];
    for _ in 0..5 {
        let start = Instant::now();
        for i in 0..iterations {
            operation(black_box(i));
        }
        samples.push(start.elapsed().as_secs_f64() * 1e6 / iterations as f64);
    }
    samples.sort_by(f64::total_cmp);
    samples[2]
}

// The previous sidebar search: read SQL progress and normalize each row on redraw.
fn previous_filter(problems: &[Problem], store: &Store, query: &str) -> Vec<usize> {
    let progress = store.all_progress();
    let query = query.to_lowercase();
    problems
        .iter()
        .enumerate()
        .filter_map(|(i, p)| {
            let local = progress.get(&p.id).cloned().unwrap_or_default();
            black_box(local);
            let matches = query.is_empty()
                || format!(
                    "{} {} {} {}",
                    p.online
                        .as_ref()
                        .map(|m| m.frontend_id.clone())
                        .unwrap_or_else(|| p.id.to_string()),
                    p.title,
                    p.slug,
                    p.tags.join(" ")
                )
                .to_lowercase()
                .contains(&query);
            matches.then_some(i)
        })
        .collect()
}

fn main() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let store = Store::open(temp.path().join("data"))?;
    let seed = catalog::builtin()?.remove(0);
    let problems: Vec<_> = (1..=4470)
        .map(|id| {
            let mut p = seed.clone();
            p.id = id;
            p.slug = format!("synthetic-problem-{id}");
            p.title = format!("第 {id} 题 · 数组与哈希表");
            p.tags = vec!["数组".into(), "哈希表".into(), format!("分类{}", id % 20)];
            p
        })
        .collect();
    for p in problems.iter().step_by(7) {
        store.save_progress(
            p.id,
            &Progress {
                favorite: true,
                state: "待复习".into(),
                tags: vec!["易错".into()],
            },
        )?;
    }
    let start = Instant::now();
    let view = CatalogView::new(&problems, store.all_progress());
    println!(
        "4470 synthetic problems, {} progress rows",
        store.all_progress().len()
    );
    println!(
        "index initialization: {:.3} ms",
        start.elapsed().as_secs_f64() * 1000.
    );
    let queries = ["数组", "synthetic-problem-12", "分类3", "不存在", "44"];
    for q in queries {
        assert_eq!(
            previous_filter(&problems, &store, q).as_slice(),
            view.filter("全部题目", "全部", q).as_ref()
        );
    }
    let old = measure(
        |i| {
            black_box(previous_filter(
                &problems,
                &store,
                queries[i % queries.len()],
            ));
        },
        100,
    );
    let changed = measure(
        |i| {
            black_box(view.filter("全部题目", "全部", queries[i % queries.len()]));
        },
        100,
    );
    view.filter("全部题目", "全部", "数组");
    let cached = measure(
        |_| {
            black_box(view.filter("全部题目", "全部", "数组"));
        },
        1_000_000,
    );
    println!("sidebar previous search + SQL: {old:.3} us/op");
    println!("sidebar changed query (indexed scan): {changed:.3} us/op");
    println!("sidebar unchanged query (cached redraw): {cached:.6} us/op");
    println!("changed query speedup: {:.1}x", old / changed);
    let file = temp.path().join("draft.go");
    let source = "// unchanged Go draft\n".repeat(1600);
    store::atomic_write(&file, source.as_bytes())?;
    let old_save = measure(
        |_| {
            let temporary = file.with_extension("saving");
            fs::write(&temporary, source.as_bytes()).unwrap();
            fs::rename(&temporary, &file).unwrap();
        },
        100,
    );
    let new_save = measure(
        |_| {
            store::atomic_write(&file, source.as_bytes()).unwrap();
        },
        100,
    );
    println!("unchanged draft previous replace: {old_save:.3} us/op");
    println!("unchanged draft skip replace: {new_save:.3} us/op");
    println!("unchanged save speedup: {:.1}x", old_save / new_save);
    println!(
        "Median of 5 batches; initialization excluded; these are component timings, not end-to-end input latency."
    );
    Ok(())
}
