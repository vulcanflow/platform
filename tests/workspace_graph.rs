//! T9 (VFL-39): workspace rules.
//!
//! Mechanical guards over the F1 scaffold decisions (VFL-9) for architecture
//! document §A1.3 (crate inventory), §A1.4 (crate dependency graph) and §A5
//! (pinned toolchain and crate set), plus the lane rules in README.md (no
//! `#[cfg(test)]` under `crates/*/src`).
//!
//! Independent of the `platform` workspace by design (see `tests/Cargo.toml`):
//! run with `cargo test --manifest-path tests/Cargo.toml`. Every assertion here
//! reads the `platform` workspace as data (its committed files, and the output of
//! `cargo metadata`, which resolves the dependency graph without compiling any
//! crate) and never builds or executes `platform` code.
//!
//! **2026-10-06, VFL-74.** Written against
//! `eab1bc56eb5d221b0ad499906a387640e2f35e31` on `jorge/f1-workspace-scaffold`,
//! the current F1 candidate: five doc/config commits on top of `88a2ef4` that
//! fixed Opus Reviewer's four MEDIUM findings and six of its eight LOW
//! findings (VFL-68; full remediation record on VFL-70's evidence document).
//! None of those five commits moves a pin value, a feature set, or a
//! `Cargo.lock` resolution, apart from one added `aws-lc-rs`/`aws-lc-sys` ban
//! pair in `deny.toml` and one added `rustls` workspace dependency for
//! `vf-hook-notify`. Every test below that passed against `88a2ef4` still
//! passes against `eab1bc5` unchanged; the only changes this revision makes
//! to the pack itself are the four fixes below, taken from VFL-68's five LOW
//! coverage notes addressed to the test revision (comment `823ef786`, "On the
//! test revision"), not from anything the candidate changed.
//!
//! Coverage notes taken this revision:
//!
//! 1. This doc comment was stale (it named `224722d` as "the current F1
//!    candidate" and was written against `20736f3`; `88a2ef4` was never
//!    mentioned). Fixed by this rewrite.
//! 2. `parse_workspace_dependency_pins` stripped a leading `=` without
//!    asserting it was there, so a non-exact requirement such as `tokio =
//!    "1.53.2"` would have satisfied `workspace_dependency_pins_match_a5_series`
//!    silently — the exact-pin property §A5 requires, and the one this test's
//!    own name claims to check, was unasserted. The parser now returns the
//!    requirement string as written, and the test asserts the leading `=`
//!    before stripping it.
//! 3. Crate inventory and `lib`/`bin` kind were never checked against §A1.3.
//!    New test `crate_inventory_matches_a1_3`, against the ratified table
//!    committed in README.md ("Crate inventory (architecture §A1.3)").
//! 4. `every_crate_root_forbids_unsafe_code` skipped `src/bin/*.rs`, so
//!    `vf-api/src/bin/openapi.rs` went unchecked (it does carry the
//!    attribute — this was a coverage gap, not a live failure). Arbiter
//!    raised the same gap independently on VFL-67. The test now also scans
//!    every `.rs` file directly under each crate's `src/bin/`.
//!
//! Not taken this revision: coverage note 5, asking for assertions over
//! `deny.toml`'s advisories/sources/bans rules, its (now-removed) `ring`
//! clarify table, and `Cargo.toml`<->`Cargo.lock` consistency. Recorded as a
//! residual LOW rather than fixed, for Cortana to weigh in on: the lock-
//! consistency half is already covered as a side effect of `cargo metadata
//! --locked --all-features` in `cargo_metadata()` below, which fails closed
//! if `Cargo.lock` does not satisfy `Cargo.toml`; the advisories/sources/bans/
//! clarify half is squarely cargo-deny's own job when `just deny` runs `cargo
//! deny check` against this same committed config, and a second, hand-rolled
//! Rust re-implementation of RustSec's advisory feed or cargo-deny's ban
//! resolution risks a divergent oracle, not an independent check — it could
//! disagree with `cargo deny check` without either reading being wrong.
//!
//! Several assertions here are written against the architecture document and
//! Cortana's recorded VFL-9 rulings, not against whatever the candidate
//! happens to contain.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use serde_json::Value;

// ---------------------------------------------------------------------------
// Shared file-system and `cargo metadata` helpers
// ---------------------------------------------------------------------------

/// Absolute path to the `platform` workspace root (the parent of this `tests/`
/// lane directory).
fn platform_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("tests/ has a parent directory")
        .to_path_buf()
}

fn read_platform_file(relative: &str) -> String {
    let path = platform_root().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

/// Runs `cargo metadata` against the `platform` workspace root and returns the
/// parsed document, cached for the lifetime of the test binary so the six tests
/// that need the dependency graph do not each pay for their own subprocess.
fn cargo_metadata() -> &'static Value {
    static METADATA: OnceLock<Value> = OnceLock::new();
    METADATA.get_or_init(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
        let manifest = platform_root().join("Cargo.toml");
        let output = Command::new(cargo)
            .arg("metadata")
            .arg("--format-version=1")
            .arg("--locked")
            .arg("--all-features")
            .arg("--manifest-path")
            .arg(&manifest)
            .output()
            .expect("invoking `cargo metadata` on the platform workspace");
        assert!(
            output.status.success(),
            "cargo metadata failed (status {:?}):\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("cargo metadata emits valid JSON")
    })
}

fn extract_quoted(s: &str) -> Option<String> {
    let start = s.find('"')?;
    let after_start = &s[start + 1..];
    let end = after_start.find('"')?;
    Some(after_start[..end].to_string())
}

/// Extracts a `key = [ ... ]` TOML array of strings, tolerating the array
/// spanning multiple lines. Not a general TOML parser: good enough for the
/// specific files this suite reads.
fn extract_toml_string_array(toml_text: &str, key: &str) -> Option<Vec<String>> {
    let needle = format!("{key} = [");
    let start = toml_text.find(needle.as_str())? + needle.len();
    let end = toml_text[start..].find(']')? + start;
    let body = &toml_text[start..end];
    Some(body.split(',').filter_map(extract_quoted).collect())
}

// ---------------------------------------------------------------------------
// §A1.4 crate dependency graph
// ---------------------------------------------------------------------------

struct Graph {
    id_to_name: HashMap<String, String>,
    id_to_deps: HashMap<String, Vec<String>>,
    workspace_members: HashSet<String>,
    bin_target_ids: HashSet<String>,
}

fn build_graph(metadata: &Value) -> Graph {
    let packages = metadata["packages"].as_array().expect("packages array");

    let mut id_to_name = HashMap::new();
    let mut bin_target_ids = HashSet::new();
    for p in packages {
        let id = p["id"].as_str().expect("package id").to_string();
        let name = p["name"].as_str().expect("package name").to_string();
        let has_bin = p["targets"]
            .as_array()
            .expect("targets array")
            .iter()
            .any(|t| {
                t["kind"]
                    .as_array()
                    .expect("target kind array")
                    .iter()
                    .any(|k| k.as_str() == Some("bin"))
            });
        if has_bin {
            bin_target_ids.insert(id.clone());
        }
        id_to_name.insert(id, name);
    }

    let workspace_members: HashSet<String> = metadata["workspace_members"]
        .as_array()
        .expect("workspace_members array")
        .iter()
        .map(|v| v.as_str().expect("workspace member id").to_string())
        .collect();

    let mut id_to_deps = HashMap::new();
    let nodes = metadata["resolve"]["nodes"]
        .as_array()
        .expect("resolve.nodes array (needs a resolvable, committed Cargo.lock)");
    for n in nodes {
        let id = n["id"].as_str().expect("node id").to_string();
        let deps = n["dependencies"]
            .as_array()
            .expect("node dependencies array")
            .iter()
            .map(|d| d.as_str().expect("dependency id").to_string())
            .collect();
        id_to_deps.insert(id, deps);
    }

    Graph {
        id_to_name,
        id_to_deps,
        workspace_members,
        bin_target_ids,
    }
}

impl Graph {
    /// The id of the workspace member named `name`. Panics if no such member
    /// exists, which is itself a useful failure: it means the crate list no
    /// longer matches §A1.3.
    fn id_of(&self, name: &str) -> String {
        self.id_to_name
            .iter()
            .find(|(id, n)| n.as_str() == name && self.workspace_members.contains(id.as_str()))
            .map(|(id, _)| id.clone())
            .unwrap_or_else(|| panic!("no workspace member named `{name}` in cargo metadata output"))
    }

    fn name_of(&self, id: &str) -> String {
        self.id_to_name
            .get(id)
            .cloned()
            .unwrap_or_else(|| panic!("no package name for id `{id}`"))
    }

    fn direct_dep_names(&self, id: &str) -> Vec<String> {
        self.id_to_deps
            .get(id)
            .map(|deps| deps.iter().map(|d| self.name_of(d)).collect())
            .unwrap_or_default()
    }

    /// All package names reachable from `start_id` by following resolved
    /// dependency edges, excluding `start_id` itself.
    fn reachable_names(&self, start_id: &str) -> HashSet<String> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut stack = vec![start_id.to_string()];
        while let Some(cur) = stack.pop() {
            if let Some(deps) = self.id_to_deps.get(&cur) {
                for d in deps {
                    if seen.insert(d.clone()) {
                        stack.push(d.clone());
                    }
                }
            }
        }
        seen.into_iter().map(|id| self.name_of(&id)).collect()
    }
}

const NO_IO_CRATES: [&str; 4] = ["tokio", "sqlx", "reqwest", "kube"];

#[test]
fn vf_core_depends_on_no_io_crate() {
    let graph = build_graph(cargo_metadata());
    let id = graph.id_of("vf-core");
    let reachable = graph.reachable_names(&id);
    for io_crate in NO_IO_CRATES {
        assert!(
            !reachable.contains(io_crate),
            "§A1.4: vf-core must depend on no I/O crate, but its dependency \
             closure reaches `{io_crate}`"
        );
    }
}

#[test]
fn vf_graph_depends_on_no_io_crate() {
    let graph = build_graph(cargo_metadata());
    let id = graph.id_of("vf-graph");
    let reachable = graph.reachable_names(&id);
    for io_crate in NO_IO_CRATES {
        assert!(
            !reachable.contains(io_crate),
            "§A1.4: vf-graph must depend on no I/O crate, but its dependency \
             closure reaches `{io_crate}`"
        );
    }
}

#[test]
fn vf_graph_does_not_depend_on_vf_core() {
    let graph = build_graph(cargo_metadata());
    let id = graph.id_of("vf-graph");
    let direct = graph.direct_dep_names(&id);
    assert!(
        !direct.iter().any(|n| n == "vf-core"),
        "§A1.4: vf-graph must not depend on vf-core (it exports its own small \
         types so the wasm32 build carries nothing it does not need); direct \
         deps were {direct:?}"
    );
}

#[test]
fn no_binary_crate_depends_on_another_binary_crate() {
    let graph = build_graph(cargo_metadata());

    let binary_ids: Vec<String> = graph
        .bin_target_ids
        .iter()
        .filter(|id| graph.workspace_members.contains(id.as_str()))
        .cloned()
        .collect();
    let binary_names: HashSet<String> = binary_ids.iter().map(|id| graph.name_of(id)).collect();

    for bin_id in &binary_ids {
        let this_name = graph.name_of(bin_id);
        let direct = graph.direct_dep_names(bin_id);
        for dep_name in &direct {
            assert!(
                dep_name == &this_name || !binary_names.contains(dep_name),
                "§A1.4: no binary crate depends on another binary crate except \
                 through its lib target; `{this_name}` directly depends on \
                 `{dep_name}`, which carries a [[bin]] target"
            );
        }
    }
}

#[test]
fn future_ai_crate_may_not_depend_on_vf_api() {
    // §18.6, recorded now, enforced when the crate exists. No crate named
    // `vf-ai*` is in the §A1.3 inventory yet, so this passes vacuously today and
    // starts enforcing the moment such a crate is added to the workspace.
    let graph = build_graph(cargo_metadata());
    let offenders: Vec<String> = graph
        .workspace_members
        .iter()
        .map(|id| graph.name_of(id))
        .filter(|name| name.starts_with("vf-ai"))
        .collect();
    for name in offenders {
        let id = graph.id_of(&name);
        let direct = graph.direct_dep_names(&id);
        assert!(
            !direct.iter().any(|n| n == "vf-api"),
            "§18.6: `{name}` may not depend on vf-api, directly or through the \
             dispatcher"
        );
    }
}

// ---------------------------------------------------------------------------
// §A1.3 crate inventory and lib/bin kind
// ---------------------------------------------------------------------------

/// (crate name, kind) exactly as README.md's own "Crate inventory
/// (architecture §A1.3)" table states it. Not parsed from README.md: the
/// table's prose Responsibility column has no fixed format to scan reliably,
/// and the two facts that matter here — the crate list and its lib/bin kind —
/// are cheap to state as a flat, reviewable constant instead.
const A1_3_INVENTORY: &[(&str, &str)] = &[
    ("vf-core", "lib"),
    ("vf-db", "lib"),
    ("vf-graph", "lib"),
    ("vf-translator", "lib"),
    ("vf-authz", "lib"),
    ("vf-meter", "lib"),
    ("vf-remediation", "lib"),
    ("vf-api", "lib + bin"),
    ("vf-operator", "lib + bin"),
    ("vf-admission", "lib + bin"),
    ("vf-ingest", "lib + bin"),
    ("vf-hook-notify", "bin"),
    ("vf-scanner-adapter", "bin"),
    ("vf-report", "lib + bin"),
    ("vf-abuse", "bin"),
    ("vf-testkit", "lib"),
];

/// Classifies a crate directory's kind from its `src/` layout: `lib` for
/// `src/lib.rs`, `bin` for `src/main.rs` or any `.rs` file directly under
/// `src/bin/`, `lib + bin` for both.
fn actual_crate_kind(crate_dir: &Path) -> &'static str {
    let has_lib = crate_dir.join("src/lib.rs").exists();
    let has_main = crate_dir.join("src/main.rs").exists();
    let has_extra_bin = std::fs::read_dir(crate_dir.join("src/bin"))
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .any(|e| e.path().extension().and_then(|ext| ext.to_str()) == Some("rs"))
        })
        .unwrap_or(false);
    match (has_lib, has_main || has_extra_bin) {
        (true, true) => "lib + bin",
        (true, false) => "lib",
        (false, true) => "bin",
        (false, false) => panic!(
            "{} has neither src/lib.rs, src/main.rs nor a src/bin/*.rs",
            crate_dir.display()
        ),
    }
}

#[test]
fn crate_inventory_matches_a1_3() {
    let mut actual: Vec<String> = crate_dirs()
        .iter()
        .map(|d| d.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    actual.sort();

    let mut expected: Vec<String> = A1_3_INVENTORY.iter().map(|(name, _)| name.to_string()).collect();
    expected.sort();

    assert_eq!(
        actual, expected,
        "§A1.3: crates/ must contain exactly the sixteen ratified crates, no \
         more and no fewer"
    );

    for (name, kind) in A1_3_INVENTORY {
        let crate_dir = platform_root().join("crates").join(name);
        let actual_kind = actual_crate_kind(&crate_dir);
        assert_eq!(
            actual_kind, *kind,
            "§A1.3: `{name}` is listed as `{kind}`, but its src/ layout is `{actual_kind}`"
        );
    }
}

// ---------------------------------------------------------------------------
// §A5 pinned toolchain and crate set
// ---------------------------------------------------------------------------

/// (dependency name, required version series). A declared version matches when
/// its leading dot-separated components equal the series' components exactly —
/// e.g. series `"1.53"` matches `"1.53.2"` but not `"1.5.3"` or `"11.53.0"`.
/// Series, not exact patches: §A5 itself pins series (plus the toolchain and
/// `wasm-bindgen`, which it gives in full and are written here in full too); the
/// exact patch within a series is Jorge's resolution, not a reviewed decision.
const A5_SERIES: &[(&str, &str)] = &[
    ("tokio", "1.53"),
    ("tokio-util", "0.7"),
    ("futures", "0.3"),
    ("axum", "0.8"),
    ("axum-extra", "0.12"),
    ("tower", "0.5"),
    ("tower-http", "0.7"),
    ("utoipa", "6.0"),
    ("utoipa-axum", "0.3"),
    ("sqlx", "0.9"),
    ("pgvector", "0.4"),
    ("object_store", "0.14"),
    ("redis", "1.7"),
    ("governor", "0.10"),
    ("kube", "4.2"),
    ("kube-runtime", "4.2"),
    ("k8s-openapi", "0.28"),
    ("jsonwebtoken", "11"),
    ("openidconnect", "4"),
    ("rustls", "0.23"),
    ("hmac", "0.13"),
    ("sha2", "0.11"),
    ("rand", "0.10"),
    ("secrecy", "0.10"),
    ("hex", "0.4"),
    ("base64", "0.23"),
    ("serde", "1"),
    ("serde_json", "1"),
    ("schemars", "1.2"),
    ("jsonschema", "0.58"),
    ("serde_yaml_ng", "0.10"),
    ("idna", "1.1"),
    ("url", "2.5"),
    ("psl", "2.1"),
    ("hickory-resolver", "0.26"),
    ("reqwest", "0.13"),
    ("chrono", "0.4"),
    ("chrono-tz", "0.10"),
    ("croner", "4.0"),
    ("askama", "0.16"),
    ("ammonia", "4.2"),
    ("tracing", "0.1"),
    ("tracing-subscriber", "0.3"),
    ("opentelemetry", "0.33"),
    ("tracing-opentelemetry", "0.34"),
    ("prometheus-client", "0.25"),
    ("thiserror", "2"),
    ("uuid", "1.27"),
    ("ipnet", "2.12"),
    ("wasm-bindgen", "0.2.129"),
    ("wasm-bindgen-test", "0.3"),
    ("proptest", "1.11"),
    ("rstest", "0.27"),
    ("insta", "1.49"),
    ("testcontainers", "0.27"),
    ("testcontainers-modules", "0.15"),
    ("wiremock", "0.6"),
    ("tower-test", "0.4"),
    ("lettre", "0.11"),
    ("chromiumoxide", "0.9"),
    ("clickhouse", "0.15"),
];

fn series_matches(resolved: &str, series: &str) -> bool {
    let resolved_parts: Vec<&str> = resolved.split('.').collect();
    let series_parts: Vec<&str> = series.split('.').collect();
    if series_parts.len() > resolved_parts.len() {
        return false;
    }
    resolved_parts
        .iter()
        .zip(series_parts.iter())
        .all(|(r, s)| r == s)
}

/// Parses `name = "=X.Y.Z"` and `name = { version = "=X.Y.Z", ... }` entries out
/// of the `[workspace.dependencies]` table of the committed root `Cargo.toml`.
/// First-party `vf-*` path dependencies (no `version` key) are skipped. This is a
/// deliberately narrow line-oriented scan, not a TOML parser: every third-party
/// entry in that table is single-line in the committed manifest.
///
/// Returns the requirement string exactly as written, leading `=` and all:
/// callers that care whether a pin is exact (§A5 requires it) assert that
/// themselves, rather than this parser silently stripping it.
fn parse_workspace_dependency_pins() -> HashMap<String, String> {
    let cargo_toml = read_platform_file("Cargo.toml");
    let mut in_section = false;
    let mut pins = HashMap::new();
    for raw_line in cargo_toml.lines() {
        let line = raw_line.trim();
        if line == "[workspace.dependencies]" {
            in_section = true;
            continue;
        }
        if !in_section {
            continue;
        }
        if line.starts_with('[') {
            break; // the [workspace.dependencies] table has ended
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, rest)) = line.split_once('=') else {
            continue;
        };
        let name = key.trim().to_string();
        let rest = rest.trim();

        let quoted = if rest.starts_with('"') {
            extract_quoted(rest)
        } else if rest.starts_with('{') {
            if rest.contains("version") {
                let version_pos = rest.find("version").expect("checked contains above");
                extract_quoted(&rest[version_pos..])
            } else {
                None // path-only first-party dependency: not a §A5 pin
            }
        } else {
            None
        };

        let Some(quoted) = quoted else { continue };
        pins.insert(name, quoted);
    }
    pins
}

#[test]
fn workspace_dependency_pins_match_a5_series() {
    let pins = parse_workspace_dependency_pins();
    for (name, series) in A5_SERIES {
        let raw = pins.get(*name).unwrap_or_else(|| {
            panic!(
                "§A5: `{name}` is pinned in the architecture document but is \
                 missing from [workspace.dependencies]"
            )
        });
        assert!(
            raw.starts_with('='),
            "§A5: every pin in [workspace.dependencies] must be an exact `=` \
             requirement; `{name}` is `{raw}`, which Cargo resolves as a caret \
             range, not a pin"
        );
        let resolved = raw.trim_start_matches('=');
        assert!(
            series_matches(resolved, series),
            "§A5: `{name}` is pinned to series `{series}`, but \
             [workspace.dependencies] has `{resolved}`"
        );
    }
}

/// Extracts the full `name = { ... }` or `name = "..."` dependency entry out of
/// the committed root `Cargo.toml`, starting at the line beginning `name `/`name=`
/// and running until its braces balance (or just that line, for a bare string
/// pin). The entry may legally span multiple lines (e.g. a `features = [...]`
/// array one element per line), so a single-line scan is not sufficient here.
fn extract_workspace_dependency_entry(toml_text: &str, name: &str) -> String {
    let lines: Vec<&str> = toml_text.lines().collect();
    let start = lines
        .iter()
        .position(|l| {
            let t = l.trim_start();
            t.starts_with(&format!("{name} ")) || t.starts_with(&format!("{name}="))
        })
        .unwrap_or_else(|| panic!("no `{name} = ...` entry in [workspace.dependencies]"));

    let mut entry = String::new();
    let mut depth = 0i32;
    for line in &lines[start..] {
        entry.push_str(line);
        entry.push('\n');
        depth += line.matches('{').count() as i32 - line.matches('}').count() as i32;
        if depth <= 0 {
            break;
        }
    }
    entry
}

#[test]
fn reqwest_feature_is_rustls_no_provider() {
    // §A5 originally wrote this feature as `rustls-tls`, then Decision 2
    // ratified the reqwest-0.13 spelling `rustls`. Decision 4 (VFL-9 comment
    // `39d52469`, §A5 revision 4) supersedes that: the plain `rustls` feature
    // pulls in `aws-lc-rs` and defeats the single-ring-provider intent, so the
    // ratified spelling is now `rustls-no-provider`, with the `ring` provider
    // installed at startup by vf-db (§A6.1) instead of selected by the feature.
    let cargo_toml = read_platform_file("Cargo.toml");
    let reqwest_entry = extract_workspace_dependency_entry(&cargo_toml, "reqwest");
    assert!(
        reqwest_entry.contains("\"rustls-no-provider\""),
        "§A5 (ratified, revision 4): reqwest's TLS feature must be spelled \
         `rustls-no-provider`, found: {reqwest_entry}"
    );
    assert!(
        !reqwest_entry.contains("rustls-tls"),
        "§A5 (ratified): reqwest's TLS feature must not use the old `rustls-tls` \
         spelling, found: {reqwest_entry}"
    );
    assert!(
        reqwest_entry.contains("default-features = false"),
        "§A5: reqwest must set default-features = false (no native-tls), found: \
         {reqwest_entry}"
    );
}

#[test]
fn object_store_features_match_a5() {
    // Decision 4 (VFL-9 comment `39d52469`, §A5 revision 4): §A5 asks for the
    // `aws` feature, but `aws` pulls `aws-lc-rs` unconditionally and that
    // switches the whole workspace's rustls provider. The ratified deviation
    // is object_store's own ring-only recipe: `aws-base` + `reqwest` +
    // `ring`, with `fs` kept for the local-filesystem test double.
    let cargo_toml = read_platform_file("Cargo.toml");
    let entry = extract_workspace_dependency_entry(&cargo_toml, "object_store");
    for feature in ["fs", "aws-base", "reqwest", "ring"] {
        assert!(
            entry.contains(&format!("\"{feature}\"")),
            "§A5 (ratified, revision 4): object_store must carry feature \
             `{feature}`, found: {entry}"
        );
    }
    assert!(
        entry.contains("default-features = false"),
        "§A5 (ratified, revision 4): object_store must set default-features = \
         false, found: {entry}"
    );
}

#[test]
fn sqlx_features_match_a5() {
    // Decision 4 (VFL-9 comment `39d52469`, §A5 revision 4): §A5 names the
    // feature `tls-rustls`, but bare `tls-rustls` selects aws-lc-rs. The
    // ratified deviation is `tls-rustls-ring` (same crate, same rustls stack,
    // `ring` provider); `macros` is added for the compile-time checked
    // queries §A5's own sqlx rationale asks for.
    let cargo_toml = read_platform_file("Cargo.toml");
    let entry = extract_workspace_dependency_entry(&cargo_toml, "sqlx");
    for feature in ["tls-rustls-ring", "macros"] {
        assert!(
            entry.contains(&format!("\"{feature}\"")),
            "§A5 (ratified, revision 4): sqlx must carry feature `{feature}`, \
             found: {entry}"
        );
    }
    assert!(
        !entry.contains("\"tls-rustls\""),
        "§A5 (ratified, revision 4): sqlx must not use the bare `tls-rustls` \
         feature (it selects aws-lc-rs), found: {entry}"
    );
}

#[test]
fn kube_features_match_a5() {
    // Decision 4 (VFL-9 comment `39d52469`, §A5 revision 4): disabling kube's
    // default features drops its own defaults (`client`, `rustls-tls`,
    // `ring`), so the ratified deviation names `client` and `ring` explicitly
    // alongside §A5's own list.
    let cargo_toml = read_platform_file("Cargo.toml");
    let entry = extract_workspace_dependency_entry(&cargo_toml, "kube");
    for feature in ["client", "ring"] {
        assert!(
            entry.contains(&format!("\"{feature}\"")),
            "§A5 (ratified, revision 4): kube must carry feature `{feature}`, \
             found: {entry}"
        );
    }
}

#[test]
fn aws_lc_rs_does_not_resolve() {
    // Decision 4 (VFL-9 comment `39d52469`, §A5 revision 4), optional
    // assertion: the reqwest/object_store/sqlx/kube feature deviations above
    // all exist to keep a single `ring` rustls provider in the resolved
    // graph. Confirm the crate they exist to avoid is actually absent, not
    // just that the features asking for it are.
    let metadata = cargo_metadata();
    let packages = metadata["packages"].as_array().expect("packages array");
    let found = packages
        .iter()
        .any(|p| p["name"].as_str() == Some("aws-lc-rs"));
    assert!(
        !found,
        "§A5 (ratified, revision 4): `aws-lc-rs` must not appear anywhere in \
         the resolved dependency graph"
    );
}

#[test]
fn rust_toolchain_targets_match_a5() {
    // §A5 names three targets. Cortana's Decision 3 on VFL-9 required adding
    // `x86_64-unknown-linux-gnu` to the two the scaffold's first commit
    // (`d61b43f`, now retired) carried. `52015ab` already has all three.
    let toolchain = read_platform_file("rust-toolchain.toml");
    let mut targets = extract_toml_string_array(&toolchain, "targets")
        .unwrap_or_else(|| panic!("no `targets = [...]` array in rust-toolchain.toml"));
    targets.sort();

    let mut expected = vec![
        "aarch64-unknown-linux-gnu".to_string(),
        "wasm32-unknown-unknown".to_string(),
        "x86_64-unknown-linux-gnu".to_string(),
    ];
    expected.sort();

    assert_eq!(
        targets, expected,
        "§A5: rust-toolchain.toml targets must be exactly the three §A5 targets"
    );
}

#[test]
fn deny_toml_global_licence_allowlist_matches_a5() {
    // The first `allow = [...]` in the file, in file order, is the global
    // [licenses] list; the per-crate [[licenses.exceptions]] blocks (checked
    // below) come later in the committed file and are not this one.
    let deny_toml = read_platform_file("deny.toml");
    let mut allow_list = extract_toml_string_array(&deny_toml, "allow")
        .unwrap_or_else(|| panic!("no `allow = [...]` array in deny.toml"));
    allow_list.sort();

    let mut expected: Vec<String> = ["MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Unicode-3.0"]
        .into_iter()
        .map(String::from)
        .collect();
    expected.sort();

    assert_eq!(
        allow_list, expected,
        "§A5: the global licence allow-list must be exactly MIT, Apache-2.0, \
         BSD-2/3-Clause, ISC, Unicode-3.0 — MPL-2.0 is \"for listed crates only\" \
         and belongs in a per-crate exception, never in the global list"
    );
}

#[test]
fn deny_toml_licence_exceptions_match_ratified_set() {
    // Ratified on VFL-9 (Cortana, Decision 2): the MPL-2.0 pair is §A5 as written
    // ("MPL-2.0 for listed crates only"); the other four are deviations from the
    // §A5 allow-list, ratified per crate so a licence accepted for one dependency
    // does not silently cover the next crate someone adds.
    let deny_toml = read_platform_file("deny.toml");
    let expected: &[(&str, &str)] = &[
        ("cssparser", "MPL-2.0"),
        ("dtoa-short", "MPL-2.0"),
        ("foldhash", "Zlib"),
        ("webpki-roots", "CDLA-Permissive-2.0"),
        ("webpki-root-certs", "CDLA-Permissive-2.0"),
        ("xxhash-rust", "BSL-1.0"),
    ];

    // Strip `#` comment lines before splitting: the policy comment above
    // `[licenses]` names the `[[licenses.exceptions]]` header literally, and a
    // naive substring split on the raw file would count that prose mention as
    // a 7th block.
    let without_comments: String = deny_toml
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    let blocks: Vec<&str> = without_comments
        .split("[[licenses.exceptions]]")
        .skip(1)
        .collect();
    assert_eq!(
        blocks.len(),
        expected.len(),
        "§A5 (ratified): expected exactly {} per-crate licence exceptions, found \
         {} `[[licenses.exceptions]]` blocks",
        expected.len(),
        blocks.len()
    );

    for (crate_name, licence) in expected {
        // cargo-deny's `[[licenses.exceptions]]` schema names the crate with
        // `name`, not `crate` (confirmed against the 0.20 schema: a `crate`
        // key is simply unrecognised and ignored, not an error, so a config
        // using it would silently grant nothing rather than fail to parse).
        let name_line = format!("name = \"{crate_name}\"");
        let licence_literal = format!("\"{licence}\"");
        let found = blocks.iter().any(|block| {
            let names_this_crate = block.lines().any(|l| l.trim() == name_line.as_str());
            names_this_crate && block.contains(licence_literal.as_str())
        });
        assert!(
            found,
            "§A5 (ratified): no `[[licenses.exceptions]]` block grants `{crate_name}` \
             the `{licence}` licence"
        );
    }
}

// ---------------------------------------------------------------------------
// Engineering rules in every crate root (§A5) and the inline-tests lane rule
// ---------------------------------------------------------------------------

fn crate_dirs() -> Vec<PathBuf> {
    let crates_dir = platform_root().join("crates");
    std::fs::read_dir(&crates_dir)
        .unwrap_or_else(|e| panic!("reading {}: {e}", crates_dir.display()))
        .map(|entry| entry.expect("reading a crates/ directory entry").path())
        .filter(|path| path.is_dir())
        .collect()
}

fn rust_files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).unwrap_or_else(|e| panic!("reading {}: {e}", d.display())) {
            let path = entry.expect("reading a directory entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }
    out
}

#[test]
fn every_crate_root_forbids_unsafe_code() {
    for crate_dir in crate_dirs() {
        let name = crate_dir.file_name().unwrap().to_string_lossy().to_string();
        let mut roots_checked = 0;
        let mut root_files = vec![crate_dir.join("src/lib.rs"), crate_dir.join("src/main.rs")];
        // A crate may carry extra non-deployed binaries under src/bin/ (e.g.
        // vf-api's `vf-openapi`), each its own crate root with its own
        // `#![forbid(unsafe_code)]`, not implied by src/lib.rs or src/main.rs.
        if let Ok(entries) = std::fs::read_dir(crate_dir.join("src/bin")) {
            for entry in entries {
                let path = entry.expect("reading a src/bin directory entry").path();
                if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                    root_files.push(path);
                }
            }
        }
        for path in root_files {
            if !path.exists() {
                continue;
            }
            roots_checked += 1;
            let content = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
            assert!(
                content.contains("#![forbid(unsafe_code)]"),
                "§A5: {} is a crate root and must carry #![forbid(unsafe_code)]",
                path.display()
            );
        }
        assert!(
            roots_checked > 0,
            "`{name}` has neither src/lib.rs, src/main.rs nor a src/bin/*.rs"
        );
    }
}

#[test]
fn no_cfg_test_under_any_crate_src() {
    for crate_dir in crate_dirs() {
        let src_dir = crate_dir.join("src");
        if !src_dir.exists() {
            continue;
        }
        for path in rust_files_under(&src_dir) {
            let content = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
            assert!(
                !content.contains("#[cfg(test)]"),
                "lane rule: {} must not contain a #[cfg(test)] module; unit tests \
                 live in crates/<crate>/tests, not inline in src",
                path.display()
            );
        }
    }
}
