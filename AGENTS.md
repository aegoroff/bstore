# bstore Project Rules

## Project Overview
Small microservice in Rust that implements blob storage with SQLite backend and simple REST API. SQLite backend means that blobs are stored inside the database, not in the file system.

## Tech Stack
- **Backend**: Rust (workspace with 4 crates: `bstore`, `client`, `server`, `kernel`)
- **Database**: SQLite (`rusqlite`, blobs stored in DB)
- **Build**: Cargo (Rust)

## Architecture

### Crates
| Crate | Role |
|-------|------|
| `bstore` | CLI binary (default member): `server`, `insert`, `list`, `bugreport` |
| `server` | Axum REST API, SQLite storage, OpenAPI/Swagger UI |
| `client` | HTTP client library used by CLI |
| `kernel` | Shared domain types (`Bucket`, `File`, `DeleteResult`) |

### Key libraries
- **HTTP**: Axum, Tower, `tower-http`
- **API docs**: `utoipa`, `utoipa-swagger-ui`
- **Data**: SQLite via `rusqlite` in `server`
- **Integrity**: BLAKE3 hashes for file content

### Runtime configuration (server)
- `BSTORE_DATA_DIR` — directory for the DB file (default: `./`)
- `BSTORE_DATA_FILE` — DB file name (default: `bstore.db`)
- `BSTORE_PORT` — listen port (default: `5000`)

## Coding Standards

### Rust
- Use `unsafe_code = "forbid"` (workspace lints)
- Prefer `Result<T, E>` over panicking; avoid `unwrap` / `expect` on `Option` / `Result` in new code
- Use async/await with tokio runtime
- Follow workspace dependency versions
- Run `cargo clippy` before committing
- Create tests for new functionality
- Write tests in AAA pattern
- If tests can be parameterized, use the `test-case` crate
- Code must pass all clippy pedantic validations
- Result code must be formatted using `cargo fmt` (`style_edition = "2024"`)
- Write code comments only in English

## Commands

Run from the repository root (default package is `bstore`):

```bash
cargo build                                          # Build workspace
cargo test                                           # Run tests
cargo clippy --workspace -- -W clippy::pedantic      # Lint
cargo run -- server                                  # Start web server
cargo run -- list bucket -u http://localhost:5000    # List buckets via CLI
```

## Important Notes
- Workspace uses resolver = "3" (Cargo edition 2024)
- Release profile: `codegen-units = 1`, strip symbols, `panic = abort`

## When Making Changes

### General
1. Check related files in the same crate and across layers (`kernel` ↔ `server`/`client` ↔ `bstore`)
2. Ensure imports are updated
3. Run linters (`cargo clippy`)
4. Verify the workspace compiles after changes

## Things NOT to do
- Do not use unsafe code
- Do not use `unwrap` or `expect` to get `Option` or `Result`
- Do not flag performance, copy/paste, or architecture nits in tests unless asked
- Do not suppress clippy warnings using attribute macros like `#[allow(clippy::unused_self)]`
- Do not write trivial code comments
- Do not write comments in Russian
