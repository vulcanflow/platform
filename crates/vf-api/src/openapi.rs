//! Code-first OpenAPI document (architecture §A5, TDD §27-17).
//!
//! F1 publishes one hello-world operation. Its only job is to prove that the
//! pinned `utoipa` emits `openapi: 3.1.0` and that the generated document can
//! be dumped reproducibly; `cargo run --bin vf-openapi` (or `just openapi`)
//! prints it. Task A1 replaces this with the real surface of §A3.8, including
//! RFC 9457 Problem Details responses.

use utoipa::OpenApi;

/// Liveness probe. Returns as soon as the process can serve a request; it
/// asserts nothing about Postgres, the wake bus or the artifact store.
#[utoipa::path(
    get,
    path = "/healthz",
    tag = "meta",
    responses((status = 200, description = "The process is serving requests.", body = String))
)]
pub async fn healthz() -> &'static str {
    "ok"
}

/// The generated document. `paths` grows as task A1 lands the real surface.
#[derive(OpenApi)]
#[openapi(
    paths(healthz),
    tags((name = "meta", description = "Liveness and build metadata."))
)]
pub struct ApiDoc;
