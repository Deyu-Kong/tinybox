use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub const IMAGE_DIR: &str = "/var/lib/tinybox/images";

pub fn image_store() -> PathBuf {
    if let Ok(value) = std::env::var("TINYBOX_IMAGE_DIR") {
        return PathBuf::from(value);
    }
    PathBuf::from(IMAGE_DIR)
}

pub fn import_tar(tar_path: &Path, alias: &str) -> Result<PathBuf> {
    if !tar_path.is_file() {
        anyhow::bail!("tar file not found: {}", tar_path.display());
    }
    validate_name(alias)?;

    let store = image_store();
    fs::create_dir_all(&store).with_context(|| format!("failed to create {}", store.display()))?;

    let dest = store.join(alias);
    if dest.exists() {
        anyhow::bail!("image alias already exists: {}", dest.display());
    }
    fs::create_dir_all(&dest).with_context(|| format!("failed to create {}", dest.display()))?;

    let file = fs::File::open(tar_path)
        .with_context(|| format!("failed to open tar {}", tar_path.display()))?;
    let mut archive = tar::Archive::new(file);
    archive
        .unpack(&dest)
        .with_context(|| format!("failed to extract tar into {}", dest.display()))?;

    if !dest.join("bin").exists() && !dest.join("usr").exists() {
        anyhow::bail!(
            "extracted image does not look like a rootfs: {}",
            dest.display()
        );
    }
    Ok(dest)
}

pub fn list() -> Result<Vec<String>> {
    let store = image_store();
    if !store.is_dir() {
        return Ok(Vec::new());
    }
    let mut names: Vec<String> = Vec::new();
    for entry in fs::read_dir(&store)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    names.sort();
    Ok(names)
}

pub fn remove(name: &str) -> Result<()> {
    validate_name(name)?;
    let target = image_store().join(name);
    if !target.is_dir() {
        anyhow::bail!("image not found: {}", target.display());
    }
    fs::remove_dir_all(&target)
        .with_context(|| format!("failed to remove image {}", target.display()))?;
    Ok(())
}

pub fn resolve(name: &str) -> Result<PathBuf> {
    validate_name(name)?;
    let target = image_store().join(name);
    if target.is_dir() {
        return Ok(target);
    }
    if target.is_file() {
        return Ok(target);
    }
    anyhow::bail!("image not found: {}", target.display());
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        anyhow::bail!("image name must not be empty");
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        anyhow::bail!("invalid image name: {}", name);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn fresh_store() -> PathBuf {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("tinybox-store-{pid}-{id}"));
        fs::create_dir_all(&dir).unwrap();
        std::env::set_var("TINYBOX_IMAGE_DIR", &dir);
        dir
    }

    fn unique_payload_dir(label: &str) -> PathBuf {
        let unique = tempfile::Builder::new()
            .prefix(&format!("tinybox-payload-{label}-"))
            .tempdir()
            .unwrap()
            .keep();
        unique
    }

    fn make_tar(label: &str) -> PathBuf {
        let tar_path = std::env::temp_dir().join(format!("tinybox-fixture-{label}.tar"));
        let payload = unique_payload_dir(label);
        let bin = payload.join("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("sh"), b"#!/bin/sh\n").unwrap();
        let file = fs::File::create(&tar_path).unwrap();
        let mut builder = tar::Builder::new(file);
        builder.append_dir_all(".", &payload).unwrap();
        builder.finish().unwrap();
        tar_path
    }

    #[test]
    fn import_and_list_roundtrip() {
        let _dir = fresh_store();
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let alias = format!("alpine-{unique}");
        let label = format!("alpine-{unique}");
        let tar = make_tar(&label);
        let extracted = import_tar(&tar, &alias).unwrap();
        assert!(extracted.join("bin").join("sh").exists());
        let names = list().unwrap();
        assert!(names.contains(&alias));
    }

    #[test]
    fn rejects_path_traversal_name() {
        assert!(import_tar(Path::new("nope"), "../escape").is_err());
    }

    #[test]
    fn remove_deletes_image() {
        let _dir = fresh_store();
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let alias = format!("alpine-rm-{unique}");
        let label = format!("alpine-rm-{unique}");
        let tar = make_tar(&label);
        import_tar(&tar, &alias).unwrap();
        remove(&alias).unwrap();
        let names = list().unwrap();
        assert!(!names.contains(&alias));
    }
}
