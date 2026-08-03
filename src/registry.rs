use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use serde::Deserialize;
use std::fs;
use std::path::Path;

const REGISTRY_URL: &str = "https://registry-1.docker.io";
const AUTH_URL: &str = "https://auth.docker.io/token";

#[derive(Deserialize)]
struct TokenResponse {
    token: String,
}

#[derive(Deserialize)]
struct Manifest {
    layers: Vec<Layer>,
}

#[derive(Deserialize)]
struct Layer {
    digest: String,
}

pub struct ImageRef {
    pub repository: String,
    pub tag: String,
}

impl ImageRef {
    pub fn parse(input: &str) -> Result<Self> {
        let (repo, tag) = if let Some(pos) = input.rfind(':') {
            (&input[..pos], &input[pos + 1..])
        } else {
            (input, "latest")
        };

        let repository = if repo.contains('/') {
            repo.to_string()
        } else {
            format!("library/{}", repo)
        };

        Ok(Self {
            repository,
            tag: tag.to_string(),
        })
    }
}

fn get_token(repository: &str) -> Result<String> {
    let url = format!(
        "{}?service=registry.docker.io&scope=repository:{}:pull",
        AUTH_URL, repository
    );
    let resp = reqwest::blocking::get(&url)
        .with_context(|| format!("failed to fetch token from {}", url))?;
    if !resp.status().is_success() {
        anyhow::bail!("auth request failed: {}", resp.status());
    }
    let token_resp: TokenResponse = resp.json().context("failed to parse token response")?;
    Ok(token_resp.token)
}

fn get_manifest(token: &str, repository: &str, tag: &str) -> Result<Manifest> {
    let url = format!("{}/v2/{}/manifests/{}", REGISTRY_URL, repository, tag);
    let client = reqwest::blocking::Client::new();
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header(
            "Accept",
            "application/vnd.docker.distribution.manifest.v2+json",
        )
        .send()
        .with_context(|| format!("failed to fetch manifest from {}", url))?;

    if !resp.status().is_success() {
        anyhow::bail!("manifest request failed: {}", resp.status());
    }

    resp.json().context("failed to parse manifest")
}

fn download_blob(token: &str, repository: &str, digest: &str) -> Result<Vec<u8>> {
    let url = format!("{}/v2/{}/blobs/{}", REGISTRY_URL, repository, digest);
    let client = reqwest::blocking::Client::new();
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .with_context(|| format!("failed to download blob {}", digest))?;

    if !resp.status().is_success() {
        anyhow::bail!("blob download failed: {}", resp.status());
    }

    resp.bytes()
        .map(|b| b.to_vec())
        .context("failed to read blob bytes")
}

fn extract_layer(data: &[u8], dest: &Path) -> Result<()> {
    let decoder = GzDecoder::new(data);
    let mut archive = tar::Archive::new(decoder);
    archive
        .unpack(dest)
        .context("failed to extract layer tar")?;
    Ok(())
}

pub fn pull(image_ref: &ImageRef, dest: &Path) -> Result<()> {
    eprintln!("Pulling {}:{}...", image_ref.repository, image_ref.tag);

    let token = get_token(&image_ref.repository)?;
    eprintln!("Authenticated");

    let manifest = get_manifest(&token, &image_ref.repository, &image_ref.tag)?;
    eprintln!("Found {} layers", manifest.layers.len());

    fs::create_dir_all(dest).with_context(|| format!("failed to create {}", dest.display()))?;

    for (i, layer) in manifest.layers.iter().enumerate() {
        eprintln!(
            "Downloading layer {}/{}: {}",
            i + 1,
            manifest.layers.len(),
            &layer.digest[..19]
        );
        let data = download_blob(&token, &image_ref.repository, &layer.digest)?;
        eprintln!("Extracting layer...");
        extract_layer(&data, dest)?;
    }

    eprintln!("Pull complete: {}", dest.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_image() {
        let r = ImageRef::parse("alpine").unwrap();
        assert_eq!(r.repository, "library/alpine");
        assert_eq!(r.tag, "latest");
    }

    #[test]
    fn parse_image_with_tag() {
        let r = ImageRef::parse("alpine:3.18").unwrap();
        assert_eq!(r.repository, "library/alpine");
        assert_eq!(r.tag, "3.18");
    }

    #[test]
    fn parse_image_with_namespace() {
        let r = ImageRef::parse("myuser/myimage:v1").unwrap();
        assert_eq!(r.repository, "myuser/myimage");
        assert_eq!(r.tag, "v1");
    }
}
