use gwm::tui::commit_graph::Pipe;

#[test]
fn commit_graph_pipe_hashes_do_not_own_heap_allocations() {
  assert!(
    !std::mem::needs_drop::<Pipe>(),
    "Pipe must stay allocation-free; use git2::Oid instead of owned String hashes"
  );
}
