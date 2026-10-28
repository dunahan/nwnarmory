use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before Unix epoch")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("nwnarmory-{label}-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).expect("create temporary test directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write(path: &Path, text: &str) {
    fs::write(path, text).expect("write test fixture");
}

fn ini() -> &'static str {
    "[Global]\nnTransforms=1\n\n[s0]\nmatch=*\nsubstitute=*\n"
}

fn transform_ini() -> &'static str {
    "[Global]\nnTransforms=1\n\n[s0]\nmatch=source\nsubstitute=target\nscale=(2, 3, 4)\ntranslate=(1, -1, 0.5)\nposition=(9, 8, 7)\ntscale=(2, 2)\nttranslate=(0.1, -0.2)\n"
}

fn valid_model() -> &'static str {
    "newmodel source\nnode dummy source\n  position 0 0 0\n  verts 1\n    0 0 0\nendnode\ndonemodel source\n"
}

fn transformed_model() -> &'static str {
    "newmodel source\nnode dummy source\n  bitmap armor_diffuse\n  position 1 2 3\n  verts 1\n    1 1 1\n  tverts 1\n    0.25 0.5 0.5\nendnode\ndonemodel source\n"
}

fn run(ini_path: &Path, source: &Path, destination: &Path) -> std::process::Output {
    run_with_args(&[], ini_path, source, destination)
}

fn run_with_args(
    extra_args: &[&str],
    ini_path: &Path,
    source: &Path,
    destination: &Path,
) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_nwnarmory"))
        .args(extra_args)
        .arg(ini_path)
        .arg(source)
        .arg(destination)
        .output()
        .expect("run nwnarmory")
}

fn run_values(source: &Path, target: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_nwnarmory"))
        .arg("--values")
        .arg(source)
        .arg(target)
        .output()
        .expect("run nwnarmory --values")
}

fn run_values_with_node_map(node_map: &Path, source: &Path, target: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_nwnarmory"))
        .arg("--values")
        .arg("--node-map")
        .arg(node_map)
        .arg(source)
        .arg(target)
        .output()
        .expect("run nwnarmory --values --node-map")
}

#[test]
fn successful_transform_writes_expected_model_and_preserves_bitmap() {
    let temp = TempDir::new("successful-transform");
    let ini_path = temp.path().join("rules.ini");
    let source = temp.path().join("source.mdl");
    let destination = temp.path().join("out");
    let target = destination.join("target.mdl");

    write(&ini_path, transform_ini());
    write(&source, transformed_model());

    let output = run(&ini_path, &source, &destination);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(target.exists());
    assert_eq!(
        fs::read_to_string(&target).expect("read transformed model"),
        "newmodel target\nnode dummy target\n  bitmap armor_diffuse\n  position 9.0000000 8.0000000 7.0000000\n  verts 1\n    3.0000000 2.0000000 4.5000000\n  tverts 1\n    0.6000000 0.8000000 0.5000000\nendnode\ndonemodel target\n"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("1 file(s) written"));
}

#[test]
fn existing_target_is_preserved_and_reported_as_failure() {
    let temp = TempDir::new("existing-target");
    let ini_path = temp.path().join("rules.ini");
    let source = temp.path().join("source.mdl");
    let destination = temp.path().join("out");
    let target = destination.join("source.mdl");

    write(&ini_path, ini());
    write(&source, valid_model());
    fs::create_dir_all(&destination).expect("create destination directory");
    write(&target, "do not overwrite me\n");

    let output = run(&ini_path, &source, &destination);

    assert!(
        !output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(&target).expect("read existing target"),
        "do not overwrite me\n"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("already exists"));
}

#[test]
fn duplicate_targets_in_one_run_are_skipped_and_reported_as_failure() {
    let temp = TempDir::new("duplicate-target");
    let ini_path = temp.path().join("rules.ini");
    let source_dir = temp.path().join("source");
    let destination = temp.path().join("out");

    write(
        &ini_path,
        "[Global]\nnTransforms=1\n\n[s0]\nmatch=*\nsubstitute=same\n",
    );
    fs::create_dir_all(&source_dir).expect("create source directory");
    write(&source_dir.join("alpha.mdl"), valid_model());
    write(&source_dir.join("beta.mdl"), valid_model());

    let output = run(&ini_path, &source_dir, &destination);

    assert!(
        !output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(destination.join("same.mdl").exists());

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("already reserved in this run"),
        "Expected notification missing. Actual stderr:\n{stderr}"
    );
}

#[test]
fn malformed_model_leaves_no_output_or_temporary_file() {
    let temp = TempDir::new("malformed-model");
    let ini_path = temp.path().join("rules.ini");
    let source = temp.path().join("source.mdl");
    let destination = temp.path().join("out");

    write(&ini_path, ini());
    write(&source, "verts 2\n  0 0 0\n");

    let output = run(&ini_path, &source, &destination);

    assert!(
        !output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!destination.join("source.mdl").exists());
    assert!(!destination.join("source.mdl.tmp").exists());

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Block 'verts': unexpected end of file"),
        "Expected strict EOF diagnostic missing. Actual stderr:\n{stderr}"
    );
}

#[test]
fn dry_run_rejects_a_missing_explicit_source_file() {
    let temp = TempDir::new("dry-run-missing-source");
    let ini_path = temp.path().join("rules.ini");
    let missing_source = temp.path().join("does-not-exist.mdl");
    let destination = temp.path().join("out");

    write(&ini_path, ini());

    let output = run_with_args(&["--dry-run"], &ini_path, &missing_source, &destination);

    assert!(
        !output.status.success(),
        "dry-run unexpectedly succeeded.\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot open source file"),
        "Expected missing-source diagnostic. Actual stderr:\n{stderr}"
    );
    assert!(
        stderr.contains(&missing_source.display().to_string()),
        "Expected the source path in the diagnostic. Actual stderr:\n{stderr}"
    );

    assert!(
        !destination.join("does-not-exist.mdl").exists(),
        "dry-run must not create a planned output"
    );
}

#[test]
fn batch_keeps_successful_outputs_but_returns_failure_when_one_model_fails() {
    let temp = TempDir::new("partial-batch");
    let ini_path = temp.path().join("rules.ini");
    let source_dir = temp.path().join("source");
    let destination = temp.path().join("out");

    write(&ini_path, ini());
    fs::create_dir_all(&source_dir).expect("create source directory");
    write(&source_dir.join("good.mdl"), valid_model());
    write(&source_dir.join("broken.mdl"), "verts 2\n  0 0 0\n");

    let output = run(&ini_path, &source_dir, &destination);

    assert!(
        !output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(destination.join("good.mdl").exists());
    assert!(!destination.join("broken.mdl").exists());
    assert!(String::from_utf8_lossy(&output.stderr).contains("1 file(s) failed"));
}

#[test]
fn rename_bitmap_flag_changes_only_the_bitmap_value() {
    let temp = TempDir::new("rename-bitmap");
    let ini_path = temp.path().join("rules.ini");
    let source = temp.path().join("source.mdl");
    let destination = temp.path().join("out");
    let target = destination.join("target.mdl");

    write(&ini_path, transform_ini());
    write(&source, transformed_model());

    let output = run_with_args(
        &["--rename-bitmap=target_diffuse"],
        &ini_path,
        &source,
        &destination,
    );

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let generated = fs::read_to_string(&target).expect("read transformed model");
    assert!(generated.contains("  bitmap target_diffuse\n"));
    assert!(!generated.contains("  bitmap armor_diffuse\n"));
    assert!(generated.contains("    3.0000000 2.0000000 4.5000000\n"));
}

#[test]
fn rename_materialname_flag_preserves_default_and_supports_literal_name() {
    let temp = TempDir::new("rename-materialname");
    let ini_path = temp.path().join("rules.ini");
    let source = temp.path().join("source.mdl");
    let keep_destination = temp.path().join("keep");
    let renamed_destination = temp.path().join("renamed");

    write(&ini_path, transform_ini());
    write(
        &source,
        "newmodel source\nnode dummy source\n  materialname source_material\nendnode\ndonemodel source\n",
    );

    let output = run(&ini_path, &source, &keep_destination);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let generated =
        fs::read_to_string(keep_destination.join("target.mdl")).expect("read transformed model");
    assert!(generated.contains("  materialname source_material\n"));

    let output = run_with_args(
        &["--rename-materialname=custom_material"],
        &ini_path,
        &source,
        &renamed_destination,
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let generated =
        fs::read_to_string(renamed_destination.join("target.mdl")).expect("read renamed model");
    assert!(generated.contains("  materialname custom_material\n"));
    assert!(!generated.contains("  materialname source_material\n"));
}

#[test]
fn renaming_model_preserves_file_dependency_references() {
    let temp = TempDir::new("file-dependency-references");
    let ini_path = temp.path().join("rules.ini");
    let source = temp.path().join("source.mdl");
    let destination = temp.path().join("out");
    let target = destination.join("target.mdl");

    write(&ini_path, transform_ini());
    write(
        &source,
        "newmodel source\nfiledependancy source_texture.tga\nfiledependency source_material.mtr\nnode dummy source\nendnode\ndonemodel source\n",
    );

    let output = run(&ini_path, &source, &destination);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let generated = fs::read_to_string(&target).expect("read transformed model");
    assert!(generated.contains("filedependancy source_texture.tga\n"));
    assert!(generated.contains("filedependency source_material.mtr\n"));
    assert!(generated.contains("newmodel target\n"));
}

#[test]
fn values_flag_prints_ini_ready_fit_for_corresponding_models() {
    let temp = TempDir::new("values");
    let source = temp.path().join("pm01_chest001.mdl");
    let target = temp.path().join("pma1_chest001.mdl");

    write(
        &source,
        "newmodel pm01_chest001\n\
         node dummy pm01_chest001\n\
           position 0 0 0\n\
           verts 4\n\
             0 0 0\n\
             1 0 0\n\
             0 1 0\n\
             0 0 1\n\
           tverts 3\n\
             0 0 0\n\
             1 0 0\n\
             0 1 0\n\
         endnode\n\
         donemodel pm01_chest001\n",
    );

    write(
        &target,
        "newmodel pma1_chest001\n\
         node dummy pma1_chest001\n\
           position 9 8 7\n\
           verts 4\n\
             1 -2 0.5\n\
             3 -2 0.5\n\
             1 1 0.5\n\
             1 -2 4.5\n\
           tverts 3\n\
             0.1 -0.2 0\n\
             2.1 -0.2 0\n\
             0.1 2.8 0\n\
         endnode\n\
         donemodel pma1_chest001\n",
    );

    let output = run_values(&source, &target);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "nTransforms=#",
        "[s#]",
        "match=pm??_chest???",
        "substitute=??a*",
        "scale=(2.0000, 3.0000, 4.0000)",
        "translate=(1.0000, -2.0000, 0.5000)",
        "tscale=(2.0000, 3.0000)",
        "ttranslate=(0.1000, -0.2000)",
        "position=(9.0000, 8.0000, 7.0000)",
        "; verts fit: n=4 max_residual=0.00000 mean_residual=0.00000",
        "; tverts fit: n=3 max_residual=0.00000 mean_residual=0.00000",
    ] {
        assert!(
            stdout.contains(expected),
            "Expected output fragment missing: {expected}\nActual stdout:\n{stdout}"
        );
    }

    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn node_map_reports_multiple_mappings_and_missing_targets() {
    let temp = TempDir::new("node-map-multiple");
    let source = temp.path().join("pmh0_robe001.mdl");
    let target = temp.path().join("pma0_robe001.mdl");
    let node_map = temp.path().join("node-map.ini");
    write(
        &source,
        "newmodel pmh0_robe001\n\
         node dummy pmh0_robe001\n\
         endnode\n\
         node dummy MHRobe\n\
         parent pmh0_robe001\n\
         position 0 0 1\n\
         endnode\n\
         donemodel pmh0_robe001\n",
    );
    write(
        &target,
        "newmodel pma0_robe001\n\
         node dummy pma0_robe001\n\
         endnode\n\
         node dummy MARobe\n\
         parent pma0_robe001\n\
         position 0 0 2\n\
         endnode\n\
         node dummy MDRobe\n\
         parent pma0_robe001\n\
         position 0 0 3\n\
         endnode\n\
         donemodel pma0_robe001\n",
    );
    write(
        &node_map,
        "[NodeMap0]\nmatch=MHRobe\nsubstitute=MARobe\n\
         [NodeMap1]\nmatch=MHRobe\nsubstitute=MDRobe\n\
         [NodeMap2]\nmatch=MHRobe\nsubstitute=MERobe\n",
    );

    let output = run_values_with_node_map(&node_map, &source, &target);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[NodeMap0] match=mhrobe substitute=marobe"));
    assert!(stdout.contains("[NodeMap1] match=mhrobe substitute=mdrobe"));
    assert!(stdout.contains("mapped mhrobe (dummy) -> marobe (dummy)"));
    assert!(stdout.contains("mapped mhrobe (dummy) -> mdrobe (dummy)"));
    assert!(stdout.contains("mapped source mhrobe -> merobe"));
}

#[test]
fn node_map_rejects_missing_and_duplicate_entries() {
    let temp = TempDir::new("node-map-invalid");
    let source = temp.path().join("source.mdl");
    let target = temp.path().join("target.mdl");
    let node_map = temp.path().join("node-map.ini");
    write(
        &source,
        "newmodel source\nnode dummy source\nendnode\ndonemodel source\n",
    );
    write(
        &target,
        "newmodel target\nnode dummy target\nendnode\ndonemodel target\n",
    );

    write(&node_map, "[NodeMap0]\nmatch=source\n");
    let output = run_values_with_node_map(&node_map, &source, &target);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("[NodeMap0] is missing substitute"));

    write(
        &node_map,
        "[NodeMap0]\nmatch=source\nmatch=other\nsubstitute=target\n",
    );
    let output = run_values_with_node_map(&node_map, &source, &target);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("duplicate [NodeMap] key 'match'"));
}

#[test]
fn skeleton_reference_values_report_segment_length_ratio() {
    let temp = TempDir::new("skeleton-reference");
    let source = temp.path().join("pmh0.mdl");
    let target = temp.path().join("pmd0.mdl");
    write(
        &source,
        "newmodel pmh0\nsetsupermodel pmh0 NULL\nsetanimationscale 1.0\n\
         node dummy pmh0\nendnode\n\
         node dummy torso\nparent pmh0\nposition 0 0 1\nendnode\n\
         donemodel pmh0\n",
    );
    write(
        &target,
        "newmodel pmd0\nsetsupermodel pmd0 NULL\nsetanimationscale 0.9\n\
         node dummy pmd0\nendnode\n\
         node dummy torso\nparent pmd0\nposition 0 0 2\nendnode\n\
         donemodel pmd0\n",
    );

    let output = run_values(&source, &target);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Skeleton comparison: pmh0 -> pmd0"));
    assert!(stdout.contains("1.000000 | 2.000000 | 2.0000"));
    assert!(stdout.contains("animation scale=1.0000"));
    assert!(stdout.contains("animation scale=0.9000"));
}

#[test]
fn node_transform_renames_node_and_parent_and_sets_position_and_orientation() {
    let temp = TempDir::new("node-transform");
    let ini_path = temp.path().join("rules.ini");
    let source = temp.path().join("source.mdl");
    let destination = temp.path().join("out");
    let target = destination.join("target.mdl");
    write(
        &ini_path,
        "[Global]\nnTransforms=1\n\n[s0]\nmatch=source\nsubstitute=target\n\
         node-match=mhrobe\nnode-substitute=mdrobe\n\
         node-position=(4, 5, 6)\nnode-orientation=(0, 0, 0, 1)\n",
    );
    write(
        &source,
        "newmodel source\n\
         node dummy source\nendnode\n\
         node dummy MHRobe\n\
         parent source\n\
         position 1 2 3\nendnode\n\
         donemodel source\n",
    );

    let output = run(&ini_path, &source, &destination);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let generated = fs::read_to_string(target).expect("read transformed model");
    assert!(generated.contains("node dummy mdrobe\n"));
    assert!(generated.contains("parent target\n"));
    assert!(generated.contains("position 4.0000000 5.0000000 6.0000000\n"));
    assert!(generated.contains("orientation 0.0000000 0.0000000 0.0000000 1.0000000\n"));
}

#[test]
fn node_transform_preserves_source_orientation_when_no_override_is_configured() {
    let temp = TempDir::new("node-transform-keep-orientation");
    let ini_path = temp.path().join("rules.ini");
    let source = temp.path().join("source.mdl");
    let destination = temp.path().join("out");
    let target = destination.join("target.mdl");
    write(
        &ini_path,
        "[Global]\nnTransforms=1\n\n[s0]\nmatch=source\nsubstitute=target\n\
         node-match=mhrobe\nnode-substitute=mdrobe\nnode-position=(4, 5, 6)\n",
    );
    write(
        &source,
        "newmodel source\n\
         node dummy source\nendnode\n\
         node dummy MHRobe\nparent source\nposition 1 2 3\n\
         orientation 0.1 0.2 0.3 0.9\nendnode\n\
         donemodel source\n",
    );

    let output = run(&ini_path, &source, &destination);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let generated = fs::read_to_string(target).expect("read transformed model");
    assert!(generated.contains("orientation 0.1 0.2 0.3 0.9\n"));
}

#[test]
fn dry_run_reports_planned_output_without_creating_destination() {
    let temp = TempDir::new("dry-run");
    let ini_path = temp.path().join("rules.ini");
    let source = temp.path().join("source.mdl");
    let destination = temp.path().join("out");

    write(&ini_path, transform_ini());
    write(&source, transformed_model());

    let output = run_with_args(&["--dry-run"], &ini_path, &source, &destination);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !destination.exists(),
        "dry-run must not create the destination directory"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Would process") && stderr.contains("target.mdl"),
        "Expected planned output missing. Actual stderr:\n{stderr}"
    );
    assert!(stderr.contains("1 file(s) would be written"), "{stderr}");
}

#[test]
fn help_and_version_flags_succeed_without_positional_arguments() {
    let binary = env!("CARGO_BIN_EXE_nwnarmory");

    let help = Command::new(binary)
        .arg("--help")
        .output()
        .expect("run --help");
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stderr).contains("--dry-run, -n"));

    let version = Command::new(binary)
        .arg("--version")
        .output()
        .expect("run --version");
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8_lossy(&version.stdout).trim(),
        format!("nwnarmory {}", env!("CARGO_PKG_VERSION"))
    );
}
