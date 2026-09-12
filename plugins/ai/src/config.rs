//! Plugin-local configuration, loaded once per invocation in the worker.

use patinae_plugin::tasks::TaskError;
use serde::Deserialize;
use std::{path::Path, time::Duration};

// Bound configuration reads and runaway agent loops independently of host limits.
const MAX_CONFIG_BYTES: u64 = 64 * 1024;
const MAX_STEPS: usize = 128;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    pub endpoint: String,
    pub model: String,
    pub api_key: Option<String>,
    #[serde(default = "default_key_env")]
    pub api_key_env: String,
    #[serde(default = "default_steps")]
    pub max_steps: usize,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
}

fn default_key_env() -> String {
    "OPENAI_API_KEY".into()
}
fn default_steps() -> usize {
    24
}
fn default_timeout() -> u64 {
    120
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, TaskError> {
        use std::io::Read;
        let file = std::fs::File::open(path).map_err(|_| {
            TaskError::new(
                "configuration",
                format!(
                    "cannot open {}; see plugins/ai/config.example.toml",
                    path.display()
                ),
            )
        })?;
        let mut source = String::new();
        file.take(MAX_CONFIG_BYTES + 1)
            .read_to_string(&mut source)
            .map_err(|_| TaskError::new("configuration", "cannot read AI configuration"))?;
        if source.len() as u64 > MAX_CONFIG_BYTES {
            return Err(TaskError::new(
                "configuration",
                "AI configuration is too large",
            ));
        }
        // Do not echo parser input: a misplaced credential could be in the file.
        let config: Self = toml::from_str(&source).map_err(|_| {
            TaskError::new(
                "configuration",
                "invalid AI configuration TOML or unknown field",
            )
        })?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), TaskError> {
        let url = reqwest::Url::parse(&self.endpoint)
            .map_err(|_| TaskError::new("configuration", "invalid endpoint URL"))?;
        let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if !(url.scheme() == "https" || url.scheme() == "http" && loopback)
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(TaskError::new("configuration", "endpoint requires HTTPS (HTTP allowed on loopback), without credentials, query or fragment"));
        }
        if self.model.trim().is_empty()
            || !(1..=MAX_STEPS).contains(&self.max_steps)
            || !(1..=600).contains(&self.timeout_seconds)
        {
            return Err(TaskError::new(
                "configuration",
                "set model, max_steps (1..128), and timeout_seconds (1..600)",
            ));
        }
        Ok(())
    }

    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_seconds)
    }

    pub fn api_key(&self) -> Result<Option<String>, TaskError> {
        if let Some(key) = &self.api_key {
            if key.trim().is_empty() {
                return Err(TaskError::new("configuration", "api_key must not be empty"));
            }
            return Ok(Some(key.clone()));
        }
        if self.api_key_env.is_empty() {
            return Ok(None);
        }
        std::env::var(&self.api_key_env)
            .ok()
            .filter(|key| !key.trim().is_empty())
            .map(Some)
            .ok_or_else(|| {
                TaskError::new(
                    "configuration",
                    format!(
                        "set api_key in configuration or environment variable {} (or api_key_env = \"\" for a local server)",
                        self.api_key_env
                    ),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_and_loop_limits_are_checked() {
        let mut config = Config {
            endpoint: "http://127.0.0.1:11434/v1/responses".into(),
            model: "fixture".into(),
            api_key: None,
            api_key_env: String::new(),
            max_steps: 24,
            timeout_seconds: 120,
        };
        assert!(config.validate().is_ok());
        for endpoint in [
            "http://example.com/v1",
            "https://user:secret@example.com/v1",
            "https://example.com/v1?key=secret",
            "file:///tmp/model",
        ] {
            config.endpoint = endpoint.into();
            assert!(config.validate().is_err(), "{endpoint}");
        }
        config.endpoint = "https://example.com/v1/responses".into();
        assert!(config.validate().is_ok());
        config.max_steps = 0;
        assert!(config.validate().is_err());
        config.max_steps = 129;
        assert!(config.validate().is_err());
        config.max_steps = 24;
        config.timeout_seconds = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn example_is_valid_and_unknown_fields_are_rejected() {
        let config: Config = toml::from_str(include_str!("../config.example.toml")).unwrap();
        assert!(config.validate().is_ok());
        assert_eq!(config.api_key_env, "OPENAI_API_KEY");
        assert!(config.api_key.is_none());
        let with_unknown_field = format!(
            "{}\nunknown_key = 'do-not-echo'\n",
            include_str!("../config.example.toml")
        );
        assert!(toml::from_str::<Config>(&with_unknown_field).is_err());
    }

    #[test]
    fn direct_key_takes_precedence_over_environment() {
        let mut config: Config = toml::from_str(
            "endpoint = 'https://example.com/v1/responses'\nmodel = 'fixture'\napi_key = 'fixture-key'",
        )
        .unwrap();
        assert_eq!(config.api_key_env, "OPENAI_API_KEY");
        // An invalid environment name ensures this cannot succeed via fallback.
        config.api_key_env = "INVALID=ENV".into();
        assert_eq!(config.api_key().unwrap().as_deref(), Some("fixture-key"));
        config.api_key_env.clear();
        assert_eq!(config.api_key().unwrap().as_deref(), Some("fixture-key"));
    }

    #[test]
    fn empty_direct_key_is_rejected() {
        let mut config: Config = toml::from_str(include_str!("../config.example.toml")).unwrap();
        config.api_key_env.clear();
        for key in ["", " \t\n"] {
            config.api_key = Some(key.into());
            assert!(config.api_key().is_err());
        }
    }

    #[test]
    fn absent_direct_key_preserves_environment_and_no_auth_modes() {
        let mut config: Config = toml::from_str(include_str!("../config.example.toml")).unwrap();
        // Read an existing non-secret variable without mutating process environment.
        config.api_key_env = "PATH".into();
        assert_eq!(
            config.api_key().unwrap(),
            Some(std::env::var("PATH").unwrap())
        );
        config.api_key_env = "INVALID=ENV".into();
        assert!(config.api_key().is_err());
        config.api_key_env.clear();
        assert_eq!(config.api_key().unwrap(), None);
    }
}
