use criterion::{black_box, criterion_group, criterion_main, Criterion};
use git2::{Repository, Signature};
use gwm::github::BranchLink;
use gwm::tui::recent_commits_lines;
use gwm::worktree::{BranchStatus, WorktreeInfo};
use tempfile::TempDir;

fn fixture_repo(commit_count: usize) -> (TempDir, Repository, WorktreeInfo) {
  let dir = TempDir::new().unwrap();
  let repo = Repository::init(dir.path()).unwrap();
  let sig = Signature::now("gwm-test", "gwm@test").unwrap();

  std::fs::write(dir.path().join("file.txt"), "seed").unwrap();
  repo
    .index()
    .unwrap()
    .add_path(std::path::Path::new("file.txt"))
    .unwrap();
  repo.index().unwrap().write().unwrap();
  {
    let tree_id = repo.index().unwrap().write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[]).unwrap();
  }

  for i in 0..commit_count {
    std::fs::write(dir.path().join("file.txt"), format!("commit-{i}")).unwrap();
    repo
      .index()
      .unwrap()
      .add_path(std::path::Path::new("file.txt"))
      .unwrap();
    repo.index().unwrap().write().unwrap();
    let parent = repo.head().unwrap().peel_to_commit().unwrap();
    {
      let tree_id = repo.index().unwrap().write_tree().unwrap();
      let tree = repo.find_tree(tree_id).unwrap();
      repo
        .commit(Some("HEAD"), &sig, &sig, &format!("commit-{i}"), &tree, &[&parent])
        .unwrap();
    }
  }

  let head = repo.head().unwrap().target().unwrap().to_string();
  let info = WorktreeInfo {
    name: "bench".into(),
    path: dir.path().to_path_buf(),
    branch: Some("main".into()),
    head: Some(head),
    is_main: true,
    is_locked: false,
    is_prunable: false,
    status: BranchStatus::default(),
    link: BranchLink::empty(),
    age: None,
  };

  (dir, repo, info)
}

fn sidebar_recent_commits(c: &mut Criterion) {
  let (_dir, _repo, info) = fixture_repo(300);
  c.bench_function("recent_commits_lines_300_rows", |b| {
    b.iter(|| recent_commits_lines(black_box(&info), black_box(300)));
  });
}

criterion_group!(benches, sidebar_recent_commits);
criterion_main!(benches);
