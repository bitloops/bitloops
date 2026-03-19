use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct FtfRunConfig {
    pub binary_path: PathBuf,
    pub suite_root: PathBuf,
}

#[derive(Debug, cucumber::World)]
pub struct FtfWorld {
    pub scenario_name: Option<String>,
    pub scenario_slug: Option<String>,
    pub flow_name: Option<String>,
    pub run_config: Option<Arc<FtfRunConfig>>,
    pub run_dir: Option<PathBuf>,
    pub repo_dir: Option<PathBuf>,
    pub terminal_log_path: Option<PathBuf>,
    pub metadata_path: Option<PathBuf>,
}

impl Default for FtfWorld {
    fn default() -> Self {
        Self {
            scenario_name: None,
            scenario_slug: None,
            flow_name: None,
            run_config: None,
            run_dir: None,
            repo_dir: None,
            terminal_log_path: None,
            metadata_path: None,
        }
    }
}

impl FtfWorld {
    pub fn reset(&mut self) {
        self.flow_name = None;
        self.run_dir = None;
        self.repo_dir = None;
        self.terminal_log_path = None;
        self.metadata_path = None;
    }

    pub fn prepare(
        &mut self,
        config: Arc<FtfRunConfig>,
        scenario_name: &str,
        scenario_slug: String,
    ) {
        self.run_config = Some(config);
        self.scenario_name = Some(scenario_name.to_string());
        self.scenario_slug = Some(scenario_slug);
        self.reset();
    }

    pub fn run_config(&self) -> &Arc<FtfRunConfig> {
        self.run_config
            .as_ref()
            .expect("ftf run config should be initialized before step execution")
    }

    pub fn run_dir(&self) -> &Path {
        self.run_dir
            .as_deref()
            .expect("ftf run directory should be initialized by CleanStart")
    }

    pub fn repo_dir(&self) -> &Path {
        self.repo_dir
            .as_deref()
            .expect("ftf repo directory should be initialized by CleanStart")
    }

    pub fn terminal_log_path(&self) -> &Path {
        self.terminal_log_path
            .as_deref()
            .expect("ftf terminal log should be initialized by CleanStart")
    }

    pub fn metadata_path(&self) -> &Path {
        self.metadata_path
            .as_deref()
            .expect("ftf run metadata should be initialized by CleanStart")
    }
}
