use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchOverrides {
    pub executable: Option<PathBuf>,
    pub home: Option<PathBuf>,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
}

impl LaunchOverrides {
    // After `compose_child_path`, so an instance PATH wins; the home field wins over `env`.
    pub(crate) fn apply(&self, cmd: &mut crate::process::Command, home_var: Option<&str>) {
        for (key, value) in &self.env {
            cmd.env(key, value);
        }
        if let (Some(var), Some(home)) = (home_var, &self.home) {
            cmd.env(var, home);
        }
    }

    pub(crate) fn home_or(&self, variable: &str, fallback: PathBuf) -> PathBuf {
        self.home
            .clone()
            .or_else(|| {
                self.env
                    .get(variable)
                    .filter(|value| !value.is_empty())
                    .map(PathBuf::from)
            })
            .unwrap_or_else(|| crate::model_context::root(variable, fallback))
    }
}
