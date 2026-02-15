//! Platform abstraction module.
//!
//! Provides unified APIs for platform-specific concerns: directory resolution,
//! GPU backend selection, and directory creation. All platform-specific code is
//! isolated here behind a common interface.

use std::path::PathBuf;
use std::{fmt, io};

/// Errors that can occur during platform operations.
#[derive(Debug)]
pub enum PlatformError {
    /// The OS did not provide a configuration directory.
    NoConfigDir,
    /// An I/O error occurred (e.g., directory creation failed).
    Io(io::Error),
}

impl fmt::Display for PlatformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoConfigDir => write!(f, "could not determine OS configuration directory"),
            Self::Io(e) => write!(f, "platform I/O error: {e}"),
        }
    }
}

impl std::error::Error for PlatformError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for PlatformError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// OS-specific directory paths for the Nebula Engine application.
///
/// Each field resolves to the platform-appropriate location following OS
/// conventions (XDG on Linux, Known Folders on Windows, Library on macOS).
pub struct PlatformDirs {
    /// User configuration: `config.ron`, keybindings, etc.
    pub config_dir: PathBuf,
    /// Persistent data: save games, world data.
    pub data_dir: PathBuf,
    /// Ephemeral cache: shader cache, asset cache.
    pub cache_dir: PathBuf,
    /// Log files.
    pub log_dir: PathBuf,
}

const APP_NAME: &str = "nebula-engine";

impl PlatformDirs {
    /// Resolve platform-specific directories without creating them on disk.
    ///
    /// # Errors
    ///
    /// Returns [`PlatformError::NoConfigDir`] if the OS does not expose a
    /// configuration directory.
    pub fn resolve() -> Result<Self, PlatformError> {
        let config_base = dirs::config_dir().ok_or(PlatformError::NoConfigDir)?;
        let app_config = config_base.join(APP_NAME);

        let data_dir = dirs::data_dir()
            .unwrap_or_else(|| app_config.clone())
            .join(APP_NAME);

        let cache_dir = dirs::cache_dir()
            .unwrap_or_else(|| app_config.clone())
            .join(APP_NAME);

        Ok(Self {
            config_dir: app_config.join("config"),
            data_dir,
            cache_dir,
            log_dir: app_config.join("logs"),
        })
    }

    /// Resolve directories and create them on disk.
    ///
    /// # Errors
    ///
    /// Returns [`PlatformError`] if resolution or directory creation fails.
    pub fn resolve_and_create() -> Result<Self, PlatformError> {
        let dirs = Self::resolve()?;
        std::fs::create_dir_all(&dirs.config_dir)?;
        std::fs::create_dir_all(&dirs.data_dir)?;
        std::fs::create_dir_all(&dirs.cache_dir)?;
        std::fs::create_dir_all(&dirs.log_dir)?;
        Ok(dirs)
    }

    /// Resolve directories rooted under a custom base path.
    ///
    /// Useful for testing without touching real OS directories.
    pub fn resolve_with_root(root: &std::path::Path) -> Self {
        let app_dir = root.join(APP_NAME);
        Self {
            config_dir: app_dir.join("config"),
            data_dir: app_dir.join("data"),
            cache_dir: app_dir.join("cache"),
            log_dir: app_dir.join("logs"),
        }
    }

    /// Create all directories on disk. The directories in `self` must already
    /// be populated (via [`resolve`](Self::resolve) or
    /// [`resolve_with_root`](Self::resolve_with_root)).
    ///
    /// # Errors
    ///
    /// Returns [`PlatformError::Io`] if any directory cannot be created.
    pub fn create_dirs(&self) -> Result<(), PlatformError> {
        std::fs::create_dir_all(&self.config_dir)?;
        std::fs::create_dir_all(&self.data_dir)?;
        std::fs::create_dir_all(&self.cache_dir)?;
        std::fs::create_dir_all(&self.log_dir)?;
        Ok(())
    }
}

/// Returns the preferred `wgpu::Backends` for the current platform.
///
/// - **Linux**: Vulkan
/// - **Windows**: DX12 + Vulkan
/// - **macOS**: Metal
pub fn preferred_backends() -> u32 {
    // Returns a bitmask matching wgpu::Backends values.
    // We avoid depending on wgpu here; nebula-render will consume this.
    #[cfg(target_os = "linux")]
    {
        // Vulkan
        1 << 1
    }

    #[cfg(target_os = "windows")]
    {
        // DX12 | Vulkan
        (1 << 3) | (1 << 1)
    }

    #[cfg(target_os = "macos")]
    {
        // Metal
        1 << 2
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        // All backends as fallback
        0xFFFF
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_dir_exists() {
        let config = dirs::config_dir();
        assert!(config.is_some(), "dirs::config_dir() returned None");
        let path = config.unwrap();
        assert!(!path.as_os_str().is_empty(), "config_dir path is empty");
    }

    #[test]
    fn test_data_dir_exists() {
        let data = dirs::data_dir();
        assert!(data.is_some(), "dirs::data_dir() returned None");
        let path = data.unwrap();
        assert!(!path.as_os_str().is_empty(), "data_dir path is empty");
    }

    #[test]
    fn test_platform_dirs_resolve() {
        let dirs = PlatformDirs::resolve().expect("PlatformDirs::resolve() failed");
        assert!(dirs.config_dir.is_absolute(), "config_dir is not absolute");
        assert!(dirs.data_dir.is_absolute(), "data_dir is not absolute");
        assert!(dirs.cache_dir.is_absolute(), "cache_dir is not absolute");
        assert!(dirs.log_dir.is_absolute(), "log_dir is not absolute");
        assert!(
            !dirs.config_dir.as_os_str().is_empty(),
            "config_dir is empty"
        );
        assert!(!dirs.data_dir.as_os_str().is_empty(), "data_dir is empty");
        assert!(!dirs.cache_dir.as_os_str().is_empty(), "cache_dir is empty");
        assert!(!dirs.log_dir.as_os_str().is_empty(), "log_dir is empty");
    }

    #[test]
    fn test_directory_creation() {
        let tmp = std::env::temp_dir().join("nebula-test-platform-dirs");
        // Clean up from any prior run.
        let _ = std::fs::remove_dir_all(&tmp);

        let dirs = PlatformDirs::resolve_with_root(&tmp);
        dirs.create_dirs()
            .expect("create_dirs failed for temp root");

        assert!(dirs.config_dir.exists(), "config_dir was not created");
        assert!(dirs.data_dir.exists(), "data_dir was not created");
        assert!(dirs.cache_dir.exists(), "cache_dir was not created");
        assert!(dirs.log_dir.exists(), "log_dir was not created");

        // Clean up.
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_preferred_backends_not_empty() {
        let backends = preferred_backends();
        assert_ne!(backends, 0, "preferred_backends() returned empty bitmask");
    }

    #[test]
    fn test_no_hardcoded_separators() {
        // Scan all .rs source files for hardcoded path separator patterns
        // in format strings that look like path construction.
        let crates_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("could not find crates directory");

        let mut violations = Vec::new();
        scan_dir_for_separators(crates_dir, &mut violations);

        assert!(
            violations.is_empty(),
            "Found hardcoded path separators in source files:\n{}",
            violations.join("\n")
        );
    }

    #[cfg(test)]
    fn scan_dir_for_separators(dir: &std::path::Path, violations: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // Skip target directories.
                if path.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                scan_dir_for_separators(&path, violations);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let Ok(content) = std::fs::read_to_string(&path) else {
                    continue;
                };
                for (line_num, line) in content.lines().enumerate() {
                    // Skip comments and this test file itself.
                    let trimmed = line.trim();
                    if trimmed.starts_with("//") || trimmed.starts_with("///") {
                        continue;
                    }
                    // Look for format!("...{}\\{}..." or "...{}/{}..." patterns
                    // that suggest path construction via string formatting.
                    if (line.contains("format!") || line.contains("println!"))
                        && (line.contains(r#"{}\\{}"#) || line.contains("{}/{}/"))
                    {
                        violations.push(format!(
                            "  {}:{}: {}",
                            path.display(),
                            line_num + 1,
                            trimmed
                        ));
                    }
                }
            }
        }
    }
}
