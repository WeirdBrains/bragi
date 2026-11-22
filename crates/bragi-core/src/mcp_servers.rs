//! Outside MCP servers (LF-37): the list the user keeps in Settings, their secrets, and connecting.
//!
//! The protocol client and the `ToolSource` adapter are `weirdnet-mcp`'s (features `client` and
//! `agent`); the tool-call loop is `weirdnet-agent`'s `AgentSession`. This module is Bragi's
//! side: the list (`mcp-servers.json`, no secrets), the secrets (an env value, a bearer token or a
//! header value, kept like API keys: OS keychain when a shell opted in, else the sealed file),
//! scopes, "always allow" per tool, and turning an entry into a connected `McpSource`.
//! The views the shells draw only say which secrets are set.
//!
//! A stdio server runs as the user, with the user's permissions. [`McpServerView::warning`] is the text to show before
//! adding one. Every call from these tools asks the user first ([`Choice`](crate::Choice));
//! "always allow" is per tool.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use weirdnet_mcp::agent::{McpSource, SourceOptions};
use weirdnet_mcp::client::{ClientOptions, HttpConfig, Secret, StdioCommand};

use crate::core::{lock, random_id};
use crate::endpoints::Endpoints;
use crate::error::{CoreError, ErrorKind, Result};

pub const MCP_FILE: &str = "mcp-servers.json";
/// Most servers a user can keep.
pub const MAX_SERVERS: usize = 16;

/// How a server is reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpTransport {
    Stdio,
    Http,
}

/// Where a server's tools are offered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum McpScope {
    /// Every chat on an endpoint (the default).
    #[default]
    All,
    /// Code mode on this folder (and its files).
    Folder(String),
    /// One conversation.
    Session(String),
}

/// One saved server. Secrets are not here, only the names they are filed under.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServer {
    pub id: String,
    /// What the user calls it; tools are offered as `<slug>.<tool>`.
    pub name: String,
    pub transport: McpTransport,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Names of the environment variables that have a stored value.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env_names: Vec<String>,
    /// Names of the headers that have a stored value.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub header_names: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub has_bearer: bool,
    #[serde(default)]
    pub scope: McpScope,
    /// Tools (as offered, `<slug>.<tool>`) the user chose "always allow" for.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub always_allow: Vec<String>,
    #[serde(default = "timeout_secs")]
    pub timeout_secs: u64,
}

fn yes() -> bool {
    true
}

fn timeout_secs() -> u64 {
    60
}

/// What the user typed to add or replace a server (no secrets).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerInput {
    pub name: String,
    pub transport: McpTransport,
    pub enabled: bool,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub url: Option<String>,
    pub scope: McpScope,
    pub timeout_secs: Option<u64>,
}

/// The secrets typed with it. Never logged, never in a view.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpSecrets {
    pub env: BTreeMap<String, String>,
    pub headers: BTreeMap<String, String>,
    pub bearer: Option<String>,
}

impl std::fmt::Debug for McpSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("McpSecrets(…)")
    }
}

/// What the shells draw.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerView {
    pub id: String,
    pub name: String,
    pub transport: McpTransport,
    pub enabled: bool,
    /// The command line or the address.
    pub target: String,
    pub env_names: Vec<String>,
    pub header_names: Vec<String>,
    pub has_bearer: bool,
    pub scope: McpScope,
    pub always_allow: Vec<String>,
    /// For a local command: what to show before it is added or started ("runs as you").
    pub warning: Option<String>,
}

/// The result of connecting to a server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpTestReport {
    pub ok: bool,
    pub tools: Vec<String>,
    pub message: String,
}

pub struct McpServers {
    dir: PathBuf,
    list: Mutex<Vec<McpServer>>,
}

fn storage(e: impl std::fmt::Display) -> CoreError {
    CoreError::new(ErrorKind::Storage, e.to_string())
}

fn bad(m: impl Into<String>) -> CoreError {
    CoreError::new(ErrorKind::BadRequest, m)
}

/// `Files Server` becomes `files-server`: what tools are prefixed with.
pub fn slug(name: &str) -> String {
    let mut s: String = name.trim().to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    s.trim_matches('-').to_string()
}

fn loopback(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://") else { return false };
    let host = rest.split(['/', '?']).next().unwrap_or_default();
    let name = if let Some(r) = host.strip_prefix('[') { r.split(']').next().unwrap_or_default() } else { host.split(':').next().unwrap_or_default() };
    name == "localhost" || name == "::1" || name.parse::<std::net::Ipv4Addr>().is_ok_and(|a| a.is_loopback())
}

fn secret_name(id: &str, kind: &str, key: &str) -> String {
    format!("mcp:{id}:{kind}:{key}")
}

impl McpServers {
    pub fn open(dir: &Path) -> Self {
        let list = std::fs::read_to_string(dir.join(MCP_FILE)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Self { dir: dir.to_path_buf(), list: Mutex::new(list) }
    }

    fn save(&self, list: &[McpServer]) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let bytes = serde_json::to_vec_pretty(list).map_err(storage)?;
        crate::identity::write_private_file(&self.dir.join(MCP_FILE), &bytes)
    }

    pub fn list(&self) -> Vec<McpServer> {
        lock(&self.list).clone()
    }

    pub fn get(&self, id: &str) -> Option<McpServer> {
        lock(&self.list).iter().find(|s| s.id == id).cloned()
    }

    pub fn views(&self) -> Vec<McpServerView> {
        lock(&self.list).iter().map(view).collect()
    }

    fn check(input: &McpServerInput, secrets: Option<&McpSecrets>) -> Result<()> {
        if slug(&input.name).is_empty() {
            return Err(bad("give the server a name"));
        }
        match input.transport {
            McpTransport::Stdio => {
                if input.command.as_deref().is_none_or(|c| c.trim().is_empty()) {
                    return Err(bad("a local server needs a command"));
                }
                if input.url.is_some() || secrets.is_some_and(|s| s.bearer.is_some() || !s.headers.is_empty()) {
                    return Err(bad("an address, a token and headers are for web servers"));
                }
            }
            McpTransport::Http => {
                let Some(url) = input.url.as_deref().map(str::trim) else { return Err(bad("a web server needs an address")) };
                if !(url.starts_with("https://") || loopback(url)) {
                    return Err(bad("the address must start with https:// (plain http only for this machine)"));
                }
                if url.split('/').nth(2).is_some_and(|h| h.contains('@')) || url.contains(char::is_whitespace) {
                    return Err(bad("put the token in the token field, not in the address"));
                }
                if input.command.is_some() || !input.args.is_empty() || secrets.is_some_and(|s| !s.env.is_empty()) {
                    return Err(bad("a command, arguments and environment are for local servers"));
                }
            }
        }
        Ok(())
    }

    fn unique_slug(list: &[McpServer], name: &str, except: Option<&str>) -> Result<()> {
        if list.iter().any(|s| Some(s.id.as_str()) != except && slug(&s.name) == slug(name)) {
            return Err(bad("another server has that name"));
        }
        Ok(())
    }

    pub fn add(&self, ep: &Endpoints, input: McpServerInput, secrets: McpSecrets) -> Result<McpServerView> {
        Self::check(&input, Some(&secrets))?;
        let mut list = lock(&self.list);
        if list.len() >= MAX_SERVERS {
            return Err(bad(format!("at most {MAX_SERVERS} servers")));
        }
        Self::unique_slug(&list, &input.name, None)?;
        let id = random_id("m");
        let server = build(&id, input, &secrets, Vec::new());
        self.write_secrets(ep, &id, &McpSecrets::default(), &secrets)?;
        let v = view(&server);
        list.push(server);
        self.save(&list)?;
        Ok(v)
    }

    /// Replaces the server's settings. `secrets: None` keeps the stored secrets; `Some` replaces them all.
    pub fn edit(&self, ep: &Endpoints, id: &str, input: McpServerInput, secrets: Option<McpSecrets>) -> Result<McpServerView> {
        Self::check(&input, secrets.as_ref())?;
        let mut list = lock(&self.list);
        let at = list.iter().position(|s| s.id == id).ok_or_else(|| missing(id))?;
        Self::unique_slug(&list, &input.name, Some(id))?;
        let old = list[at].clone();
        let mut server = match &secrets {
            Some(s) => {
                let old_names = McpSecrets {
                    env: old.env_names.iter().map(|n| (n.clone(), String::new())).collect(),
                    headers: old.header_names.iter().map(|n| (n.clone(), String::new())).collect(),
                    bearer: old.has_bearer.then(String::new),
                };
                self.write_secrets(ep, id, &old_names, s)?;
                build(id, input, s, old.always_allow.clone())
            }
            None => {
                let mut s = build(id, input, &McpSecrets::default(), old.always_allow.clone());
                s.env_names = old.env_names.clone();
                s.header_names = old.header_names.clone();
                s.has_bearer = old.has_bearer;
                s
            }
        };
        // A renamed server's tools have new names: the old "always allow" entries no longer apply.
        if slug(&old.name) != slug(&server.name) {
            server.always_allow.clear();
        }
        let v = view(&server);
        list[at] = server;
        self.save(&list)?;
        Ok(v)
    }

    pub fn set_enabled(&self, id: &str, on: bool) -> Result<()> {
        let mut list = lock(&self.list);
        list.iter_mut().find(|s| s.id == id).ok_or_else(|| missing(id))?.enabled = on;
        self.save(&list)
    }

    pub fn delete(&self, ep: &Endpoints, id: &str) -> Result<()> {
        let mut list = lock(&self.list);
        let at = list.iter().position(|s| s.id == id).ok_or_else(|| missing(id))?;
        let s = list.remove(at);
        let old = McpSecrets {
            env: s.env_names.iter().map(|n| (n.clone(), String::new())).collect(),
            headers: s.header_names.iter().map(|n| (n.clone(), String::new())).collect(),
            bearer: s.has_bearer.then(String::new),
        };
        self.write_secrets(ep, id, &old, &McpSecrets::default())?;
        self.save(&list)
    }

    /// "Always allow" for one tool of a server (as offered, `<slug>.<tool>`); `false` takes it back.
    pub fn set_always_allow(&self, id: &str, tool: &str, on: bool) -> Result<()> {
        let mut list = lock(&self.list);
        let s = list.iter_mut().find(|s| s.id == id).ok_or_else(|| missing(id))?;
        s.always_allow.retain(|t| t != tool);
        if on {
            s.always_allow.push(tool.to_string());
        }
        self.save(&list)
    }

    /// Every tool the user chose "always allow" for, across enabled servers.
    pub fn always_allowed(&self) -> std::collections::HashSet<String> {
        lock(&self.list).iter().filter(|s| s.enabled).flat_map(|s| s.always_allow.iter().cloned()).collect()
    }

    /// The servers that apply to a chat: enabled, and in scope.
    pub fn applying(&self, folder: Option<&str>, session: Option<&str>) -> Vec<McpServer> {
        lock(&self.list)
            .iter()
            .filter(|s| s.enabled)
            .filter(|s| match &s.scope {
                McpScope::All => true,
                McpScope::Folder(f) => folder.is_some_and(|d| Path::new(d).starts_with(f)),
                McpScope::Session(id) => session == Some(id.as_str()),
            })
            .cloned()
            .collect()
    }

    fn write_secrets(&self, ep: &Endpoints, id: &str, old: &McpSecrets, new: &McpSecrets) -> Result<()> {
        for k in old.env.keys().filter(|k| !new.env.contains_key(*k)) {
            ep.put_secret(&secret_name(id, "env", k), None)?;
        }
        for k in old.headers.keys().filter(|k| !new.headers.contains_key(*k)) {
            ep.put_secret(&secret_name(id, "header", k), None)?;
        }
        if old.bearer.is_some() && new.bearer.is_none() {
            ep.put_secret(&secret_name(id, "bearer", ""), None)?;
        }
        for (k, v) in &new.env {
            ep.put_secret(&secret_name(id, "env", k), Some(v))?;
        }
        for (k, v) in &new.headers {
            ep.put_secret(&secret_name(id, "header", k), Some(v))?;
        }
        if let Some(b) = &new.bearer {
            ep.put_secret(&secret_name(id, "bearer", ""), Some(b))?;
        }
        Ok(())
    }

    /// Starts or connects to the server (secrets resolved only here). Its tools are offered as
    /// `<slug>.<tool>`. A local command runs now, as the user.
    pub async fn connect(&self, ep: &Endpoints, id: &str) -> Result<McpSource> {
        let s = self.get(id).ok_or_else(|| missing(id))?;
        let get = |kind: &str, key: &str| -> Result<Secret> {
            Ok(Secret::new(ep.get_secret(&secret_name(&s.id, kind, key))?.unwrap_or_default()))
        };
        let options = ClientOptions { timeout: Duration::from_secs(s.timeout_secs.max(1)), client_name: "bragi".into(), ..ClientOptions::default() };
        let source = match s.transport {
            McpTransport::Stdio => {
                let mut c = StdioCommand::new(s.command.clone().unwrap_or_default());
                for a in &s.args {
                    c = c.arg(a.clone());
                }
                if let Some(d) = &s.cwd {
                    c = c.cwd(d);
                }
                for k in &s.env_names {
                    c = c.env(k.clone(), get("env", k)?);
                }
                McpSource::connect_stdio(s.id.clone(), &c, options).await
            }
            McpTransport::Http => {
                let mut h = HttpConfig::new(s.url.clone().unwrap_or_default());
                if s.has_bearer {
                    h = h.bearer(get("bearer", "")?);
                }
                for k in &s.header_names {
                    h = h.header(k.clone(), get("header", k)?);
                }
                McpSource::connect_http(s.id.clone(), &h, options).await
            }
        }
        .map_err(|e| CoreError::new(ErrorKind::Unavailable, format!("{}: {}", s.name, client_error(&e))))?;
        Ok(source.with_options(SourceOptions { name_prefix: Some(format!("{}.", slug(&s.name))), ..SourceOptions::default() }))
    }

    /// Connects and lists the tools; never an error, the report carries it. Closes the connection.
    pub async fn test(&self, ep: &Endpoints, id: &str) -> McpTestReport {
        let source = match self.connect(ep, id).await {
            Ok(s) => s,
            Err(e) => return McpTestReport { ok: false, tools: Vec::new(), message: e.message },
        };
        let r = source.client().list_tools().await;
        source.client().close().await;
        let prefix = self.get(id).map(|s| slug(&s.name)).unwrap_or_default();
        match r {
            Ok(tools) => McpTestReport {
                ok: true,
                message: format!("Connected; {} tool{}.", tools.len(), if tools.len() == 1 { "" } else { "s" }),
                tools: tools.into_iter().map(|t| format!("{prefix}.{}", t.name)).collect(),
            },
            Err(e) => McpTestReport { ok: false, tools: Vec::new(), message: format!("couldn't list the tools: {}", client_error(&e)) },
        }
    }
}

impl crate::core::Core {
    /// The saved MCP servers, without secrets.
    pub fn mcp_servers(&self) -> Vec<McpServerView> {
        self.inner.mcp.views()
    }

    /// Adds a server. The secrets are stored like API keys and never returned.
    pub fn add_mcp_server(&self, input: McpServerInput, secrets: McpSecrets) -> Result<McpServerView> {
        self.inner.mcp.add(&self.inner.endpoints, input, secrets)
    }

    /// Replaces a server's settings; `secrets: None` keeps what is stored, `Some` replaces it.
    pub fn edit_mcp_server(&self, id: &str, input: McpServerInput, secrets: Option<McpSecrets>) -> Result<McpServerView> {
        self.inner.mcp.edit(&self.inner.endpoints, id, input, secrets)
    }

    pub fn set_mcp_server_enabled(&self, id: &str, on: bool) -> Result<()> {
        self.inner.mcp.set_enabled(id, on)
    }

    pub fn delete_mcp_server(&self, id: &str) -> Result<()> {
        self.inner.mcp.delete(&self.inner.endpoints, id)
    }

    /// Takes back "always allow" for a tool, or gives it.
    pub fn set_mcp_tool_always_allowed(&self, id: &str, tool: &str, on: bool) -> Result<()> {
        self.inner.mcp.set_always_allow(id, tool, on)
    }

    /// Connects now and lists the tools (a local command is started). Never an error.
    pub async fn test_mcp_server(&self, id: &str) -> McpTestReport {
        self.inner.mcp.test(&self.inner.endpoints, id).await
    }
}

fn client_error(e: &weirdnet_mcp::client::ClientError) -> String {
    use weirdnet_mcp::client::ClientError as E;
    match e {
        E::Spawn(m) => format!("couldn't start it ({m})"),
        E::Http { status: 401 | 403, .. } => "the server refused the token".into(),
        E::Http { status, .. } => format!("the server answered HTTP {status}"),
        E::Closed => "the server closed the connection".into(),
        E::Rpc { message, .. } => message.chars().take(200).collect(),
        other => format!("{other:?}").chars().take(200).collect(),
    }
}

fn missing(id: &str) -> CoreError {
    CoreError::new(ErrorKind::NotFound, format!("no MCP server {id}"))
}

fn build(id: &str, input: McpServerInput, secrets: &McpSecrets, always_allow: Vec<String>) -> McpServer {
    McpServer {
        id: id.to_string(),
        name: input.name.trim().to_string(),
        transport: input.transport,
        enabled: input.enabled,
        command: input.command.map(|c| c.trim().to_string()),
        args: input.args,
        cwd: input.cwd.filter(|c| !c.is_empty()),
        url: input.url.map(|u| u.trim().trim_end_matches('/').to_string()),
        env_names: secrets.env.keys().cloned().collect(),
        header_names: secrets.headers.keys().cloned().collect(),
        has_bearer: secrets.bearer.is_some(),
        scope: input.scope,
        always_allow,
        timeout_secs: input.timeout_secs.unwrap_or(60).clamp(1, 600),
    }
}

fn view(s: &McpServer) -> McpServerView {
    let (target, warning) = match s.transport {
        McpTransport::Stdio => {
            let mut c = StdioCommand::new(s.command.clone().unwrap_or_default());
            for a in &s.args {
                c = c.arg(a.clone());
            }
            (c.display(), Some(c.warning()))
        }
        McpTransport::Http => (s.url.clone().unwrap_or_default(), None),
    };
    McpServerView {
        id: s.id.clone(),
        name: s.name.clone(),
        transport: s.transport,
        enabled: s.enabled,
        target,
        env_names: s.env_names.clone(),
        header_names: s.header_names.clone(),
        has_bearer: s.has_bearer,
        scope: s.scope.clone(),
        always_allow: s.always_allow.clone(),
        warning,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stdio(name: &str) -> McpServerInput {
        McpServerInput {
            name: name.into(),
            transport: McpTransport::Stdio,
            enabled: true,
            command: Some("/bin/true".into()),
            args: vec!["--x".into()],
            cwd: None,
            url: None,
            scope: McpScope::All,
            timeout_secs: None,
        }
    }

    fn web(url: &str) -> McpServerInput {
        McpServerInput { transport: McpTransport::Http, command: None, args: vec![], url: Some(url.into()), ..stdio("Web") }
    }

    #[test]
    fn slugs() {
        assert_eq!(slug(" Files  Server! "), "files-server");
        assert_eq!(slug("!!"), "");
    }

    #[test]
    fn validation_and_secrets_never_in_the_list() {
        let dir = tempfile::tempdir().unwrap();
        let ep = Endpoints::open(dir.path());
        let m = McpServers::open(dir.path());
        assert!(m.add(&ep, web("http://example.com/mcp"), McpSecrets::default()).is_err());
        assert!(m.add(&ep, web("https://tok@example.com/mcp"), McpSecrets::default()).is_err());
        assert!(m.add(&ep, stdio("!!"), McpSecrets::default()).is_err());
        let secrets = McpSecrets { env: [("API_KEY".into(), "sk-super-secret".into())].into(), ..Default::default() };
        let v = m.add(&ep, stdio("Files"), secrets.clone()).unwrap();
        assert_eq!(v.env_names, ["API_KEY"]);
        assert!(v.warning.unwrap().contains("/bin/true"));
        assert!(m.add(&ep, stdio("files"), McpSecrets::default()).is_err(), "same slug");
        let on_disk = std::fs::read_to_string(dir.path().join(MCP_FILE)).unwrap();
        assert!(!on_disk.contains("sk-super-secret"));
        let sealed = std::fs::read(dir.path().join(crate::endpoints::SECRETS_FILE)).unwrap();
        assert!(!String::from_utf8_lossy(&sealed).contains("sk-super-secret"));
        assert_eq!(ep.get_secret(&secret_name(&v.id, "env", "API_KEY")).unwrap().as_deref(), Some("sk-super-secret"));
        assert!(!format!("{secrets:?}").contains("sk-super"));

        // Edit keeps secrets unless replaced; delete removes them.
        let mut input = stdio("Files");
        input.enabled = false;
        m.edit(&ep, &v.id, input, None).unwrap();
        assert_eq!(ep.get_secret(&secret_name(&v.id, "env", "API_KEY")).unwrap().as_deref(), Some("sk-super-secret"));
        assert!(m.applying(None, None).is_empty(), "disabled");
        m.edit(&ep, &v.id, stdio("Files"), Some(McpSecrets::default())).unwrap();
        assert_eq!(ep.get_secret(&secret_name(&v.id, "env", "API_KEY")).unwrap(), None);
        m.delete(&ep, &v.id).unwrap();
        assert!(m.list().is_empty());
    }

    #[test]
    fn scopes_and_always_allow() {
        let dir = tempfile::tempdir().unwrap();
        let ep = Endpoints::open(dir.path());
        let m = McpServers::open(dir.path());
        let mut a = stdio("A");
        a.scope = McpScope::Folder("/src/app".into());
        let a = m.add(&ep, a, McpSecrets::default()).unwrap();
        let mut b = stdio("B");
        b.scope = McpScope::Session("c1".into());
        m.add(&ep, b, McpSecrets::default()).unwrap();
        m.add(&ep, stdio("C"), McpSecrets::default()).unwrap();
        let names = |f, s| m.applying(f, s).into_iter().map(|s| s.name).collect::<Vec<_>>();
        assert_eq!(names(None, None), ["C"]);
        assert_eq!(names(Some("/src/app/sub"), None), ["A", "C"]);
        assert_eq!(names(Some("/src/other"), Some("c1")), ["B", "C"]);
        m.set_always_allow(&a.id, "a.read", true).unwrap();
        assert!(m.always_allowed().contains("a.read"));
        m.set_always_allow(&a.id, "a.read", false).unwrap();
        assert!(m.always_allowed().is_empty());
        // Survives a reopen.
        assert_eq!(McpServers::open(dir.path()).list().len(), 3);
    }
}
