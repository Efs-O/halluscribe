// HalluScribe - `halluscribe-mcp`: a stdio JSON-RPC MCP server exposing the
// existing archive read capabilities (search, read, profile, digest) to any
// MCP client. No Tauri runtime - it reads the archive directory directly.
// Read-only by design: no write/redact/delete tools. See
// docs/internal/PERSONA_PROTOCOL_PLAN.md Phase 3.

use app_lib::mcp::{resolve_archive_dir, HalluscribeServer};
use rmcp::transport::stdio;
use rmcp::ServiceExt;

#[tokio::main]
async fn main() {
    let archive_dir = match resolve_archive_dir(std::env::var("HALLUSCRIBE_DIR").ok()) {
        Ok(dir) => dir,
        Err(error) => {
            eprintln!("halluscribe-mcp: {error}");
            std::process::exit(1);
        }
    };

    let server = HalluscribeServer::new(archive_dir);
    let service = match server.serve(stdio()).await {
        Ok(service) => service,
        Err(error) => {
            eprintln!("halluscribe-mcp: failed to start stdio transport: {error}");
            std::process::exit(1);
        }
    };

    if let Err(error) = service.waiting().await {
        eprintln!("halluscribe-mcp: server error: {error}");
        std::process::exit(1);
    }
}
