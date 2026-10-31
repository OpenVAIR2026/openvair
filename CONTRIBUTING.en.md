[中文](CONTRIBUTING.md) · **English**

# Contributing

Thanks for helping improve OpenVAIR.

## Reporting issues

- Bugs: include your OpenVAIR version, deployment method, steps to reproduce, expected result and actual result. Remove API keys, cookies and internal addresses before pasting logs.
- Feature requests: open an issue first describing the problem you want to solve and your use case. Agree on the direction before writing code so no effort goes to waste.
- Don't report security issues publicly. Report them privately as described in [SECURITY.md](SECURITY.md).

## Local development

The backend is Rust (Axum, SQLx, PostgreSQL, Redis), the console is React + TypeScript + Vite, and the desktop client (`client/`) is Tauri 2.

```bash
# Requires local PostgreSQL and Redis; the Rust process does not read .env automatically
export DATABASE_URL=postgres://openvair:openvair@localhost:5432/openvair
export REDIS_URL=redis://127.0.0.1/
export MASTER_KEY="$(openssl rand -hex 32)"
cargo run

npm --prefix web ci
npm --prefix web run dev
```

Database migrations live in `migrations/` and run automatically when the service starts. Design docs are in [docs/design/](docs/design/) (Chinese), deployment docs in [docs/](docs/), and the marketing site in [site/](site/).

## Submitting code

1. Fork the repository and create a branch from `main`.
2. Keep each PR to a single change. For database schema changes, add a new migration file; don't modify existing migrations.
3. Before submitting, run:

   ```bash
   cargo fmt --all
   cargo clippy --locked --all-targets -- -D warnings
   cargo clippy --locked --all-targets --features cloud -- -D warnings
   cargo test --locked
   npm --prefix web ci
   npm --prefix web run build
   git diff --check
   ```

   If your change touches the database, also run the relevant `test-*.sh` scripts under `scripts/`. If it touches the desktop client, verify it under `client/` following [client/README.md](client/README.md) (Chinese).

4. In the PR description, explain what you changed, why, and how you verified it.

See [AGENTS.md](AGENTS.md) for code conventions.

## License

The server and console are licensed under [AGPL-3.0](LICENSE), and the desktop client (`client/`) under [Apache-2.0](client/LICENSE). By submitting a PR, you agree that your contribution is licensed under the license of the directory it belongs to.
