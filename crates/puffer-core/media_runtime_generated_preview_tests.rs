use super::*;
use tempfile::tempdir;

#[test]
fn generated_media_preview_by_artifact_uses_sidecar_path() {
    let workspace = tempdir().unwrap();
    let service = crate::runtime::media::MediaGenerationService::new(workspace.path());
    let image_path = service
        .write_image_artifact_bytes("artifact-1", "image.jpeg", &[0xff, 0xd8, 0xff, 0xd9])
        .unwrap();
    service
        .save_artifact(&crate::runtime::media::MediaArtifact {
            id: "artifact-1".to_string(),
            job_id: "job-1".to_string(),
            kind: crate::runtime::media::MediaKind::Image,
            path: image_path,
            mime_type: "image/jpeg".to_string(),
            byte_count: 4,
            metadata: serde_json::json!({}),
            created_at_ms: 1,
        })
        .unwrap();

    let result = read_generated_media_preview_by_artifact(workspace.path(), "artifact-1");

    assert_eq!(
        result,
        GeneratedMediaPreviewResult::Available {
            mime_type: "image/jpeg".to_string(),
            bytes: vec![0xff, 0xd8, 0xff, 0xd9],
        }
    );
}

#[test]
fn generated_media_preview_by_artifact_rejects_symlink_escape() {
    let workspace = tempdir().unwrap();
    let outside = tempdir().unwrap();
    let outside_image = outside.path().join("image.jpeg");
    std::fs::write(&outside_image, [0xff, 0xd8, 0xff, 0xd9]).unwrap();
    let link_dir = workspace.path().join(".puffer/media/images/artifact-1");
    std::fs::create_dir_all(&link_dir).unwrap();
    let link = link_dir.join("image.jpeg");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside_image, &link).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(&outside_image, &link).unwrap();

    let service = crate::runtime::media::MediaGenerationService::new(workspace.path());
    service
        .save_artifact(&crate::runtime::media::MediaArtifact {
            id: "artifact-1".to_string(),
            job_id: "job-1".to_string(),
            kind: crate::runtime::media::MediaKind::Image,
            path: link,
            mime_type: "image/jpeg".to_string(),
            byte_count: 4,
            metadata: serde_json::json!({}),
            created_at_ms: 1,
        })
        .unwrap();

    assert_eq!(
        read_generated_media_preview_by_artifact(workspace.path(), "artifact-1"),
        GeneratedMediaPreviewResult::Unsupported
    );
}

#[test]
fn generated_media_attachment_metadata_rejects_symlink_escape() {
    let workspace = tempdir().unwrap();
    let outside = tempdir().unwrap();
    let outside_image = outside.path().join("image.jpeg");
    std::fs::write(&outside_image, [0xff, 0xd8, 0xff, 0xd9]).unwrap();
    let link_dir = workspace.path().join(".puffer/media/images/artifact-1");
    std::fs::create_dir_all(&link_dir).unwrap();
    let link = link_dir.join("image.jpeg");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside_image, &link).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(&outside_image, &link).unwrap();

    let service = crate::runtime::media::MediaGenerationService::new(workspace.path());
    service
        .save_artifact(&crate::runtime::media::MediaArtifact {
            id: "artifact-1".to_string(),
            job_id: "job-1".to_string(),
            kind: crate::runtime::media::MediaKind::Image,
            path: link,
            mime_type: "image/jpeg".to_string(),
            byte_count: 4,
            metadata: serde_json::json!({}),
            created_at_ms: 1,
        })
        .unwrap();

    assert_eq!(
        generated_media_attachment_metadata(workspace.path(), "artifact-1"),
        None
    );
}

#[test]
fn generated_media_preview_by_artifact_sniffs_mime_when_extension_lies() {
    let workspace = tempdir().unwrap();
    let service = crate::runtime::media::MediaGenerationService::new(workspace.path());
    let image_path = service
        .write_image_artifact_bytes("artifact-1", "image.png", &[0xff, 0xd8, 0xff, 0xd9])
        .unwrap();
    service
        .save_artifact(&crate::runtime::media::MediaArtifact {
            id: "artifact-1".to_string(),
            job_id: "job-1".to_string(),
            kind: crate::runtime::media::MediaKind::Image,
            path: image_path,
            mime_type: "image/png".to_string(),
            byte_count: 4,
            metadata: serde_json::json!({}),
            created_at_ms: 1,
        })
        .unwrap();

    let result = read_generated_media_preview_by_artifact(workspace.path(), "artifact-1");

    assert!(matches!(
        result,
        GeneratedMediaPreviewResult::Available { mime_type, .. } if mime_type == "image/jpeg"
    ));
}
