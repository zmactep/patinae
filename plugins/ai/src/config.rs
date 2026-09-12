//! Plugin-local configuration, loaded once per invocation in the worker.

use patinae_plugin::tasks::TaskError;
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

// Bound configuration reads and runaway agent loops independently of host limits.
const MAX_CONFIG_BYTES: u64 = 64 * 1024;
const MAX_STEPS: usize = 128;
// Keep startup and model tool discovery bounded when several servers are configured.
const MAX_MCP_SERVERS: usize = 16;

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
    #[serde(default)]
    pub mcp_servers: BTreeMap<String, McpServerConfig>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct McpServerConfig {
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub cwd: Option<PathBuf>,
    pub url: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub env_headers: BTreeMap<String, String>,
}

impl McpServerConfig {
    fn validate(&self) -> Result<(), TaskError> {
        let invalid = || {
            TaskError::new("configuration", "MCP server requires either command (with optional args, env, cwd) or url (with optional headers, env_headers)")
        };
        match (&self.command, &self.url) {
            (Some(command), None) if !command.trim().is_empty() => {
                if !self.headers.is_empty()
                    || !self.env_headers.is_empty()
                    || self
                        .env
                        .keys()
                        .any(|name| name.is_empty() || name.contains(['=', '\0']))
                {
                    return Err(invalid());
                }
            }
            (None, Some(url)) => {
                if !self.args.is_empty() || !self.env.is_empty() || self.cwd.is_some() {
                    return Err(invalid());
                }
                validate_http_url(url)?;
                self.http_headers()?;
            }
            _ => return Err(invalid()),
        }
        Ok(())
    }

    pub fn http_headers(
        &self,
    ) -> Result<
        std::collections::HashMap<reqwest::header::HeaderName, reqwest::header::HeaderValue>,
        TaskError,
    > {
        let mut headers = std::collections::HashMap::new();
        for (name, source) in self
            .headers
            .iter()
            .map(|(name, value)| (name, Ok(value.clone())))
            .chain(
                self.env_headers
                    .iter()
                    .map(|(name, variable)| (name, std::env::var(variable))),
            )
        {
            let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| TaskError::new("configuration", "invalid MCP HTTP header name"))?;
            // Protocol headers belong to the SDK; duplicate auth headers are ambiguous.
            if matches!(
                name.as_str(),
                "host"
                    | "content-length"
                    | "content-type"
                    | "accept"
                    | "connection"
                    | "transfer-encoding"
            ) || name.as_str().starts_with("mcp-")
                || headers.contains_key(&name)
            {
                return Err(TaskError::new(
                    "configuration",
                    "reserved or duplicate MCP HTTP header",
                ));
            }
            let value = source.map_err(|_| {
                TaskError::new(
                    "configuration",
                    "MCP header environment variable is unavailable",
                )
            })?;
            let mut value = reqwest::header::HeaderValue::from_str(&value)
                .map_err(|_| TaskError::new("configuration", "invalid MCP HTTP header value"))?;
            value.set_sensitive(true);
            headers.insert(name, value);
        }
        Ok(headers)
    }
}

fn validate_endpoint(endpoint: &str) -> Result<(), TaskError> {
    let url = validate_http_url(endpoint)?;
    if url.query().is_some() {
        return Err(TaskError::new(
            "configuration",
            "AI endpoint must not contain query parameters",
        ));
    }
    Ok(())
}

fn validate_http_url(endpoint: &str) -> Result<reqwest::Url, TaskError> {
    let url = reqwest::Url::parse(endpoint)
        .map_err(|_| TaskError::new("configuration", "invalid endpoint URL"))?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !(url.scheme() == "https" || url.scheme() == "http" && loopback)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(TaskError::new(
            "configuration",
            "endpoint requires HTTPS (HTTP allowed on loopback), without URL userinfo or fragment",
        ));
    }
    Ok(url)
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
        validate_endpoint(&self.endpoint)?;
        if self.model.trim().is_empty()
            || !(1..=MAX_STEPS).contains(&self.max_steps)
            || !(1..=600).contains(&self.timeout_seconds)
        {
            return Err(TaskError::new(
                "configuration",
                "set model, max_steps (1..128), and timeout_seconds (1..600)",
            ));
        }
        if self.mcp_servers.len() > MAX_MCP_SERVERS {
            return Err(TaskError::new(
                "configuration",
                "too many MCP servers (maximum 16)",
            ));
        }
        for (name, server) in &self.mcp_servers {
            if name.is_empty()
                || name.len() > 32
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            {
                return Err(TaskError::new("configuration", "MCP server names must contain 1..32 ASCII letters, digits, underscores or hyphens"));
            }
            server.validate().map_err(|error| {
                TaskError::new(error.code, format!("MCP server {name}: {}", error.message))
            })?;
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
            mcp_servers: BTreeMap::new(),
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

    #[test]
    fn mcp_config_accepts_transports_and_rejects_ambiguous_or_secret_bearing_errors() {
        for server in [
            "command = 'server'\nargs = ['--stdio']\nenv = { TOKEN = 'fixture' }\ncwd = 'tools'",
            "url = 'https://example.com/mcp'\nheaders = { Authorization = 'Bearer fixture' }",
            "url = 'https://example.com/mcp?api_key=private-token&option=foo%2Fbar'",
        ] {
            let source = format!(
                "{}\n[mcp_servers.fixture]\n{server}",
                include_str!("../config.example.toml")
            );
            let config: Config = toml::from_str(&source).unwrap();
            config.validate().unwrap();
        }
        for server in [
            "command = 'server'\nurl = 'https://example.com/mcp'",
            "command = ''", "args = ['--stdio']",
            "command = 'server'\nheaders = { Authorization = 'private-token' }",
            "url = 'https://example.com/mcp'\ncwd = 'tools'",
            "url = 'http://example.com/mcp'",
            "url = 'https://example.com/mcp?key=private-token#fragment'",
            "url = 'https://example.com/mcp'\nheaders = { Host = 'private-token' }",
            "url = 'https://example.com/mcp'\nheaders = { Authorization = 'private-token', authorization = 'private-token' }",
            "url = 'https://example.com/mcp'\nenv_headers = { Authorization = 'INVALID=ENV' }",
        ] {
            let config: McpServerConfig = toml::from_str(server).unwrap();
            let error = config.validate().unwrap_err();
            assert!(!error.message.contains("private-token"));
        }
        let config: McpServerConfig =
            toml::from_str("url = 'https://example.com/mcp'\nenv_headers = { 'X-Test' = 'PATH' }")
                .unwrap();
        assert_eq!(
            config.http_headers().unwrap()[&reqwest::header::HeaderName::from_static("x-test")]
                .to_str()
                .unwrap(),
            std::env::var("PATH").unwrap()
        );
    }

    #[test]
    fn mcp_url_errors_identify_server_without_exposing_url_or_query_values() {
        let config: Config = toml::from_str(&format!(
            "{}\n[mcp_servers.remote]\nurl = 'https://private-host.example/mcp?key=private-token#fragment'",
            include_str!("../config.example.toml")
        )).unwrap();
        let error = config.validate().unwrap_err();
        assert!(error.message.starts_with("MCP server remote:"));
        assert!(!error.message.contains("private-host"));
        assert!(!error.message.contains("private-token"));
    }
}
