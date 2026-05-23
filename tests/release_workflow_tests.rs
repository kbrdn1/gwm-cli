use std::fs;

#[test]
fn stable_release_workflow_skips_prerelease_tags() {
  let workflow = fs::read_to_string(".github/workflows/release.yml").unwrap();

  for suffix in ["-rc.", "-alpha.", "-beta."] {
    let guard = format!("!contains(github.event.inputs.tag || github.ref_name, '{suffix}')");
    assert!(
      workflow.contains(&guard),
      "release.yml must guard stable release jobs against {suffix} tags"
    );
  }
}
