use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

fn tinybox_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_tinybox"))
}

fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

fn create_test_rootfs() -> TempDir {
    let tmpdir = TempDir::new().expect("failed to create tempdir");
    let rootfs = tmpdir.path();

    fs::create_dir_all(rootfs.join("bin")).expect("failed to create bin");
    fs::create_dir_all(rootfs.join("lib")).expect("failed to create lib");
    fs::create_dir_all(rootfs.join("lib64")).expect("failed to create lib64");
    fs::create_dir_all(rootfs.join("proc")).expect("failed to create proc");
    fs::create_dir_all(rootfs.join("tmp")).expect("failed to create tmp");

    for bin in &["sh", "echo", "cat", "ls", "id", "hostname", "ps"] {
        let src = PathBuf::from("/bin").join(bin);
        let dst = rootfs.join("bin").join(bin);
        if src.exists() {
            fs::copy(&src, &dst).expect("failed to copy binary");
        }
    }

    let output = Command::new("sh")
        .args([
            "-c",
            &format!(
                "ldd {}/bin/* 2>/dev/null | grep -o '/[^ ]*' | sort -u",
                rootfs.display()
            ),
        ])
        .output()
        .expect("failed to run ldd");

    let libs = String::from_utf8_lossy(&output.stdout);
    for lib in libs.lines() {
        let lib = lib.trim();
        if lib.is_empty() || !lib.starts_with('/') {
            continue;
        }
        let src = PathBuf::from(lib);
        if !src.exists() {
            continue;
        }
        let dst = rootfs.join(lib.trim_start_matches('/'));
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).ok();
        }
        fs::copy(&src, &dst).ok();
    }

    let ld_linux = PathBuf::from("/lib64/ld-linux-x86-64.so.2");
    if ld_linux.exists() {
        let dst = rootfs.join("lib64/ld-linux-x86-64.so.2");
        fs::copy(&ld_linux, &dst).ok();
    }

    tmpdir
}

#[test]
fn test_rootfs_basic() {
    if !is_root() {
        eprintln!("skipping: requires root");
        return;
    }

    let rootfs = create_test_rootfs();
    let output = tinybox_bin()
        .args([
            "run",
            "--root",
            rootfs.path().to_str().unwrap(),
            "--",
            "echo",
            "hello",
        ])
        .output()
        .expect("failed to execute tinybox");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("hello"));
}

#[test]
fn test_rootfs_isolation() {
    if !is_root() {
        eprintln!("skipping: requires root");
        return;
    }

    let rootfs = create_test_rootfs();
    let test_file = rootfs.path().join("tmp").join("test.txt");

    let output = tinybox_bin()
        .args([
            "run",
            "--root",
            rootfs.path().to_str().unwrap(),
            "--",
            "sh",
            "-c",
            "echo 'isolated' > /tmp/test.txt && cat /tmp/test.txt",
        ])
        .output()
        .expect("failed to execute tinybox");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("isolated"));

    assert!(
        !test_file.exists(),
        "file should not exist on host after sandbox exits"
    );
}

#[test]
fn test_rootfs_with_hostname() {
    if !is_root() {
        eprintln!("skipping: requires root");
        return;
    }

    let rootfs = create_test_rootfs();
    let output = tinybox_bin()
        .args([
            "run",
            "--root",
            rootfs.path().to_str().unwrap(),
            "--hostname",
            "sandbox3",
            "--",
            "hostname",
        ])
        .output()
        .expect("failed to execute tinybox");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), "sandbox3");
}

#[test]
fn test_rootfs_nonexistent() {
    if !is_root() {
        eprintln!("skipping: requires root");
        return;
    }

    let output = tinybox_bin()
        .args(["run", "--root", "/nonexistent/rootfs", "--", "echo", "test"])
        .output()
        .expect("failed to execute tinybox");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("does not exist") || stderr.contains("not exist"));
}
