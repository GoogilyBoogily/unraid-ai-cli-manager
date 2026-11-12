Desktop preview for server review. Don't publish this draft as a supported beta yet.

The Rust manager provides immutable backups, SQLite backup/validation, complete standalone packages, explicit per-tool legacy takeover, native settings, and bounded lifecycle handling.

The shipped vendor target catalog is intentionally empty. Existing valid installations can be adopted, backed up, and restored. Fresh installs and manager updates remain blocked until reviewed targets are added with server evidence.

Before publishing, complete docs/server-validation.md, including Codex native update suppression across CLI startup and a running daemon. No vendor executables or private server data are bundled.
