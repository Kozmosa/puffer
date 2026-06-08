use reqwest::header::CONTENT_TYPE;
use serde::Serialize;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::AppHandle;
use url::Url;

const MAX_IMAGE_DOWNLOAD_BYTES: u64 = 20 * 1024 * 1024;
const IMAGE_DOWNLOAD_TIMEOUT_SECS: u64 = 15;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DownloadImageResult {
    path: String,
}

/// Opens the folder containing an absolute, existing image file path.
#[tauri::command]
pub(crate) fn open_image_containing_folder(app: AppHandle, path: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;

    let dir = resolve_image_containing_folder(Path::new(&path))?;
    app.opener()
        .open_path(dir.to_string_lossy().to_string(), None::<&str>)
        .map_err(|error| error.to_string())
}

/// Downloads an explicit remote image URL into the user's Downloads folder.
#[tauri::command]
pub(crate) async fn download_image_from_url(
    url: String,
    suggested_name: Option<String>,
) -> Result<DownloadImageResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        download_image_from_url_blocking(&url, suggested_name.as_deref())
    })
    .await
    .map_err(|error| error.to_string())?
}

fn resolve_image_containing_folder(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("image path must be absolute".to_string());
    }
    let canonical = path.canonicalize().map_err(|error| error.to_string())?;
    if !canonical.is_file() {
        return Err("image path must be an existing file".to_string());
    }
    canonical
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "image path has no containing folder".to_string())
}

fn validate_image_download_url(input: &str) -> Result<Url, String> {
    let url = Url::parse(input.trim()).map_err(|error| error.to_string())?;
    match url.scheme() {
        "http" | "https" => Ok(url),
        _ => Err("image URL must use http or https".to_string()),
    }
}

fn image_download_allowed(
    content_type: Option<&str>,
    url: &Url,
    suggested_name: Option<&str>,
) -> bool {
    let normalized = normalized_content_type(content_type);
    if normalized.starts_with("image/") {
        return true;
    }
    if !normalized.is_empty() && !generic_download_content_type(&normalized) {
        return false;
    }
    suggested_name
        .and_then(known_image_extension_for_name)
        .or_else(|| url_filename(url).and_then(known_image_extension_for_name))
        .is_some()
}

fn normalized_content_type(content_type: Option<&str>) -> String {
    content_type
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .unwrap_or_default()
}

fn generic_download_content_type(value: &str) -> bool {
    matches!(
        value,
        "application/octet-stream"
            | "binary/octet-stream"
            | "application/download"
            | "application/x-download"
    )
}

fn sanitize_image_download_filename(suggested_name: Option<&str>, url: &Url) -> String {
    let raw = suggested_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| url_filename(url))
        .unwrap_or("image");
    let basename = raw
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(raw)
        .trim()
        .trim_matches('.');
    let mut sanitized = basename
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_' | ' ') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    sanitized = sanitized.trim_matches('.').to_string();
    if sanitized.is_empty() {
        sanitized = "image".to_string();
    }
    if sanitized.len() > 128 {
        sanitized.truncate(128);
        sanitized = sanitized.trim_matches('.').to_string();
    }
    if known_image_extension_for_name(&sanitized).is_none() {
        if let Some(extension) = url_filename(url).and_then(known_image_extension_for_name) {
            sanitized.push('.');
            sanitized.push_str(extension);
        }
    }
    sanitized
}

fn download_image_from_url_blocking(
    url: &str,
    suggested_name: Option<&str>,
) -> Result<DownloadImageResult, String> {
    let parsed = validate_image_download_url(url)?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(IMAGE_DOWNLOAD_TIMEOUT_SECS))
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .get(parsed.clone())
        .send()
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;

    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    if !image_download_allowed(content_type.as_deref(), &parsed, suggested_name) {
        return Err("downloaded content is not an allowed image".to_string());
    }
    if response.content_length().unwrap_or(0) > MAX_IMAGE_DOWNLOAD_BYTES {
        return Err("image download is too large".to_string());
    }

    let mut bytes = Vec::new();
    response
        .take(MAX_IMAGE_DOWNLOAD_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_IMAGE_DOWNLOAD_BYTES {
        return Err("image download is too large".to_string());
    }
    if bytes.is_empty() {
        return Err("image download was empty".to_string());
    }

    let downloads = downloads_dir()?;
    fs::create_dir_all(&downloads).map_err(|error| error.to_string())?;
    let filename = sanitize_image_download_filename(suggested_name, &parsed);
    let final_path = unique_download_path(&downloads, &filename);
    let tmp_path = downloads.join(format!(".puffer-image-{}.download", uuid::Uuid::new_v4()));
    let write_result = File::create(&tmp_path)
        .and_then(|mut file| {
            file.write_all(&bytes)?;
            file.flush()
        })
        .and_then(|_| fs::rename(&tmp_path, &final_path));
    if let Err(error) = write_result {
        let _ = fs::remove_file(&tmp_path);
        return Err(error.to_string());
    }
    Ok(DownloadImageResult {
        path: final_path.display().to_string(),
    })
}

fn downloads_dir() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .map(|home| home.join("Downloads"))
        .ok_or_else(|| "home directory is unavailable".to_string())
}

fn unique_download_path(dir: &Path, filename: &str) -> PathBuf {
    let initial = dir.join(filename);
    if !initial.exists() {
        return initial;
    }
    let path = Path::new(filename);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("image");
    let extension = path.extension().and_then(|value| value.to_str());
    for index in 1..10_000 {
        let candidate = match extension {
            Some(extension) => dir.join(format!("{stem} ({index}).{extension}")),
            None => dir.join(format!("{stem} ({index})")),
        };
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{}-{}", uuid::Uuid::new_v4(), filename))
}

fn url_filename(url: &Url) -> Option<&str> {
    url.path_segments()
        .and_then(|mut segments| segments.next_back())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn known_image_extension_for_name(name: &str) -> Option<&'static str> {
    let extension = Path::new(name)
        .extension()
        .and_then(|value| value.to_str())?
        .to_ascii_lowercase();
    match extension.as_str() {
        "png" => Some("png"),
        "jpg" => Some("jpg"),
        "jpeg" => Some("jpeg"),
        "gif" => Some("gif"),
        "webp" => Some("webp"),
        "bmp" => Some("bmp"),
        "tif" => Some("tif"),
        "tiff" => Some("tiff"),
        "avif" => Some("avif"),
        "heic" => Some("heic"),
        "heif" => Some("heif"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn image_containing_folder_requires_absolute_existing_file() {
        let temp = tempfile::tempdir().unwrap();
        let image = temp.path().join("pixel.png");
        std::fs::write(&image, b"png").unwrap();

        assert_eq!(
            super::resolve_image_containing_folder(&image).unwrap(),
            temp.path().canonicalize().unwrap()
        );
        assert!(super::resolve_image_containing_folder(std::path::Path::new("pixel.png")).is_err());
        assert!(super::resolve_image_containing_folder(temp.path()).is_err());
        assert!(super::resolve_image_containing_folder(&temp.path().join("missing.png")).is_err());
    }

    #[test]
    fn image_download_url_allows_only_http_and_https() {
        assert!(super::validate_image_download_url("https://example.test/pixel.png").is_ok());
        assert!(super::validate_image_download_url("http://example.test/pixel.png").is_ok());
        assert!(super::validate_image_download_url("file:///tmp/pixel.png").is_err());
        assert!(super::validate_image_download_url("ftp://example.test/pixel.png").is_err());
    }

    #[test]
    fn image_download_accepts_image_types_or_generic_known_extensions() {
        let png = super::validate_image_download_url("https://example.test/pixel.png").unwrap();
        let txt = super::validate_image_download_url("https://example.test/pixel.txt").unwrap();

        assert!(super::image_download_allowed(Some("image/png"), &txt, None));
        assert!(super::image_download_allowed(None, &png, None));
        assert!(super::image_download_allowed(
            Some("application/octet-stream"),
            &png,
            None
        ));
        assert!(super::image_download_allowed(
            Some("application/octet-stream"),
            &txt,
            Some("pixel.webp")
        ));
        assert!(!super::image_download_allowed(
            Some("text/plain"),
            &png,
            None
        ));
        assert!(!super::image_download_allowed(None, &txt, None));
    }

    #[test]
    fn image_download_filename_is_sanitized() {
        let url =
            super::validate_image_download_url("https://example.test/assets/pixel.png").unwrap();

        assert_eq!(
            super::sanitize_image_download_filename(Some("../bad:name?.png"), &url),
            "bad_name_.png"
        );
        assert_eq!(
            super::sanitize_image_download_filename(Some(""), &url),
            "pixel.png"
        );
    }
}
