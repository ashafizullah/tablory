<p align="center">
  <img src="design/icon.png" width="128" alt="Tablory icon">
</p>

<h1 align="center">Tablory</h1>

<p align="center">
  A free, open-source, native database manager for macOS and Windows. PostgreSQL, MySQL, SQL Server, SQLite, Redis and MongoDB in one small app.
</p>

<p align="center">
  <a href="https://github.com/ashafizullah/tablory/actions/workflows/ci.yml"><img src="https://github.com/ashafizullah/tablory/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/ashafizullah/tablory/releases/latest"><img src="https://img.shields.io/github/v/release/ashafizullah/tablory?include_prereleases&label=release" alt="Release"></a>
  <a href="https://github.com/ashafizullah/tablory/releases"><img src="https://img.shields.io/github/downloads/ashafizullah/tablory/total?color=2563eb" alt="Downloads"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/ashafizullah/tablory?color=blue" alt="MIT License"></a>
  <br>
  <img src="https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white" alt="Tauri 2">
  <img src="https://img.shields.io/badge/Svelte-5-FF3E00?logo=svelte&logoColor=white" alt="Svelte 5">
  <img src="https://img.shields.io/badge/Rust-stable-000000?logo=rust" alt="Rust">
  <img src="https://img.shields.io/badge/platform-macOS%20%7C%20Windows-lightgrey" alt="macOS | Windows">
</p>

<p align="center">
  <a href="https://trakteer.id/adamshafizullah/tip"><img src="https://img.shields.io/badge/Buy%20me%20an%20AI%20token-Trakteer-C02E2E?style=for-the-badge" alt="Buy me an AI token on Trakteer"></a>
</p>

## Download

Get the latest build from [Releases](https://github.com/ashafizullah/tablory/releases/latest):

- **macOS** (Apple Silicon + Intel): `Tablory_*_universal.dmg`. The app is ad-hoc signed, not notarized, so the first time right-click Tablory → Open, or run `xattr -cr /Applications/Tablory.app`.
- **Windows**: `Tablory_*_x64-setup.exe` or the `.msi`.

The app checks for updates on launch and daily; click the version number on the connections screen to check now.

## Features

- PostgreSQL, MySQL / MariaDB, SQL Server, SQLite, Redis and MongoDB
- Connections over SSH tunnels (password or private key); passwords live in the OS keychain
- Organize connections into collapsible groups: right-click the list for a new group, drag connections between groups
- Import connections from Navicat (File → Export Connections, `.ncx`), passwords included
- Browse tables with server-side paging, sorting and filters (conditions or a raw `WHERE`)
- Edit inline: change cells, add and delete rows, preview the SQL, commit in one transaction (⌘S)
- SQL editor with highlighting and autocomplete; run the statement under the cursor (⌘↵) or everything (⇧⌘↵), cancel long queries
- Table structure: columns, indexes, foreign keys
- Redis: browse keys by pattern (SCAN), view and edit strings, hashes, lists, sets and sorted sets, set TTLs, rename and delete keys, and a command console
- MongoDB: browse collections with JSON filters and sorts, edit documents as Extended JSON, insert and delete documents, and run database commands; connect by host or by connection string

On SQL Server, separate batches with `GO` lines. A batch that changes data reports affected rows; any other batch shows its result sets. Cancelling a query ends the editor session, which reconnects on the next run.

Rows can only be edited in tables with a primary key, and an edit that does not match exactly one row rolls the whole commit back.

## Shortcuts

| Key | Action |
| --- | --- |
| ⌘T | New SQL query |
| ⌘W | Close tab |
| ⌘S | Commit pending edits |
| ⌘R | Reload table |
| ⌘↵ / ⇧⌘↵ | Run statement / run all |
| ⌘K | Disconnect |
| ⌘F | Search tables |

## Development

```sh
npm install
npm run tauri dev
```

Integration tests run against local databases and an SSH bastion:

```sh
ssh-keygen -t ed25519 -N '' -f dev/ssh/id_ed25519   # once, test key for the bastion
docker compose -f dev/docker-compose.yml up -d --build
cd src-tauri && TABLORY_TEST_DOCKER=1 cargo test
```

Without `TABLORY_TEST_DOCKER` only the unit tests and the SQLite test run.

## Releasing

1. Add the updater signing key as the repository secret `TAURI_SIGNING_PRIVATE_KEY` (contents of `~/.tauri/tablory.key`, empty password).
2. Bump the version in `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`, and add a `## <version>` section to `CHANGELOG.md`.
3. Tag and push: `git tag v0.1.0 && git push origin v0.1.0`. The workflow builds a draft release; publish it when it looks right.

## Support

Tablory is free and always will be. If it saves you time, **[buy me an AI token on Trakteer](https://trakteer.id/adamshafizullah/tip)** ☕🤖. It keeps this project being developed.

<p align="center">
  <a href="https://trakteer.id/adamshafizullah/tip"><img src="design/trakteer-qr.png" width="200" alt="QR code: trakteer.id/adamshafizullah/tip"></a>
  <br>
  <sub>Scan to support on Trakteer</sub>
</p>

## License

MIT
