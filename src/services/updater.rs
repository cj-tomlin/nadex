use log::{error, info, warn};
use self_update::cargo_crate_version;
use semver::Version;

// Repository details for GitHub releases
const REPO_OWNER: &str = "cj-tomlin"; // Updated with actual GitHub username
const REPO_NAME: &str = "nadex";
const BIN_NAME: &str = "nadex";

/// Auto-updater status result
pub enum UpdateStatus {
    /// Application is up to date
    UpToDate,
    /// New version is available
    UpdateAvailable { version: String, notes: String },
    /// Update was successfully applied
    Updated { version: String },
    /// Update check failed
    Error(String),
}

/// Check if a new version is available without installing it
pub fn check_for_update() -> UpdateStatus {
    info!("Checking for updates...");

    match self_update::backends::github::Update::configure()
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .bin_name(BIN_NAME)
        .current_version(cargo_crate_version!())
        .build()
    {
        Ok(updater) => {
            match updater.get_latest_release() {
                Ok(release) => {
                    let current_version = cargo_crate_version!();

                    // Parse versions for proper semantic version comparison
                    let current_semver =
                        Version::parse(current_version).unwrap_or_else(|_| Version::new(0, 0, 0));
                    let release_semver =
                        Version::parse(&release.version).unwrap_or_else(|_| Version::new(0, 0, 0));

                    if release_semver > current_semver {
                        info!("New version available: {}", release.version);
                        UpdateStatus::UpdateAvailable {
                            version: release.version.to_string(),
                            notes: release
                                .body
                                .unwrap_or_else(|| "No release notes available".to_string()),
                        }
                    } else {
                        info!("Application is up to date (version {})", current_version);
                        UpdateStatus::UpToDate
                    }
                }
                Err(e) => {
                    warn!("Failed to get latest release: {}", e);
                    UpdateStatus::Error(e.to_string())
                }
            }
        }
        Err(e) => {
            error!("Failed to configure updater: {}", e);
            UpdateStatus::Error(e.to_string())
        }
    }
}

/// Perform the update to the latest version
pub fn update_to_latest() -> UpdateStatus {
    info!("Updating to latest version...");

    match self_update::backends::github::Update::configure()
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .bin_name(BIN_NAME)
        .current_version(cargo_crate_version!())
        .show_download_progress(true)
        .no_confirm(true)
        .build()
        .and_then(|updater| {
            updater.update()?;
            Ok(updater.get_latest_release()?.version.to_string())
        }) {
        Ok(version) => {
            info!("Successfully updated to version {}", version);
            UpdateStatus::Updated { version }
        }
        Err(e) => {
            error!("Update failed: {}", e);
            UpdateStatus::Error(e.to_string())
        }
    }
}
