# Security

## Reporting a vulnerability

Please report suspected vulnerabilities through GitHub's private security-advisory flow. Do not include clipboard contents, secrets, or private logs in a public issue.

## Release security gates

Cubby release candidates must pass the JavaScript production dependency audit, the Rust advisory audit, the automated checks in `scripts/smoke-release.ps1`, and the privacy checks in `scripts/check-release.mjs`. Packaged-install smoke steps remain in `docs/RELEASE_CHECKLIST.md`.

The opener URL allowlist in `src-tauri/capabilities/default.json` is checked by `scripts/check-release.mjs` using the same glob defaults as `tauri-plugin-opener` (star and question-mark match slash). A pattern whose scheme or authority contains a wildcard is refused.

### RUSTSEC-2023-0071 waiver

`Cargo.lock` currently records `rsa 0.9.10` through SQLx's disabled optional MySQL dependency. Cubby configures SQLx with default features disabled and enables SQLite only. `cargo tree --target all` confirms that RSA is not reachable in Cubby's active dependency graph.

`scripts/audit-rust.ps1` permits this one lockfile-only advisory and fails if RSA becomes reachable on any target. The waiver must be removed if SQLx stops recording the inactive package or if Cubby enables another SQLx database driver.

- Reviewed: 2026-10-21
- Next review: 2027-01-21 (or immediately if SQLx or the lockfile graph changes)

### RUSTSEC-2026-0459 waiver

SQLite before 3.54.0 can return stale process memory when it opens a database beside a crafted `-wal` file with a different page size (CVE-2026-119375). The fix is in `libsqlite3-sys` 0.39. Cubby gets SQLite through SQLx, and SQLx 0.9.0, the newest release, still requires `libsqlite3-sys` below 0.38, so there is no patched version to move to yet.

The attack needs Cubby to open a database and `-wal` pair that someone else wrote. Cubby opens two kinds of database:

- Its own history, in the app data folder or, in portable mode, in the `data` folder beside `cubby.exe`. Anyone who can write to the app data folder already runs as the user and can read the history directly. A portable copy kept where other accounts can write, such as a shared drive, does not have that protection, so keep portable Cubby in a folder only you can write to.
- A Ditto database the user picks to import. Cubby copies only the `.db` file to a new temporary name and opens that copy read-only and immutable, so SQLite never reads a `-wal` file for it.

`scripts/audit-rust.ps1` permits this advisory and fails if the Ditto import stops opening a private immutable copy. Remove the waiver as soon as SQLx accepts `libsqlite3-sys` 0.39 or newer.

- Reviewed: 2026-11-14
- Next review: 2026-12-14 (or immediately when a newer SQLx is released or Cubby opens another outside database)


## Win+V helper channel

When Win+V replacement is enabled, the helper signals the main process over
loopback UDP. The datagram must carry a per-session token Cubby generated for
that helper, arrive from a loopback address, and stay under a short rate limit.
A packet whose body is only `activate` is ignored. The token is passed as a
helper argument, so a same-user process that can read Cubby's command line can
still forge an activation.

## Privileged GitHub Actions pins

Third-party actions in privileged release, Store, and signing workflows must be pinned to a 40-character commit SHA with a version comment. See docs/RELEASE_CHECKLIST.md for reviewing Dependabot action pin updates.

## Sensitive clipboard handling

In addition to AES-256-GCM at rest, Cubby skips:

- Clipboard items tagged with Windows `ExcludeClipboardContentFromMonitorProcessing` (default on).
- Text that matches high-confidence secret heuristics such as private keys, cloud API tokens, and grouped payment-card numbers (off by default, enable in Settings; category logged, never content). This one is opt-in because a wrong guess silently drops a clip the user wanted to keep. Pastes larger than 8 KiB are still scanned: the first 8 KiB is checked so a secret marker at the start of a log or PEM is skipped. Content after that window is not scanned. While this setting is on, text that cannot be decoded as UTF-8 is treated as unscannable and is not stored; with the setting off, that text is stored like any other clip.
- A one-time seeded ignore list of major password-manager executables, editable in Settings.

Release Info logs persist under the application log directory. The History source-app filter is recorded there as `none` / `blank` / `set`, not as the selected application name.

## Clipboard history at rest

Cubby encrypts clipboard payloads, previews, source attribution, metadata, and image files with AES-256-GCM. Dedupe values use a keyed HMAC rather than a plain content hash. The random storage key is protected for the current Windows user with DPAPI and is never stored in plaintext. The backup passphrase, the key derived from it, the in-memory backup JSON, and the unlocked storage key are wrapped so they are overwritten when those values drop (SBS-983).

Existing plaintext history is migrated before the clipboard listener starts. Cubby fails closed if the key cannot be unlocked or migration cannot complete, preventing new history from being mixed into an unreadable or partially encrypted store.

Core Windows clipboard representations are retained together: Unicode text, HTML, RTF, and images. Auxiliary formats are encrypted in the same authenticated store. Cubby intentionally does not persist arbitrary private application formats because some contain process-specific handles or unsafe opaque data that cannot be replayed reliably.

Copying files is intentionally not recorded. A file payload is a reference to a path, not durable content, so a history entry for it can silently stop working after a move, a disconnect, or a target-app mismatch. Cubby ignores both physical (`CF_HDROP`) and virtual file payloads before reading any text they advertise, and a migration removes file rows written by earlier versions. The one exception is a screenshot tool that publishes a real bitmap alongside a file reference: Cubby keeps that as an image, never as a path.

## Backup path grants

`export_backup`, `import_backup`, and `import_from_ditto` accept a filesystem path only when the matching file dialog produced that exact string. The grant is in-memory, purpose-keyed, and compared without canonicalization. It is consumed by the first mutating call, expires after 60 seconds, and is dropped when the Settings window is destroyed, so an abandoned pick cannot stay writable until Cubby exits.

