# Rollback Plan

Rollback is not executed by this script. Run commands only after explicit user confirmation.

## Global CLI

```bash
npm install -g @mindfoldhq/trellis@unknown
trellis --version
```

## Local Dependency

Use the existing package manager. Verify commands before running:

```bash
npm install --save-dev @mindfoldhq/trellis@unknown
```

## `.trellis` Snapshot

Restore files from:

```text
E:\SynologyDrive\git_files\agent-switch\.trellis\archive\upgrades\2026-07-09-trellis-unknown-to-0.6.6\snapshot
```

Review `snapshot-manifest.json` before copying files back. Do not delete current `.trellis` content unless the user explicitly confirms.

## Target Version That Was Attempted

0.6.6
