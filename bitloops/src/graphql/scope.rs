#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub(crate) struct ResolverScope {
    project_path: Option<String>,
}

impl ResolverScope {
    pub(crate) fn project_path(&self) -> Option<&str> {
        self.project_path.as_deref()
    }

    pub(crate) fn with_project_path(&self, project_path: String) -> Self {
        Self {
            project_path: Some(project_path),
        }
    }

    pub(crate) fn contains_repo_path(&self, path: &str) -> bool {
        match self.project_path() {
            Some(project_path) => {
                path == project_path
                    || path
                        .strip_prefix(project_path)
                        .is_some_and(|suffix| suffix.starts_with('/'))
            }
            None => true,
        }
    }
}
