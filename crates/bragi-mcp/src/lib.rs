//! `bragi mcp`: Bragi as a [Model Context Protocol](https://modelcontextprotocol.io) server, so
//! tools such as Cursor, Claude Desktop and VS Code can chat through Bragi, read and file its
//! sessions, and see its endpoints and models.
//!
//! - Transports: stdio (the default; the client starts `bragi mcp`), or stateless streamable HTTP
//!   on a loopback address with `--http`, where clients send Bragi's local API token as a bearer
//!   token (`bragi mcp token`; the same token and file `bragi serve` uses, from `bragi_serve`).
//! - Tools: see [`tools::READ_TOOLS`], [`code::CODE_TOOLS`], and the tools offered only with
//!   `--allow-writes` ([`tools::WRITE_TOOLS`], [`code::CODE_WRITE_TOOLS`]) or `--allow-deletes`
//!   ([`tools::DELETE_TOOLS`]).
//! - Resources: `bragi://status`, `bragi://sessions`, `bragi://sessions/{id}` (Markdown) and
//!   `bragi://sessions/{id}/files/{name}`.
//!
//! - Prompts: [`prompts::PROMPTS`] (review, rewrite, explain, summarize or continue a session).
//!
//! Everything goes through [`bragi_core::Core`]; the protocol layer (JSON-RPC, stdio, the HTTP
//! guard, resources, media content) is `weirdnet-mcp` 0.6.

pub mod cli;
pub mod code;
mod collect;
pub mod prompts;
pub mod tools;
pub mod video;

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use weirdnet_mcp::Server;
use weirdnet_mcp::http::{Auth, Guard};

pub use tools::{BragiTools, Options, server};

/// Where the streamable HTTP endpoint lives.
pub const HTTP_PATH: &str = "/mcp";

/// The default `--http` address.
pub const DEFAULT_HTTP: &str = "127.0.0.1:4878";

/// Serves newline-delimited JSON-RPC on `input` and `output` until `input` ends and every request
/// has been answered. Requests run concurrently, so a long chat does not hold up a ping.
pub async fn serve_stdio<R, W>(server: Arc<Server<BragiTools>>, input: R, output: W) -> Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    weirdnet_mcp::stdio::serve(server, input, output).await.context("serving the MCP client")
}

/// Parses `--http ADDR` and refuses anything but a loopback address.
pub fn loopback_addr(s: &str) -> Result<SocketAddr> {
    let s = s.trim();
    let addr: SocketAddr = match s.strip_prefix("localhost:") {
        Some(port) => SocketAddr::from(([127, 0, 0, 1], port.parse().with_context(|| format!("bad port in {s}"))?)),
        None => s.parse().with_context(|| format!("`{s}` is not an address like {DEFAULT_HTTP}"))?,
    };
    if !addr.ip().is_loopback() {
        bail!("the MCP server only listens on loopback addresses (127.0.0.1 or ::1), not {}", addr.ip());
    }
    Ok(addr)
}

/// Serves stateless streamable HTTP at [`HTTP_PATH`] on `listener`, which must be bound to a
/// loopback address, until `shutdown`. `weirdnet-mcp`'s guard checks `Host`, `Origin` and the
/// bearer token (in constant time); the loopback bind check is ours.
pub async fn serve_http(
    listener: tokio::net::TcpListener,
    server: Arc<Server<BragiTools>>,
    token: String,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<()> {
    if !listener.local_addr()?.ip().is_loopback() {
        bail!("refusing to serve MCP on {}: loopback only", listener.local_addr()?);
    }
    let app = weirdnet_mcp::http::router(HTTP_PATH, server, Guard::loopback(Auth::token(token)));
    axum::serve(listener, app).with_graceful_shutdown(shutdown).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_loopback_addresses() {
        assert_eq!(loopback_addr("127.0.0.1:4878").unwrap().port(), 4878);
        assert!(loopback_addr("[::1]:9").is_ok());
        assert_eq!(loopback_addr("localhost:7").unwrap(), "127.0.0.1:7".parse().unwrap());
        assert!(loopback_addr("0.0.0.0:4878").is_err());
        assert!(loopback_addr("192.0.2.1:4878").is_err());
        assert!(loopback_addr("nonsense").is_err());
    }
}
