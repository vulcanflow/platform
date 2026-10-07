//! Postgres for a test: migrated, templated, and reachable from both sides of
//! PgBouncer (architecture §A6.1 Postgres row, §A6.2).
//!
//! # Why two pools
//!
//! §A5 pins PgBouncer into the local harness precisely so prepared-statement
//! behaviour is exercised now rather than first on Aether. A test that wants to
//! prove a query survives a pooled connection being handed to the next
//! transaction uses [`TestDb::pooled`]; a test that needs session-scoped state
//! (`SET`, `LISTEN`, a temp table) uses [`TestDb::direct`]. Both point at the
//! same database, so a row written through one is visible through the other.
//!
//! # Why a template schema
//!
//! §A4 creates one tenant schema per tenant from a single migration template,
//! and the sqlx offline query data in `.sqlx/` is generated against that
//! template. [`TestDb`] materialises it once per database as
//! [`TestDb::TENANT_TEMPLATE_SCHEMA`], and [`TestDb::create_tenant_schema`]
//! builds each further tenant schema the way §A4's provisioner does: a
//! `CREATE SCHEMA` and a replay of the tenant migrations into it.
//!
//! A replay, not a table copy. `CREATE TABLE … (LIKE … INCLUDING ALL)` copies
//! columns, defaults, checks and indexes, and silently drops row-level
//! security, its policies, foreign keys and triggers — which is to say the
//! parts of a tenant schema that §A7 invariant 7 is about. A harness whose
//! tenant schemas have no RLS would let an isolation test pass for the wrong
//! reason, so the clone is built by the same migrations as the original.
//!
//! # Where the migrations are
//!
//! §A4: forward-only SQL under `crates/vf-db/migrations/control/` (applied
//! once per database) and `crates/vf-db/migrations/tenant/` (applied once per
//! tenant schema, with `search_path` set to that schema). They are read at run
//! time rather than embedded with `sqlx::migrate!`, because the macro refuses
//! to compile against a directory that does not exist, and until task C3
//! lands neither does. An absent or empty directory is an empty migration
//! set, as it is for `just db-migrate`.
//!
//! # Backends
//!
//! | | [`Backend::Compose`] | [`Backend::Testcontainers`] |
//! |---|---|---|
//! | Postgres | the `postgres` service of `just dev-up` | a container per handle |
//! | PgBouncer | the `pgbouncer` service | a container per handle, on a network shared with that Postgres only |
//! | Database | `vf_test_<uuid>`, created for the handle and dropped with it | the container's own |
//! | Images | whatever compose started | the pins in `docker-compose.yml`, or the same `VF_*_IMAGE` override compose honours |
//!
//! The compose backend needs PgBouncer to route a database it has never heard
//! of, because every handle creates a new one: the `pgbouncer` service's
//! `[databases]` section has to carry the `*` wildcard entry. When it does
//! not, the pooled connection is refused and [`TestDb::with_backend`] says so
//! in those terms rather than as a bare PgBouncer error.
//!
//! A compose database is dropped when its handle is, including when the test
//! panics. A test process that is killed outright cannot drop anything; its
//! databases are left behind, all named `vf_test_*`, and
//! `just dev-down --volumes` discards them.

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{Connection, PgConnection, PgPool};
use testcontainers::core::{Healthcheck, IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};
use testcontainers_modules::postgres::Postgres;

use crate::{Error, Result};

/// The environment variable holding the pooled URL, through PgBouncer
/// (`.env.example`).
pub const POOLED_URL_VAR: &str = "VF_DATABASE_URL";

/// The environment variable holding the direct URL, straight to Postgres
/// (`.env.example`).
pub const DIRECT_URL_VAR: &str = "VF_DATABASE_URL_DIRECT";

/// Where the Postgres under a [`TestDb`] comes from.
///
/// Both backends present the same surface, PgBouncer included: the acceptance
/// criterion for the pooled pool is that it is genuinely pooled, so the
/// testcontainers backend starts a PgBouncer container of its own rather than
/// aliasing [`TestDb::pooled`] onto the direct pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Backend {
    /// The long-lived `postgres` and `pgbouncer` services from
    /// `docker-compose.yml`, addressed through the `.env.example` variables.
    /// Fast, and the default when those variables are set.
    Compose,
    /// Throwaway containers started by `testcontainers` for this process.
    /// Needs only a container runtime, so it is what a developer with no
    /// `just dev-up` running gets.
    Testcontainers,
}

/// A migrated Postgres database owned by one test process.
///
/// Dropping a `TestDb` releases the pools. For the [`Backend::Testcontainers`]
/// backend it also stops the containers; for [`Backend::Compose`] the database
/// created for this handle is dropped, leaving the shared services running.
#[derive(Debug)]
pub struct TestDb {
    backend: Backend,
    database: String,
    direct: sqlx::PgPool,
    pooled: sqlx::PgPool,
    /// What has to be released with this handle. `None` once [`TestDb::close`]
    /// has released it, so `Drop` does not do it twice.
    teardown: Option<Teardown>,
}

impl TestDb {
    /// The schema that holds the control tables of §A4.
    pub const CONTROL_SCHEMA: &str = "vf";

    /// The schema the per-tenant schemas of §A4 are cloned from.
    pub const TENANT_TEMPLATE_SCHEMA: &str = "tenant_template";

    /// Brings up a database, applies the `vf-db` migrations to it, and
    /// materialises the tenant template schema.
    ///
    /// Picks [`Backend::Compose`] when the `.env.example` Postgres variables
    /// ([`DIRECT_URL_VAR`] and [`POOLED_URL_VAR`]) are set in the environment
    /// and both services answer, and [`Backend::Testcontainers`] otherwise.
    /// The variables are read from the process environment only; `cargo test`
    /// does not read `.env`, so export it (`set -a; . ./.env; set +a`) to get
    /// the compose backend.
    ///
    /// Until task C3 lands there are no migration files; the migration step
    /// applies an empty set and succeeds, and the resulting database has the
    /// two schemas and nothing in them. The acceptance test for this
    /// constructor is re-run when C3 closes.
    pub async fn new() -> Result<Self> {
        if let Some(config) = ComposeConfig::from_env()?
            && config.answers().await
        {
            return Self::compose(config).await;
        }
        Self::testcontainers().await
    }

    /// As [`TestDb::new`], with the backend chosen by the caller rather than
    /// detected. A test that is specifically about pooled connection reuse
    /// pins [`Backend::Compose`] so it fails loudly when the harness is down,
    /// instead of silently falling back.
    pub async fn with_backend(backend: Backend) -> Result<Self> {
        match backend {
            Backend::Compose => {
                let config = ComposeConfig::from_env()?.ok_or_else(|| {
                    Error::Backend(format!(
                        "the compose backend needs {DIRECT_URL_VAR} and {POOLED_URL_VAR} in the \
                         environment: run `just dev-up` and export `.env`"
                    ))
                })?;
                Self::compose(config).await
            }
            Backend::Testcontainers => Self::testcontainers().await,
        }
    }

    /// Which backend this handle actually got.
    pub fn backend(&self) -> Backend {
        self.backend
    }

    /// The database name this handle owns.
    pub fn database(&self) -> &str {
        &self.database
    }

    /// A pool connected straight to Postgres, bypassing PgBouncer.
    ///
    /// Use it for migrations, for session-scoped state, and for any assertion
    /// about what the server itself did.
    pub fn direct(&self) -> &sqlx::PgPool {
        &self.direct
    }

    /// A pool connected through PgBouncer in transaction pooling mode.
    ///
    /// This is the pool the services use, so it is the one that proves §A5's
    /// claim about prepared statements and the one `vf-db`'s RLS tests reuse a
    /// connection across. Its statement cache is off whatever the URL says,
    /// because a statement prepared on one server connection does not exist on
    /// the next one PgBouncer hands out.
    pub fn pooled(&self) -> &sqlx::PgPool {
        &self.pooled
    }

    /// Creates `tenant_<slug>` from the tenant migrations and returns the
    /// schema name, as §A3.9's provisioner does for real.
    ///
    /// Idempotent: calling it twice for one slug returns the same schema
    /// without touching its contents.
    ///
    /// `slug` is lower-case ASCII letters, digits and `_`, at most 56 of them
    /// so the schema name fits Postgres's 63-byte identifier limit. Anything
    /// else is [`Error::TenantSlug`]; the real slug rules are task C1's, and
    /// these are only the ones a schema name needs.
    pub async fn create_tenant_schema(&self, slug: &str) -> Result<String> {
        let valid = !slug.is_empty()
            && slug.len() <= MAX_SLUG_LEN
            && slug
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if !valid {
            return Err(Error::TenantSlug(slug.to_owned()));
        }

        let schema = format!("tenant_{slug}");
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_namespace WHERE nspname = $1)")
                .bind(&schema)
                .fetch_one(&self.direct)
                .await?;
        if !exists {
            materialise_tenant_schema(&self.direct, &schema).await?;
        }
        Ok(schema)
    }

    /// Truncates every table in the control schema and in every tenant schema,
    /// leaving the structure in place. Cheaper than a fresh [`TestDb`] for a
    /// test that only needs empty tables.
    ///
    /// Identity columns and sequences owned by those tables restart, so ids a
    /// test reads back are the same on every run. The `_sqlx_migrations`
    /// bookkeeping tables are left alone: emptying them would make the next
    /// [`TestDb::create_tenant_schema`] replay migrations onto tables that
    /// already exist.
    pub async fn truncate_all(&self) -> Result<()> {
        let tables: Vec<String> = sqlx::query_scalar(
            "SELECT format('%I.%I', n.nspname, c.relname) \
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE c.relkind IN ('r', 'p') \
               AND NOT c.relispartition \
               AND c.relname <> '_sqlx_migrations' \
               AND (n.nspname = $1 OR n.nspname LIKE 'tenant\\_%') \
             ORDER BY 1",
        )
        .bind(Self::CONTROL_SCHEMA)
        .fetch_all(&self.direct)
        .await?;
        if tables.is_empty() {
            return Ok(());
        }
        // The names come back from `format('%I.%I')`, so they are already
        // quoted where they need to be.
        ddl(format!(
            "TRUNCATE TABLE {} RESTART IDENTITY CASCADE",
            tables.join(", ")
        ))
        .execute(&self.direct)
        .await?;
        Ok(())
    }

    /// Closes both pools and releases the backend. Called for you on drop;
    /// call it explicitly when a test wants the teardown error rather than a
    /// best-effort drop.
    pub async fn close(mut self) -> Result<()> {
        self.direct.close().await;
        self.pooled.close().await;
        match self.teardown.take() {
            Some(Teardown::DropDatabase { admin }) => drop_database(&admin, &self.database).await,
            Some(Teardown::Containers(containers)) => {
                let Containers {
                    pgbouncer,
                    postgres,
                } = *containers;
                pgbouncer
                    .rm()
                    .await
                    .map_err(backend_error("removing pgbouncer"))?;
                postgres
                    .rm()
                    .await
                    .map_err(backend_error("removing postgres"))
            }
            None => Ok(()),
        }
    }

    /// The compose backend: a fresh database on the running services.
    async fn compose(config: ComposeConfig) -> Result<Self> {
        let database = format!("vf_test_{}", uuid::Uuid::now_v7().simple());
        let mut admin = PgConnection::connect_with(&config.direct).await?;
        ddl(format!("CREATE DATABASE {}", quote_ident(&database)))
            .execute(&mut admin)
            .await?;
        admin.close().await?;

        let direct = config.direct.clone().database(&database);
        let pooled = through_pgbouncer(config.pooled.clone().database(&database));
        let made = async {
            let pooled = connect(pooled).await.map_err(|e| {
                Error::Backend(format!(
                    "PgBouncer at {POOLED_URL_VAR} would not connect to database `{database}` \
                     ({e}). The compose `pgbouncer` service has to route every database — a \
                     `*` entry in its [databases] section — because each TestDb creates its own."
                ))
            })?;
            Ok::<_, Error>((connect(direct).await?, pooled))
        }
        .await;

        let (direct, pooled) = match made {
            Ok(pools) => pools,
            Err(e) => {
                // Best effort: the error being returned is the one the test
                // needs to see, not a second one from the cleanup.
                let _ = drop_database(&config.direct, &database).await;
                return Err(e);
            }
        };

        // From here on the handle exists, so a failure below drops the
        // database through `Drop`.
        let db = Self {
            backend: Backend::Compose,
            database,
            direct,
            pooled,
            teardown: Some(Teardown::DropDatabase {
                admin: Box::new(config.direct),
            }),
        };
        db.migrate().await?;
        Ok(db)
    }

    /// The testcontainers backend: a Postgres and a PgBouncer of our own.
    async fn testcontainers() -> Result<Self> {
        let postgres_image = ImageRef::pinned("VF_POSTGRES_IMAGE")?;
        let pgbouncer_image = ImageRef::pinned("VF_PGBOUNCER_IMAGE")?;

        let id = uuid::Uuid::now_v7().simple();
        let network = format!("vf-testkit-{id}");
        let postgres_host = format!("vf-testkit-pg-{id}");

        let postgres = Postgres::default()
            .with_db_name(CONTAINER_DATABASE)
            .with_user(CONTAINER_USER)
            .with_password(CONTAINER_PASSWORD)
            .with_name(postgres_image.name)
            .with_tag(postgres_image.tag)
            // scram, as in docker-compose.yml: PgBouncer's auth path differs
            // between md5 and scram, and the harness exists to exercise the
            // real one.
            .with_env_var("POSTGRES_INITDB_ARGS", "--auth-host=scram-sha-256")
            .with_network(&network)
            .with_container_name(&postgres_host)
            .start()
            .await
            .map_err(backend_error("starting postgres"))?;

        // The same settings as the compose `pgbouncer` service, except that
        // there is no `DB_NAME`: without it the image routes every database
        // (`*`), which is what a harness that creates databases needs.
        let port = PGBOUNCER_PORT.to_string();
        let pgbouncer = GenericImage::new(pgbouncer_image.name, pgbouncer_image.tag)
            .with_exposed_port(PGBOUNCER_PORT.tcp())
            .with_wait_for(WaitFor::healthcheck())
            .with_env_var("DB_HOST", &postgres_host)
            .with_env_var("DB_PORT", "5432")
            .with_env_var("DB_USER", CONTAINER_USER)
            .with_env_var("DB_PASSWORD", CONTAINER_PASSWORD)
            .with_env_var("AUTH_TYPE", "scram-sha-256")
            .with_env_var("POOL_MODE", "transaction")
            .with_env_var("LISTEN_PORT", &port)
            .with_env_var("DEFAULT_POOL_SIZE", "10")
            .with_env_var("MAX_CLIENT_CONN", "100")
            .with_env_var("MAX_PREPARED_STATEMENTS", "0")
            // Probed inside the container, as compose does: Docker's port
            // proxy accepts a host connection before PgBouncer is listening.
            .with_health_check(
                Healthcheck::cmd(["nc", "-z", "127.0.0.1", port.as_str()])
                    .with_interval(Duration::from_millis(500))
                    .with_timeout(Duration::from_secs(3))
                    .with_retries(60),
            )
            .with_network(&network)
            .start()
            .await
            .map_err(backend_error("starting pgbouncer"))?;

        let host = postgres
            .get_host()
            .await
            .map_err(backend_error("reading the postgres host"))?
            .to_string();
        let postgres_port = postgres
            .get_host_port_ipv4(5432)
            .await
            .map_err(backend_error("reading the postgres port"))?;
        let pgbouncer_port = pgbouncer
            .get_host_port_ipv4(PGBOUNCER_PORT)
            .await
            .map_err(backend_error("reading the pgbouncer port"))?;

        let options = |port| {
            PgConnectOptions::new()
                .host(&host)
                .port(port)
                .username(CONTAINER_USER)
                .password(CONTAINER_PASSWORD)
                .database(CONTAINER_DATABASE)
        };
        // A failure from here on drops the containers, and with them the
        // network, through their own `Drop`.
        let direct = connect(options(postgres_port)).await?;
        let pooled = connect(through_pgbouncer(options(pgbouncer_port))).await?;

        let db = Self {
            backend: Backend::Testcontainers,
            database: CONTAINER_DATABASE.to_owned(),
            direct,
            pooled,
            teardown: Some(Teardown::Containers(Box::new(Containers {
                pgbouncer,
                postgres,
            }))),
        };
        db.migrate().await?;
        Ok(db)
    }

    /// Applies the control migrations, creates the two schemas, and replays
    /// the tenant migrations into the template.
    async fn migrate(&self) -> Result<()> {
        if let Some(control) = migrator("control").await? {
            control.run(&self.direct).await?;
        }
        // After the control migrations, so a migration that creates `vf`
        // itself is not refused; `IF NOT EXISTS` makes this a no-op then.
        ddl(format!(
            "CREATE SCHEMA IF NOT EXISTS {}",
            quote_ident(Self::CONTROL_SCHEMA)
        ))
        .execute(&self.direct)
        .await?;
        materialise_tenant_schema(&self.direct, Self::TENANT_TEMPLATE_SCHEMA).await
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        match self.teardown.take() {
            Some(Teardown::DropDatabase { admin }) => {
                let database = std::mem::take(&mut self.database);
                // `Drop` cannot await, and blocking on the test's own runtime
                // from inside it panics. So the drop runs on a thread of its
                // own, with a runtime of its own and a connection of its own,
                // and this thread waits for it. `WITH (FORCE)` ends the pool
                // connections that are still open, here and in any pool clone
                // the test handed to the code under test.
                let dropped = std::thread::spawn(move || {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|e| Error::Backend(e.to_string()))?;
                    runtime.block_on(drop_database(&admin, &database))
                })
                .join();
                if let Ok(Err(e)) = dropped {
                    // Nothing can be returned from `Drop`; say it where the
                    // test output will show it rather than lose it.
                    eprintln!("vf-testkit: could not drop the test database: {e}");
                }
            }
            // `ContainerAsync`'s own `Drop` removes the containers.
            Some(Teardown::Containers(_)) | None => {}
        }
    }
}

/// What has to be released when a [`TestDb`] goes. Both payloads are boxed:
/// connection options and container handles are each a few hundred bytes,
/// and the handle carries this for its whole life to use once.
enum Teardown {
    /// Compose: drop the handle's database through a fresh connection with
    /// these options, which point at the configured maintenance database.
    DropDatabase { admin: Box<PgConnectOptions> },
    /// Testcontainers: remove both containers.
    Containers(Box<Containers>),
}

/// The two containers of a [`Backend::Testcontainers`] handle.
struct Containers {
    pgbouncer: ContainerAsync<GenericImage>,
    postgres: ContainerAsync<Postgres>,
}

/// Written by hand so that a `TestDb` in a test failure message prints which
/// backend it holds and not the connection options, password included.
impl fmt::Debug for Teardown {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DropDatabase { .. } => f.write_str("DropDatabase"),
            Self::Containers(containers) => f
                .debug_struct("Containers")
                .field("pgbouncer", &containers.pgbouncer.id())
                .field("postgres", &containers.postgres.id())
                .finish(),
        }
    }
}

/// Postgres's identifier limit is 63 bytes, and `tenant_` takes seven.
const MAX_SLUG_LEN: usize = 63 - "tenant_".len();

/// The database, user and password inside a testcontainers Postgres. Local
/// to a container that lives as long as one test, so obvious values are the
/// right ones; they match `.env.example`'s.
const CONTAINER_DATABASE: &str = "vulcanflow";
const CONTAINER_USER: &str = "vulcanflow";
const CONTAINER_PASSWORD: &str = "vulcanflow";

/// The port PgBouncer listens on inside its container.
const PGBOUNCER_PORT: u16 = 6432;

/// Connections per pool. Small on purpose: compose Postgres allows 100
/// connections, cargo runs tests in parallel, and each handle holds two pools
/// plus PgBouncer's server connections for its database.
const POOL_SIZE: u32 = 4;

/// The compose backend's two URLs, parsed.
struct ComposeConfig {
    direct: PgConnectOptions,
    pooled: PgConnectOptions,
}

impl ComposeConfig {
    /// `None` when neither variable is set. Exactly one being set, or either
    /// not parsing, is an error rather than a fallback: it is a harness
    /// configured wrongly, and falling back would hide that.
    fn from_env() -> Result<Option<Self>> {
        let direct = std::env::var(DIRECT_URL_VAR).ok().filter(|v| !v.is_empty());
        let pooled = std::env::var(POOLED_URL_VAR).ok().filter(|v| !v.is_empty());
        let (direct, pooled) = match (direct, pooled) {
            (None, None) => return Ok(None),
            (Some(direct), Some(pooled)) => (direct, pooled),
            (Some(_), None) | (None, Some(_)) => {
                return Err(Error::Backend(format!(
                    "set both {DIRECT_URL_VAR} and {POOLED_URL_VAR}, or neither"
                )));
            }
        };
        // The URL carries the password, so a parse error names the variable
        // and not the value.
        let parse = |var: &str, url: &str| {
            PgConnectOptions::from_str(url)
                .map_err(|e| Error::Backend(format!("{var} is not a usable Postgres URL: {e}")))
        };
        Ok(Some(Self {
            direct: parse(DIRECT_URL_VAR, &direct)?,
            pooled: parse(POOLED_URL_VAR, &pooled)?,
        }))
    }

    /// Whether both services accept a connection to the configured database.
    async fn answers(&self) -> bool {
        for options in [&self.direct, &self.pooled] {
            let probe = PgPoolOptions::new()
                .max_connections(1)
                .acquire_timeout(Duration::from_secs(3))
                .connect_with(options.clone())
                .await;
            match probe {
                Ok(pool) => pool.close().await,
                Err(_) => return false,
            }
        }
        true
    }
}

/// An image reference split the way `testcontainers` wants it: the name, and
/// a tag that carries the digest when there is one, so the reference it
/// pulls is the pinned one byte for byte.
struct ImageRef {
    name: String,
    tag: String,
}

impl ImageRef {
    /// The pin for `variable`: its value when it is set and not empty, as
    /// compose's `${VAR:-default}` would have it, and otherwise the default
    /// written in `docker-compose.yml`.
    ///
    /// Read from that file rather than restated here, because it is the one
    /// place the image pins live (it says so in its header) and a second copy
    /// would drift.
    fn pinned(variable: &str) -> Result<Self> {
        if let Some(reference) = std::env::var(variable).ok().filter(|v| !v.is_empty()) {
            return Ok(Self::parse(&reference));
        }
        let path = compose_file();
        let text = std::fs::read_to_string(&path).map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;
        let needle = format!("${{{variable}:-");
        let reference = text
            .split_once(&needle)
            .and_then(|(_, rest)| rest.split_once('}'))
            .map(|(reference, _)| reference.trim())
            .filter(|reference| !reference.is_empty())
            .ok_or_else(|| {
                Error::Backend(format!(
                    "{} has no `{needle}…}}` image pin to start a container from",
                    path.display()
                ))
            })?;
        Ok(Self::parse(reference))
    }

    /// Splits `name[:tag][@digest]`. The tag is looked for after the last
    /// `/`, so a registry port (`registry:5000/image`) is not taken for one.
    fn parse(reference: &str) -> Self {
        let (named, digest) = match reference.split_once('@') {
            Some((named, digest)) => (named, Some(digest)),
            None => (reference, None),
        };
        let last_segment = named.rfind('/').map_or(0, |i| i + 1);
        let (name, tag) = match named[last_segment..].rfind(':') {
            Some(i) => (&named[..last_segment + i], &named[last_segment + i + 1..]),
            None => (named, "latest"),
        };
        Self {
            name: name.to_owned(),
            tag: match digest {
                Some(digest) => format!("{tag}@{digest}"),
                None => tag.to_owned(),
            },
        }
    }
}

/// `docker-compose.yml` at the root of the `platform` repository.
fn compose_file() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docker-compose.yml")
}

/// `crates/vf-db/migrations/<set>`.
fn migrations_dir(set: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../vf-db/migrations")
        .join(set)
}

/// The migrations in `set`, or `None` when there are none to apply.
///
/// `None` rather than an empty migrator, so that a database with no
/// migrations has no `_sqlx_migrations` table either and is exactly the "two
/// schemas and nothing in them" that [`TestDb::new`] promises.
async fn migrator(set: &str) -> Result<Option<sqlx::migrate::Migrator>> {
    let dir = migrations_dir(set);
    if !dir.is_dir() {
        return Ok(None);
    }
    let migrator = sqlx::migrate::Migrator::new(dir).await?;
    Ok((migrator.iter().next().is_some()).then_some(migrator))
}

/// Creates `schema` and replays the tenant migrations into it.
///
/// The replay runs on a connection of its own whose `search_path` is the new
/// schema, set as a startup parameter, so unqualified names in the migrations
/// land there — as they do under `vf-db migrate --all-tenants` — and nothing
/// about the session leaks back into a pool.
async fn materialise_tenant_schema(direct: &PgPool, schema: &str) -> Result<()> {
    ddl(format!(
        "CREATE SCHEMA IF NOT EXISTS {}",
        quote_ident(schema)
    ))
    .execute(direct)
    .await?;
    let Some(tenant) = migrator("tenant").await? else {
        return Ok(());
    };
    let options = (*direct.connect_options())
        .clone()
        .options([("search_path", schema)]);
    let mut conn = PgConnection::connect_with(&options).await?;
    let replayed = tenant.run(&mut conn).await;
    let closed = conn.close().await;
    replayed?;
    closed?;
    Ok(())
}

/// A pool on `options`, sized for a test.
async fn connect(options: PgConnectOptions) -> Result<PgPool> {
    Ok(PgPoolOptions::new()
        .max_connections(POOL_SIZE)
        .acquire_timeout(Duration::from_secs(30))
        .connect_with(options)
        .await?)
}

/// `options` made safe for PgBouncer's transaction pooling: no client-side
/// statement cache, whatever the URL said (see `.env.example` on
/// `VF_DATABASE_URL`).
fn through_pgbouncer(options: PgConnectOptions) -> PgConnectOptions {
    options.statement_cache_capacity(0)
}

/// Drops `database`, ending any connection still open on it.
async fn drop_database(admin: &PgConnectOptions, database: &str) -> Result<()> {
    let mut conn = PgConnection::connect_with(admin).await?;
    ddl(format!(
        "DROP DATABASE IF EXISTS {} WITH (FORCE)",
        quote_ident(database)
    ))
    .execute(&mut conn)
    .await?;
    conn.close().await?;
    Ok(())
}

/// `ident` as a quoted SQL identifier.
fn quote_ident(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

/// A statement whose only dynamic parts are identifiers.
///
/// sqlx 0.9 refuses a SQL string that is not a literal unless it is wrapped
/// in `AssertSqlSafe`, and the wrapper is a claim that someone audited it.
/// This is the one place this crate makes that claim. Every caller builds its
/// statement from literal SQL plus identifiers that went through
/// [`quote_ident`] or Postgres's own `format('%I')`; a value is never spliced
/// in, it is a bound parameter on a `sqlx::query` instead.
///
/// `raw_sql` is also the simple query protocol, which is what `CREATE
/// DATABASE` and `DROP DATABASE` need: they refuse to run inside a
/// transaction block, and an extended-protocol statement may be wrapped in an
/// implicit one.
fn ddl(sql: String) -> sqlx::RawSql {
    sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
}

/// Maps a `testcontainers` error to [`Error::Backend`], saying what was being
/// attempted.
fn backend_error(doing: &'static str) -> impl Fn(testcontainers::TestcontainersError) -> Error {
    move |e| Error::Backend(format!("{doing}: {e}"))
}
