use criterion::{criterion_group, criterion_main, Criterion};
use gwm::tui::commit_graph::{render_commits, test_row};
use std::hint::black_box;

fn graph_rows(count: usize) -> Vec<gwm::worktree::CommitRow> {
  let hashes: Vec<String> = (0..count).map(|i| format!("{:040x}", i + 1)).collect();

  (0..count)
    .map(|i| {
      let mut parents = Vec::new();
      if i + 1 < count {
        parents.push(hashes[i + 1].as_str());
      }
      if i % 17 == 0 && i + 7 < count {
        parents.push(hashes[i + 7].as_str());
      }
      test_row(hashes[i].as_str(), &parents)
    })
    .collect()
}

fn render_commit_graph(c: &mut Criterion) {
  let rows = graph_rows(300);
  c.bench_function("render_commits_300_rows", |b| {
    b.iter(|| render_commits(black_box(&rows)));
  });
}

criterion_group!(benches, render_commit_graph);
criterion_main!(benches);
