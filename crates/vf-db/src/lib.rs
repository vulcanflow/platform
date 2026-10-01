#![forbid(unsafe_code)]
//! Tenant-scoped data access. `TenantTx` is the only way tenant data is reached.
//!
//! # Invariants
//!
//! **Isolation by construction.** There is no public path from this crate to a
//! queryable handle that does not already carry a tenant context. Forgetting the
//! tenant filter has to be a compile error, not a leak — TDD §3.5 puts isolation
//! in the database (schema separation plus `FORCE`d row-level security), and this
//! crate's job is to make the application side unable to bypass it.
//!
//! **`SET LOCAL`, never a bare `SET`.** ADR-0002 §4.3 selected PgBouncer in
//! *transaction* pooling mode. Under transaction pooling a session-level `SET`
//! leaks to the next borrower of that server connection, and a leaked tenant GUC
//! is a cross-tenant RLS bypass. Tenant context is therefore applied inside the
//! transaction with `SET LOCAL` and dies with it. A bare `SET` anywhere in this
//! crate is a required change at review (§25 `isolation/background-queries`,
//! `db/tenanttx-set-local-isolation`).
//!
//! **No SQL-level `PREPARE` / `EXECUTE` / `DEALLOCATE`.** PgBouncer's
//! prepared-statement support rewrites the extended-query protocol and explicitly
//! does not track those statements; they break under transaction pooling.
//! `sqlx`'s own statement cache is left at its default capacity, for the reasons
//! recorded in ADR-0002 §4.3.
//!
//! **Background work is tenant-scoped too.** A scheduled job, a reconciler and an
//! ingest worker reach data the same way a request does. There is no
//! "trusted internal" path; §25 `isolation/background-queries` exists because that
//! is where isolation is usually lost.
//!
//! # Status
//!
//! Scaffold. `TenantTx` itself is VUL-13.
