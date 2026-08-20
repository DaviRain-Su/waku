//! proofship-pf-mcp — ProofForge gate exposed as a stdio MCP server.
//!
//! Maintained in the ProofShip repo (replaces proof_forge's python MCP-V0
//! script). Built on the official `rmcp` SDK, speaking the current MCP
//! spec with backwards-compatible version negotiation, so any MCP client
//! (Claude Code, Codex, ACP agents spawned by the ProofShip engine) can
//! attach it with just `{ "command": "proofship-pf-mcp" }`.

mod cli;
mod server;

use rmcp::ServiceExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--self-check") {
        match cli::find_cli() {
            Ok(path) => {
                println!("ok: proof-forge-next at {}", path.display());
                return Ok(());
            }
            Err(e) => {
                eprintln!("missing: {e}");
                std::process::exit(1);
            }
        }
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "proofship-pf-mcp {} — ProofForge gate as a stdio MCP server\n\
             Tools: pf_doctor pf_check pf_build pf_artifacts\n\
             Env: PROOF_FORGE_CLI (cli path) | PROOF_FORGE_ROOT (checkout with .lake build)\n\
             Flags: --self-check (verify CLI resolution and exit)",
            env!("CARGO_PKG_VERSION")
        );
        return Ok(());
    }

    let service = server::PfGate::new()
        .serve(rmcp::transport::stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}
