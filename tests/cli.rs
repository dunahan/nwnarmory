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
