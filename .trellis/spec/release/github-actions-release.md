# GitHub Actions Release

## 1. Scope / Trigger

This contract applies when publishing an Agent-Switch Windows release. A pushed `v*` tag triggers `.github/workflows/release.yml`; GitHub's Windows runner is the single owner of distributable MSI compilation, signing, and upload.

Development changes must still receive checks proportional to their risk. This rule removes only the redundant local `pnpm tauri build` / MSI packaging step from the release procedure.

## 2. Signatures

- Workflow trigger: push tag matching `v*`.
- Version declarations that must match:
  - `package.json` -> `version`
  - `src-tauri/Cargo.toml` -> `[package].version`
  - `src-tauri/Cargo.lock` -> the `agent-switch` package version
  - `src-tauri/tauri.conf.json` -> `version`
- Required GitHub secrets:
  - `TAURI_SIGNING_PRIVATE_KEY`
  - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
  - `GITHUB_TOKEN` is supplied by GitHub Actions.

## 3. Contracts

Release sequence:

1. Confirm the latest GitHub Release and select the next semantic version.
2. Update the four version declarations and verify they are identical.
3. Reuse the development task's completed checks; run only lightweight release checks locally (`cargo metadata --no-deps`, JSON parsing, version equality, `git diff --check`).
4. Commit the version bump, create an annotated `v<version>` tag, then atomically push `main` and the tag:

   ```powershell
   $version = "0.3.2"
   git tag -a "v$version" -m "Agent-Switch v$version"
   git push --atomic origin main "v$version"
   ```

5. Monitor the `Release` workflow through completion.
6. Verify that the new GitHub Release is latest and contains:
   - `Agent-Switch_<version>_x64_en-US.msi`
   - `Agent-Switch_<version>_x64_en-US.msi.sig`
   - `latest.json`

Do not upload a locally built MSI when the workflow can build the same tagged commit. This avoids machine-dependent artifacts and unnecessary local CPU, memory, and disk consumption.

## 4. Validation & Error Matrix

| Condition | Required action |
|---|---|
| Four version declarations differ | Stop before tagging and make them identical. |
| `origin/main` diverged from local `main` | Stop; reconcile history without force-pushing. |
| Release tag already exists | Stop; never move or overwrite a published version tag. |
| GitHub workflow fails | Inspect the failed job; do not manually publish an unverified local MSI as a substitute. |
| MSI exists but `.sig` or `latest.json` is missing | Release is incomplete; fix signing/updater configuration and publish a new version. |
| Local `src-tauri/target` consumes excessive space | Run `cargo clean --manifest-path src-tauri/Cargo.toml`; this does not affect cloud builds. |

## 5. Good / Base / Bad Cases

- Good: development checks pass, versions match, atomic tag push succeeds, cloud workflow uploads all three artifacts, and the Release becomes latest.
- Base: a documentation-only release uses lightweight local checks and still delegates MSI packaging to GitHub Actions.
- Bad: run a full local release build only to upload that MSI manually, while also triggering the cloud workflow for the same tag.

## 6. Tests Required

- Parse both JSON files and assert their versions match the Cargo package and lockfile.
- Run `cargo metadata --manifest-path src-tauri/Cargo.toml --no-deps --format-version 1`; it reads metadata without compiling the application.
- Run `git diff --check` before the release commit.
- After pushing, assert `HEAD`, `origin/main`, and `v<version>^{}` resolve to the intended commit.
- Query the GitHub Release and assert the MSI, signature, and `latest.json` assets are uploaded.

## 7. Wrong vs Correct

### Wrong

```powershell
pnpm tauri build
# Upload the local MSI manually, then also push the release tag.
```

This duplicates a costly build, leaves large local `target` artifacts, and creates ambiguity over which binary corresponds to the tagged source.

### Correct

```powershell
cargo metadata --manifest-path src-tauri/Cargo.toml --no-deps --format-version 1
git diff --check
$version = "0.3.2"
git tag -a "v$version" -m "Agent-Switch v$version"
git push --atomic origin main "v$version"
gh run watch <run-id> --repo XM-Chen/agent-switch --exit-status
```

GitHub Actions builds, signs, uploads, and marks the tagged release as latest.
