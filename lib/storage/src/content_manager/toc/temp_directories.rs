use std::path::{Path, PathBuf};

use collection::operations::types::{CollectionError, CollectionResult};
use fs_err as fs;

use crate::content_manager::toc::TableOfContent;
use crate::types::StorageConfig;

const TEMP_SUBDIR_NAME: &str = "tmp";
const FILE_UPLOAD_SUBDIR_NAME: &str = "upload";

/// Remove all temporary directories based on storage configuration.
///
/// Should be called early during startup, before loading collection data and
/// applying WAL, to ensure no stale temp files (e.g. from interrupted snapshot
/// transfers) interfere with recovery.
pub fn clear_tmp_directories(storage_config: &StorageConfig) -> CollectionResult<()> {
    let snapshots_temp = storage_config.snapshots_path.join(TEMP_SUBDIR_NAME);
    let storage_temp = storage_config.storage_path.join(TEMP_SUBDIR_NAME);
    let optional_temp = storage_config
        .temp_path
        .as_deref()
        .map(|p| Path::new(p).join(TEMP_SUBDIR_NAME));

    clear_tmp_dirs(&snapshots_temp, &storage_temp, optional_temp.as_deref())
}

/// Remove the three canonical tmp directories by path
///
/// Separated from [`clear_tmp_directories`] so unit tests can exercise the
/// removal logic without constructing a full [`StorageConfig`]
fn clear_tmp_dirs(
    snapshots_tmp: &Path,
    storage_tmp: &Path,
    optional_tmp: Option<&Path>,
) -> CollectionResult<()> {
    for path in [Some(snapshots_tmp), Some(storage_tmp), optional_tmp]
        .into_iter()
        .flatten()
    {
        if path.exists() {
            fs::remove_dir_all(path).map_err(|e| {
                CollectionError::service_error(format!(
                    "Failed to remove temp directory at {}: {e:?}",
                    path.display(),
                ))
            })?;
        }
    }

    Ok(())
}

/// Functions for managing temporary storages of TOC.
///
/// The directory structure is as follows:
///
/// ./snapshots
///           └── tmp
///               └── (tempdirs)
/// ./optional_temp_path (if specified)
///           └── tmp
///               └── upload
///               └── (tempdirs)
/// ./storage
///           └── tmp
///               └── (tempdirs)
///
/// optional_temp_path can be used instead of `snapshots/tmp` or `storage/tmp`
/// to speed up processing.
///
/// Assume all temp directories are located on different filesystems, so
/// the choice between them should be made from the performance considerations.
///
/// Subdirectories are required for simpler cleanup on the start of the process.
impl TableOfContent {
    pub fn temp_path(&self) -> Option<&Path> {
        self.storage_config.temp_path.as_deref()
    }

    fn get_snapshots_temp_path(&self) -> PathBuf {
        self.snapshots_path().join(TEMP_SUBDIR_NAME)
    }

    fn get_storage_temp_path(&self) -> PathBuf {
        self.storage_path().join(TEMP_SUBDIR_NAME)
    }

    fn get_optional_temp_path(&self) -> Option<PathBuf> {
        self.temp_path()
            .map(|path| Path::new(path).join(TEMP_SUBDIR_NAME))
    }

    /// Get temporary storage path inside the `snapshots` directory.
    pub fn snapshots_temp_path(&self) -> CollectionResult<PathBuf> {
        let path = self.get_snapshots_temp_path();

        if !path.exists() {
            fs::create_dir_all(&path).map_err(|e| {
                CollectionError::service_error(format!(
                    "Failed to create snapshots temp directory at {}: {:?}",
                    path.display(),
                    e,
                ))
            })?;
        }
        Ok(path)
    }

    /// Get temporary storage path inside the `storage` directory.
    pub fn storage_temp_path(&self) -> CollectionResult<PathBuf> {
        let path = self.get_storage_temp_path();

        if !path.exists() {
            fs::create_dir_all(&path).map_err(|e| {
                CollectionError::service_error(format!(
                    "Failed to create storage temp directory at {}: {:?}",
                    path.display(),
                    e,
                ))
            })?;
        }
        Ok(path)
    }

    /// Get temporary storage path inside the `optional_temp_path` directory.
    pub fn optional_temp_path(&self) -> CollectionResult<Option<PathBuf>> {
        if let Some(path) = self.get_optional_temp_path() {
            if !path.exists() {
                fs::create_dir_all(&path).map_err(|e| {
                    CollectionError::service_error(format!(
                        "Failed to create optional temp directory at {}: {:?}",
                        path.display(),
                        e,
                    ))
                })?;
            }
            Ok(Some(path))
        } else {
            Ok(None)
        }
    }

    /// Get directory for snapshots-related temporary files.
    /// If the optional_temp_path is specified, it will be used instead of snapshots_temp_path.
    pub fn optional_temp_or_snapshot_temp_path(&self) -> CollectionResult<PathBuf> {
        match self.optional_temp_path() {
            Ok(Some(path)) => Ok(path),
            Ok(None) => self.snapshots_temp_path(),
            Err(err) => Err(err),
        }
    }

    /// Get directory for storage-related temporary files.
    /// If the optional_temp_path is specified, it will be used instead of storage_temp_path.
    pub fn optional_temp_or_storage_temp_path(&self) -> CollectionResult<PathBuf> {
        match self.optional_temp_path() {
            Ok(Some(path)) => Ok(path),
            Ok(None) => self.storage_temp_path(),
            Err(err) => Err(err),
        }
    }

    pub fn upload_dir(&self) -> CollectionResult<PathBuf> {
        let tmp_storage_dir = match self.optional_temp_path() {
            Ok(Some(path)) => path,
            Ok(None) => self.snapshots_temp_path()?,
            Err(err) => return Err(err),
        };

        let upload_dir = tmp_storage_dir.join(FILE_UPLOAD_SUBDIR_NAME);

        if !upload_dir.exists() {
            fs::create_dir_all(&upload_dir).map_err(|e| {
                CollectionError::service_error(format!(
                    "Failed to create upload directory at {}: {:?}",
                    upload_dir.display(),
                    e,
                ))
            })?;
        }
        Ok(upload_dir)
    }
}

#[cfg(test)]
mod tests {
    use tempfile::Builder;

    use super::*;

    // only tmp/ subdirs should be removed, files next to them must survive
    #[test]
    fn test_clear_tmp_dirs_removes_only_tmp_subdirs() {
        let temp_dir = Builder::new().prefix("test_clear_tmp").tempdir().unwrap();
        let base_path = temp_dir.path();

        let snapshots_root = base_path.join("snapshots");
        let storage_root = base_path.join("storage");
        let optional_root = base_path.join("optional");

        for root in [&snapshots_root, &storage_root, &optional_root] {
            fs::create_dir_all(root).unwrap();
        }
        let survivor_files = [
            storage_root.join("valid_data.txt"),
            snapshots_root.join("valid_snapshot.tar"),
            optional_root.join("valid_optional.txt"),
        ];
        for f in &survivor_files {
            fs::write(f, "keep me").unwrap();
        }

        let snapshots_tmp = snapshots_root.join(TEMP_SUBDIR_NAME);
        let storage_tmp = storage_root.join(TEMP_SUBDIR_NAME);
        let optional_tmp = optional_root.join(TEMP_SUBDIR_NAME);
        for (tmp, junk) in [
            (&snapshots_tmp, "partial_snap"),
            (&storage_tmp, "dirty_state"),
            (&optional_tmp, "bad_upload"),
        ] {
            fs::create_dir_all(tmp).unwrap();
            fs::write(tmp.join(junk), "junk").unwrap();
        }

        clear_tmp_dirs(&snapshots_tmp, &storage_tmp, Some(&optional_tmp)).unwrap();

        assert!(!snapshots_tmp.exists(), "snapshots tmp should be removed");
        assert!(!storage_tmp.exists(), "storage tmp should be removed");
        assert!(!optional_tmp.exists(), "optional tmp should be removed");

        for f in &survivor_files {
            assert!(f.exists(), "{} should survive cleanup", f.display());
        }
    }

    // when optional path is not set, only the two mandatory dirs are cleaned up
    #[test]
    fn test_clear_tmp_dirs_without_optional_path() {
        let temp_dir = Builder::new()
            .prefix("test_clear_tmp_no_opt")
            .tempdir()
            .unwrap();
        let base_path = temp_dir.path();

        let snapshots_tmp = base_path.join("snapshots").join(TEMP_SUBDIR_NAME);
        let storage_tmp = base_path.join("storage").join(TEMP_SUBDIR_NAME);

        fs::create_dir_all(&snapshots_tmp).unwrap();
        fs::create_dir_all(&storage_tmp).unwrap();
        fs::write(snapshots_tmp.join("stale"), "junk").unwrap();
        fs::write(storage_tmp.join("stale"), "junk").unwrap();

        clear_tmp_dirs(&snapshots_tmp, &storage_tmp, None).unwrap();

        assert!(!snapshots_tmp.exists());
        assert!(!storage_tmp.exists());
    }

    // calling this when dirs don't exist yet should do nothing and not fail
    #[test]
    fn test_clear_tmp_dirs_nonexistent_is_noop() {
        let temp_dir = Builder::new()
            .prefix("test_clear_tmp_noop")
            .tempdir()
            .unwrap();
        let base_path = temp_dir.path();

        let snapshots_tmp = base_path.join("snapshots").join(TEMP_SUBDIR_NAME);
        let storage_tmp = base_path.join("storage").join(TEMP_SUBDIR_NAME);
        let optional_tmp = base_path.join("optional").join(TEMP_SUBDIR_NAME);

        // dirs not created on purpose
        let result = clear_tmp_dirs(&snapshots_tmp, &storage_tmp, Some(&optional_tmp));
        assert!(result.is_ok(), "should be a no-op when dirs are absent");
    }

    // if the parent dir is read only, we should get a ServiceError with the path in the message
    #[cfg(unix)]
    #[test]
    fn test_clear_tmp_dirs_permission_error_is_reported() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt as _;

        let temp_dir = Builder::new()
            .prefix("test_clear_tmp_perm")
            .tempdir()
            .unwrap();
        let base_path = temp_dir.path();

        // snapshots_tmp doesn't exist, storage_tmp is the one we're testing
        let storage_root = base_path.join("storage");
        let storage_tmp = storage_root.join(TEMP_SUBDIR_NAME);
        let snapshots_tmp = base_path.join("snapshots").join(TEMP_SUBDIR_NAME);

        fs::create_dir_all(&storage_tmp).unwrap();
        fs::write(storage_tmp.join("stale"), "junk").unwrap();

        // make parent read only so remove_dir_all can't delete the tmp subdir
        fs::set_permissions(&storage_root, fs::Permissions::from_mode(0o555)).unwrap();

        let result = clear_tmp_dirs(&snapshots_tmp, &storage_tmp, None);

        // restore perms before asserting so tempdir cleanup doesn't fail
        fs::set_permissions(&storage_root, fs::Permissions::from_mode(0o755)).unwrap();

        let err = result.expect_err("expected an error when the dir cannot be removed");
        let err_str = err.to_string();
        assert!(
            err_str.contains(storage_tmp.to_str().unwrap()),
            "error message should contain the offending path; got: {err_str}",
        );
    }
}
