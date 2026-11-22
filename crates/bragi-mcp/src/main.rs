//! `bragi-mcp`: the MCP server on its own (the `bragi` desktop binary also runs it as `bragi mcp`).

fn main() -> std::process::ExitCode {
    bragi_mcp::cli::main(std::env::args().skip(1))
}
