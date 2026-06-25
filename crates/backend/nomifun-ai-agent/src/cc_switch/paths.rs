use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct CcSwitchPaths {
    pub settings_path: PathBuf,
    pub database_path: PathBuf,
    pub claude_settings_path: PathBuf,
}

impl CcSwitchPaths {
    pub fn from_home(home: &Path) -> Self {
        let base = home.join(".cc-switch");
        Self {
            settings_path: base.join("settings.json"),
            database_path: base.join("cc-switch.db"),
            claude_settings_path: home.join(".claude").join("settings.json"),
        }
    }

    pub fn system() -> Option<Self> {
        dirs::home_dir().map(|h| Self::from_home(&h))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn resolves_paths_from_home() {
        let paths = CcSwitchPaths::from_home(Path::new("/home/testuser"));
        assert_eq!(
            paths.settings_path,
            Path::new("/home/testuser/.cc-switch/settings.json")
        );
        assert_eq!(paths.database_path, Path::new("/home/testuser/.cc-switch/cc-switch.db"));
        assert_eq!(
            paths.claude_settings_path,
            Path::new("/home/testuser/.claude/settings.json")
        );
    }

    #[test]
    fn system_returns_some_when_home_exists() {
        let paths = CcSwitchPaths::system();
        assert!(paths.is_some());
    }
}
