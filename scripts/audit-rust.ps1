$ErrorActionPreference = 'Stop'

$releaseTargets = @(
    'x86_64-pc-windows-msvc',
    'aarch64-pc-windows-msvc'
)

foreach ($target in $releaseTargets) {
    # Cargo writes download/progress messages to stderr. Keep those out of the
    # parsed tree so an inert lockfile download cannot look like a reachable node.
    $tree = cargo tree --manifest-path src-tauri/Cargo.toml --locked --target $target | Out-String
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to inspect the Rust dependency tree for $target."
    }

    if ($tree -match '(?m)(?:^|\s)rsa v[^\s]+(?:\s|$)') {
        throw "RUSTSEC-2023-0071 is reachable for $target; remove the waiver."
    }
}

# Cargo.lock includes SQLx optional MySQL driver packages even though Cubby builds
# SQLx with default features disabled and SQLite only. The tree check above makes
# this waiver fail closed if RSA ever becomes reachable in a release installer.
# RUSTSEC-2026-0459 (SQLite before 3.54.0 can return stale memory when a database
# is opened beside a crafted -wal file). SQLx 0.9.0 still caps libsqlite3-sys below
# the patched 0.39, so there is no release to move to yet. The only database Cubby
# opens that someone else may have written is the Ditto import, which copies the
# single .db file to a fresh temp name and opens it immutable, so no -wal is read.
# Fail closed if that open ever changes. See SECURITY.md.
$dittoImport = Get-Content -Raw src-tauri/src/ditto_import.rs
if ($dittoImport -notmatch '(?s)std::fs::copy\(ditto_db_path, &temp_path\).*?\.read_only\(true\)\s*\.immutable\(true\)') {
    throw 'The Ditto import no longer opens a private immutable copy; remove the RUSTSEC-2026-0459 waiver.'
}

cargo audit --file src-tauri/Cargo.lock --ignore RUSTSEC-2023-0071 --ignore RUSTSEC-2026-0459
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}
