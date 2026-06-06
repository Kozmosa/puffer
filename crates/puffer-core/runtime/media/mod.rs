pub(crate) mod artifacts;
pub(crate) mod capabilities;
pub(crate) mod chat_image_output;
pub(crate) mod discovery;
pub(crate) mod http_support;
pub(crate) mod images_json;
pub(crate) mod jobs;
pub(crate) mod minimax_image;
pub(crate) mod replicate_video;
pub(crate) mod resolver;

pub(crate) use artifacts::MediaArtifact;
pub(crate) use capabilities::MediaKind;
pub(crate) use jobs::{MediaJob, MediaJobStatus};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};

/// Provides workspace-local media job and artifact persistence helpers.
#[derive(Debug, Clone)]
pub(crate) struct MediaGenerationService {
    workspace_root: PathBuf,
}

impl MediaGenerationService {
    /// Creates a media persistence service rooted in the workspace.
    pub(crate) fn new(workspace_root: impl AsRef<Path>) -> Self {
        Self {
            workspace_root: workspace_root.as_ref().to_path_buf(),
        }
    }

    /// Resolves a safe artifact file path below the service artifact directory.
    pub(crate) fn artifact_file_path(&self, artifact_id: &str, filename: &str) -> Result<PathBuf> {
        validate_simple_id(artifact_id, "artifact id")?;
        validate_artifact_filename(filename)?;
        Ok(self.artifacts_dir().join(artifact_id).join(filename))
    }

    /// Resolves a safe generated-image file path below the service image directory.
    pub(crate) fn image_artifact_file_path(
        &self,
        artifact_id: &str,
        filename: &str,
    ) -> Result<PathBuf> {
        validate_simple_id(artifact_id, "artifact id")?;
        validate_artifact_filename(filename)?;
        Ok(self.images_dir().join(artifact_id).join(filename))
    }

    /// Writes generated artifact bytes to a safe artifact path.
    pub(crate) fn write_artifact_bytes(
        &self,
        artifact_id: &str,
        filename: &str,
        bytes: &[u8],
    ) -> Result<PathBuf> {
        let path = self.artifact_file_path(artifact_id, filename)?;
        write_media_bytes(&path, bytes)?;
        Ok(path)
    }

    /// Writes generated image bytes to a safe image artifact path.
    pub(crate) fn write_image_artifact_bytes(
        &self,
        artifact_id: &str,
        filename: &str,
        bytes: &[u8],
    ) -> Result<PathBuf> {
        let path = self.image_artifact_file_path(artifact_id, filename)?;
        write_media_bytes(&path, bytes)?;
        Ok(path)
    }

    /// Persists a media job JSON sidecar.
    pub(crate) fn save_job(&self, job: &MediaJob) -> Result<()> {
        validate_simple_id(&job.id, "job id")?;
        write_json_sidecar(&self.job_sidecar_path(&job.id)?, job)
    }

    /// Loads a media job JSON sidecar by id.
    pub(crate) fn load_job(&self, job_id: &str) -> Result<MediaJob> {
        read_json_sidecar(&self.job_sidecar_path(job_id)?)
    }

    /// Persists a media artifact JSON sidecar.
    pub(crate) fn save_artifact(&self, artifact: &MediaArtifact) -> Result<()> {
        validate_simple_id(&artifact.id, "artifact id")?;
        write_json_sidecar(&self.artifact_sidecar_path(&artifact.id)?, artifact)
    }

    /// Loads a media artifact JSON sidecar by id.
    pub(crate) fn load_artifact(&self, artifact_id: &str) -> Result<MediaArtifact> {
        read_json_sidecar(&self.artifact_sidecar_path(artifact_id)?)
    }

    fn media_dir(&self) -> PathBuf {
        self.workspace_root.join(".puffer").join("media")
    }

    fn jobs_dir(&self) -> PathBuf {
        self.media_dir().join("jobs")
    }

    fn artifacts_dir(&self) -> PathBuf {
        self.media_dir().join("artifacts")
    }

    fn images_dir(&self) -> PathBuf {
        self.media_dir().join("images")
    }

    fn artifact_sidecars_dir(&self) -> PathBuf {
        self.media_dir().join("artifact-sidecars")
    }

    fn job_sidecar_path(&self, job_id: &str) -> Result<PathBuf> {
        validate_simple_id(job_id, "job id")?;
        Ok(self.jobs_dir().join(format!("{job_id}.json")))
    }

    fn artifact_sidecar_path(&self, artifact_id: &str) -> Result<PathBuf> {
        validate_simple_id(artifact_id, "artifact id")?;
        Ok(self
            .artifact_sidecars_dir()
            .join(format!("{artifact_id}.json")))
    }
}

fn write_media_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create media output directory {}", parent.display()))?;
    }
    fs::write(path, bytes).with_context(|| format!("write media bytes {}", path.display()))?;
    Ok(())
}

fn write_json_sidecar<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create media sidecar directory {}", parent.display()))?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(value)?)
        .with_context(|| format!("write media sidecar {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("commit media sidecar {}", path.display()))?;
    Ok(())
}

fn read_json_sidecar<T>(path: &Path) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    let text = fs::read_to_string(path)
        .with_context(|| format!("read media sidecar {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parse media sidecar {}", path.display()))
}

fn validate_simple_id(value: &str, field: &str) -> Result<()> {
    if value.trim().is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        bail!("{field} must be a simple identifier");
    }
    Ok(())
}

fn validate_artifact_filename(filename: &str) -> Result<()> {
    let mut components = Path::new(filename).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(name)), None)
            if !name.is_empty() && name.to_string_lossy() != "." =>
        {
            Ok(())
        }
        _ => bail!("artifact filename must be a single safe filename"),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn media_job_status_terminal_states_are_explicit() {
        assert!(!MediaJobStatus::Queued.is_terminal());
        assert!(!MediaJobStatus::Running.is_terminal());
        assert!(MediaJobStatus::Succeeded.is_terminal());
        assert!(MediaJobStatus::Failed.is_terminal());
        assert!(MediaJobStatus::Canceled.is_terminal());
    }

    #[test]
    fn media_job_rejects_transition_out_of_terminal_state() {
        let mut job = MediaJob::new(
            "job-1",
            MediaKind::Image,
            "openai",
            "gpt-image-1",
            "draw a ship",
            10,
        );

        job.transition(MediaJobStatus::Running, 11).unwrap();
        job.transition(MediaJobStatus::Succeeded, 12).unwrap();

        let error = job.transition(MediaJobStatus::Running, 13).unwrap_err();
        assert!(error.to_string().contains("terminal media job"));
    }

    #[test]
    fn media_artifact_paths_must_stay_under_artifact_directory() {
        let temp = tempfile::tempdir().unwrap();
        let service = MediaGenerationService::new(temp.path());

        let path = service
            .artifact_file_path("artifact-1", "image.png")
            .unwrap();
        assert!(path.starts_with(temp.path().join(".puffer/media/artifacts")));

        let error = service
            .artifact_file_path("artifact-1", "../escape.png")
            .unwrap_err();
        assert!(error.to_string().contains("artifact filename"));
    }

    #[test]
    fn media_image_paths_must_stay_under_image_directory() {
        let temp = tempfile::tempdir().unwrap();
        let service = MediaGenerationService::new(temp.path());

        let path = service
            .image_artifact_file_path("artifact-1", "image.png")
            .unwrap();
        assert!(path.starts_with(temp.path().join(".puffer/media/images")));

        let error = service
            .image_artifact_file_path("artifact-1", "../escape.png")
            .unwrap_err();
        assert!(error.to_string().contains("artifact filename"));
    }

    #[test]
    fn media_jobs_and_artifacts_roundtrip_through_json_sidecars() {
        let temp = tempfile::tempdir().unwrap();
        let service = MediaGenerationService::new(temp.path());
        let mut job = MediaJob::new(
            "job-1",
            MediaKind::Image,
            "openai",
            "gpt-image-1",
            "draw a ship",
            10,
        );
        job.transition(MediaJobStatus::Running, 11).unwrap();
        service.save_job(&job).unwrap();

        let loaded_job = service.load_job("job-1").unwrap();
        assert_eq!(loaded_job.status, MediaJobStatus::Running);
        assert_eq!(loaded_job.prompt, "draw a ship");

        let artifact_path = service
            .write_artifact_bytes("artifact-1", "ship.png", b"png-bytes")
            .unwrap();
        let artifact = MediaArtifact {
            id: "artifact-1".to_string(),
            job_id: job.id.clone(),
            kind: MediaKind::Image,
            path: artifact_path.clone(),
            mime_type: "image/png".to_string(),
            byte_count: 9,
            metadata: json!({"size": "1024x1024"}),
            created_at_ms: 12,
        };
        service.save_artifact(&artifact).unwrap();

        let loaded_artifact = service.load_artifact("artifact-1").unwrap();
        assert_eq!(loaded_artifact.path, artifact_path);
        assert_eq!(std::fs::read(artifact_path).unwrap(), b"png-bytes");
    }
}
