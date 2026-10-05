#![forbid(unsafe_code)]
#![deny(clippy::all)]
#![deny(clippy::unwrap_used)]

//! Prints the generated OpenAPI document to stdout.
//!
//! Used by `just openapi` to record the §27-17 evidence that the pinned
//! `utoipa` emits `openapi: 3.1.0`, and by the CI drift check that will diff
//! the committed document against the generated one.

use std::io::Write as _;

use utoipa::OpenApi as _;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let doc = vf_api::openapi::ApiDoc::openapi().to_pretty_json()?;
    let mut out = std::io::stdout().lock();
    out.write_all(doc.as_bytes())?;
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}
