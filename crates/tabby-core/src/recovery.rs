//! Persist the open tab/pane layout so the next launch can reopen it, matching
//! Tabby's `recoverTabs` behaviour. Local shells are reopened as new sessions;
//! SSH panes are reopened using the stored profile (credentials still come from
//! the profile or the password prompt).

use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct RecoveryState {
    pub version: u32,
    pub tabs: Vec<RecoveredTab>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct RecoveredTab {
    pub title: Option<String>,
    pub layout: Option<RecoveredLayout>,
    pub focused_pane: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RecoveredLayout {
    Pane(RecoveredPane),
    Split {
        axis: RecoveredAxis,
        ratio: f32,
        first: Box<RecoveredLayout>,
        second: Box<RecoveredLayout>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecoveredAxis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct RecoveredPane {
    pub profile_id: String,
    pub title: Option<String>,
    pub cwd: Option<String>,
}

impl Default for RecoveredLayout {
    fn default() -> Self {
        Self::Pane(RecoveredPane::default())
    }
}

impl RecoveryState {
    pub fn path() -> std::path::PathBuf {
        crate::config::Config::state_dir().join("recovery.yaml")
    }

    pub fn load() -> Option<Self> {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        match serde_yaml::from_str(&text) {
            Ok(state) => Some(state),
            Err(error) => {
                log::warn!("ignoring malformed recovery state: {error}");
                None
            }
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&Self::path())
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_yaml::to_string(self)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        let temp = path.with_extension(format!("yaml.{}.tmp", std::process::id()));
        std::fs::write(&temp, text)?;
        std::fs::rename(&temp, path)
    }

    pub fn clear() {
        let _ = std::fs::remove_file(Self::path());
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_split_layout_roundtrips() {
        let state = RecoveryState {
            version: 1,
            tabs: vec![RecoveredTab {
                title: Some("tab".into()),
                focused_pane: 1,
                layout: Some(RecoveredLayout::Split {
                    axis: RecoveredAxis::Horizontal,
                    ratio: 0.6,
                    first: Box::new(RecoveredLayout::Pane(RecoveredPane {
                        profile_id: "local:default".into(),
                        title: Some("bash".into()),
                        cwd: Some("/tmp".into()),
                    })),
                    second: Box::new(RecoveredLayout::Split {
                        axis: RecoveredAxis::Vertical,
                        ratio: 0.5,
                        first: Box::new(RecoveredLayout::Pane(RecoveredPane {
                            profile_id: "ssh:prod".into(),
                            ..Default::default()
                        })),
                        second: Box::new(RecoveredLayout::Pane(RecoveredPane::default())),
                    }),
                }),
            }],
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recovery.yaml");
        state.save_to(&path).unwrap();
        let loaded = RecoveryState::load_from(&path).unwrap();
        assert_eq!(loaded.tabs.len(), 1);
        assert_eq!(loaded.tabs[0].focused_pane, 1);
        assert!(matches!(
            loaded.tabs[0].layout,
            Some(RecoveredLayout::Split { .. })
        ));
    }
}
