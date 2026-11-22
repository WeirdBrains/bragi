//! `bragi-tui mcp-servers`: the outside MCP servers Bragi's chats can use (LF-37), for scripts and
//! SSH sessions. It is `Core`'s MCP API; secrets are never typed on the command line: `--env NAME`,
//! `--bearer-env VAR` and `--header NAME=VAR` read the value from the environment variable, at
//! the moment of adding, and it goes straight to the sealed store (or the OS keychain).

use std::collections::BTreeMap;

use bragi_core::{Core, McpScope, McpSecrets, McpServerInput, McpServerView, McpTransport};

pub const USAGE: &str = "\
  bragi-tui mcp-servers list
  bragi-tui mcp-servers add NAME (--command PROGRAM [--arg A]... [--cwd DIR] [--env VAR]...
                                  | --url https://HOST/mcp [--bearer-env VAR] [--header NAME=VAR]...)
                                  [--scope all|folder:PATH|session:ID] [--timeout SECS] [--yes]
  bragi-tui mcp-servers remove|enable|disable|test NAME
  bragi-tui mcp-servers revoke NAME TOOL           take back \"always allow\" for NAME.TOOL";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sub {
    List,
    Add(Box<AddArgs>),
    Remove(String),
    Enable(String),
    Disable(String),
    Test(String),
    Revoke(String, String),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AddArgs {
    pub name: String,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub env: Vec<String>,
    pub url: Option<String>,
    pub bearer_env: Option<String>,
    pub headers: Vec<String>,
    pub scope: Option<String>,
    pub timeout: Option<u64>,
    pub yes: bool,
}

/// Parses what follows `mcp-servers`.
pub fn parse(args: Vec<String>) -> Result<Sub, String> {
    let mut it = args.into_iter();
    let sub = it.next().ok_or("mcp-servers needs list, add, remove, enable, disable, test or revoke")?;
    let rest: Vec<String> = it.collect();
    let one = |what: &str| -> Result<String, String> {
        match rest.as_slice() {
            [n] if !n.starts_with("--") => Ok(n.clone()),
            _ => Err(format!("{what} takes one server name")),
        }
    };
    match sub.as_str() {
        "list" if rest.is_empty() => Ok(Sub::List),
        "remove" => one("remove").map(Sub::Remove),
        "enable" => one("enable").map(Sub::Enable),
        "disable" => one("disable").map(Sub::Disable),
        "test" => one("test").map(Sub::Test),
        "revoke" => match rest.as_slice() {
            [n, t] => Ok(Sub::Revoke(n.clone(), t.clone())),
            _ => Err("revoke takes a server name and a tool name".into()),
        },
        "add" => {
            let mut a = AddArgs::default();
            let mut it = rest.into_iter();
            while let Some(w) = it.next() {
                let mut value = |n: &str| it.next().ok_or_else(|| format!("{n} needs a value"));
                match w.as_str() {
                    "--command" => a.command = Some(value("--command")?),
                    "--arg" => a.args.push(value("--arg")?),
                    "--cwd" => a.cwd = Some(value("--cwd")?),
                    "--env" => a.env.push(value("--env")?),
                    "--url" => a.url = Some(value("--url")?),
                    "--bearer-env" => a.bearer_env = Some(value("--bearer-env")?),
                    "--header" => a.headers.push(value("--header")?),
                    "--scope" => a.scope = Some(value("--scope")?),
                    "--timeout" => a.timeout = Some(value("--timeout")?.parse().map_err(|_| "--timeout needs a number of seconds")?),
                    "--yes" | "-y" => a.yes = true,
                    s if s.starts_with("--") => return Err(format!("unknown option {s}")),
                    _ if a.name.is_empty() => a.name = w,
                    _ => return Err("give one server name".into()),
                }
            }
            if a.name.is_empty() {
                return Err("add needs a server name".into());
            }
            if a.command.is_some() == a.url.is_some() {
                return Err("give either --command (a local server) or --url (a web server)".into());
            }
            Ok(Sub::Add(Box::new(a)))
        }
        other => Err(format!("unknown mcp-servers command {other}")),
    }
}

fn scope(s: Option<&str>) -> Result<McpScope, String> {
    match s {
        None | Some("all") => Ok(McpScope::All),
        Some(x) => match x.split_once(':') {
            Some(("folder", p)) if !p.is_empty() => Ok(McpScope::Folder(p.to_string())),
            Some(("session", id)) if !id.is_empty() => Ok(McpScope::Session(id.to_string())),
            _ => Err("--scope is all, folder:PATH or session:ID".into()),
        },
    }
}

fn from_env(var: &str) -> Result<String, String> {
    std::env::var(var).map_err(|_| format!("environment variable {var} is not set"))
}

fn find(core: &Core, name: &str) -> Result<McpServerView, String> {
    let slug = bragi_core::mcp_servers::slug(name);
    core.mcp_servers()
        .into_iter()
        .find(|s| s.id == name || bragi_core::mcp_servers::slug(&s.name) == slug)
        .ok_or_else(|| format!("no MCP server named {name}"))
}

/// Builds the input and secrets for `add` (reads the environment for secret values).
pub fn add_input(a: &AddArgs) -> Result<(McpServerInput, McpSecrets), String> {
    let mut secrets = McpSecrets::default();
    for v in &a.env {
        secrets.env.insert(v.clone(), from_env(v)?);
    }
    if let Some(v) = &a.bearer_env {
        secrets.bearer = Some(from_env(v)?);
    }
    let mut headers = BTreeMap::new();
    for h in &a.headers {
        let (name, var) = h.split_once('=').ok_or("--header is NAME=VAR (VAR is an environment variable holding the value)")?;
        headers.insert(name.to_string(), from_env(var)?);
    }
    secrets.headers = headers;
    let input = McpServerInput {
        name: a.name.clone(),
        transport: if a.url.is_some() { McpTransport::Http } else { McpTransport::Stdio },
        enabled: true,
        command: a.command.clone(),
        args: a.args.clone(),
        cwd: a.cwd.clone(),
        url: a.url.clone(),
        scope: scope(a.scope.as_deref())?,
        timeout_secs: a.timeout,
    };
    Ok((input, secrets))
}

pub fn run(core: &Core, sub: Sub, confirm: &dyn Fn(&str) -> bool) -> Result<(), String> {
    let run_async = |f: std::pin::Pin<Box<dyn std::future::Future<Output = bragi_core::McpTestReport> + Send>>| core.handle().block_on(f);
    match sub {
        Sub::List => {
            let list = core.mcp_servers();
            if list.is_empty() {
                println!("No MCP servers. Add one with `bragi-tui mcp-servers add`.");
            }
            for s in list {
                let scope = match &s.scope {
                    McpScope::All => "all chats".to_string(),
                    McpScope::Folder(p) => format!("folder {p}"),
                    McpScope::Session(id) => format!("session {id}"),
                };
                let secrets = s.env_names.len() + s.header_names.len() + usize::from(s.has_bearer);
                println!(
                    "{}\t{}\t{}\t{scope}\t({secrets} secret{} set)",
                    s.name,
                    if s.enabled { "on" } else { "off" },
                    s.target,
                    if secrets == 1 { "" } else { "s" }
                );
                for t in &s.always_allow {
                    println!("  always allowed: {t}");
                }
            }
            Ok(())
        }
        Sub::Add(a) => {
            let (input, secrets) = add_input(&a)?;
            if input.transport == McpTransport::Stdio && !a.yes {
                let c = StdioPreview(&input);
                if !confirm(&format!("{}\nAdd it?", c.warning())) {
                    println!("Not added.");
                    return Ok(());
                }
            }
            let v = core.add_mcp_server(input, secrets).map_err(|e| e.message)?;
            println!("Added {}. Every tool call will ask first.", v.name);
            Ok(())
        }
        Sub::Remove(n) => {
            let s = find(core, &n)?;
            core.delete_mcp_server(&s.id).map_err(|e| e.message)?;
            println!("Removed {}.", s.name);
            Ok(())
        }
        Sub::Enable(n) => toggle(core, &n, true),
        Sub::Disable(n) => toggle(core, &n, false),
        Sub::Test(n) => {
            let s = find(core, &n)?;
            let c = core.clone();
            let id = s.id.clone();
            let r = run_async(Box::pin(async move { c.test_mcp_server(&id).await }));
            println!("{}", r.message);
            for t in &r.tools {
                println!("  {t}");
            }
            if r.ok { Ok(()) } else { Err(format!("{} didn't work", s.name)) }
        }
        Sub::Revoke(n, tool) => {
            let s = find(core, &n)?;
            core.set_mcp_tool_always_allowed(&s.id, &tool, false).map_err(|e| e.message)?;
            println!("{tool} asks again.");
            Ok(())
        }
    }
}

fn toggle(core: &Core, n: &str, on: bool) -> Result<(), String> {
    let s = find(core, n)?;
    core.set_mcp_server_enabled(&s.id, on).map_err(|e| e.message)?;
    println!("{} is {}.", s.name, if on { "on" } else { "off" });
    Ok(())
}

/// The warning for a command server about to be added.
struct StdioPreview<'a>(&'a McpServerInput);

impl StdioPreview<'_> {
    fn warning(&self) -> String {
        let cmd = self.0.command.clone().unwrap_or_default();
        format!("This starts `{} {}` on this computer, with your permissions, whenever a chat uses it.", cmd, self.0.args.join(" "))
            .replace("  ", " ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parsing() {
        assert_eq!(parse(v(&["list"])).unwrap(), Sub::List);
        assert_eq!(parse(v(&["remove", "files"])).unwrap(), Sub::Remove("files".into()));
        assert!(parse(v(&["remove"])).is_err());
        assert!(parse(v(&["add", "x"])).is_err(), "needs a command or a url");
        assert!(parse(v(&["add", "x", "--command", "a", "--url", "https://h"])).is_err());
        let Sub::Add(a) = parse(v(&["add", "x", "--command", "npx", "--arg", "-y", "--arg", "srv", "--env", "TOKEN", "--yes"])).unwrap() else {
            panic!()
        };
        assert_eq!((a.args.as_slice(), a.env.as_slice(), a.yes), (&v(&["-y", "srv"])[..], &v(&["TOKEN"])[..], true));
        assert_eq!(scope(Some("folder:/src")).unwrap(), McpScope::Folder("/src".into()));
        assert!(scope(Some("nope")).is_err());
        assert!(parse(v(&["add", "x", "--url", "https://h", "--bogus"])).is_err());
    }
}
