# Packaging & Release (P33)

How to produce signed installers for Windows / macOS / Linux and ship auto-updates.

## What's in place

| File | Purpose |
|------|---------|
| `src-tauri/tauri.conf.json` | Bundle metadata, `.caspian` file association, `createUpdaterArtifacts: true`, `plugins.updater` (pubkey placeholder). |
| `.github/workflows/release.yml` | One GitHub runner per OS; builds native target, signs, uploads to a draft release. |
| `.github/workflows/ci.yml` | Headless gate (`cargo test --lib` + frontend build/typecheck/lint). No webview needed. |
| `scripts/build.sh` | Local single-platform build smoke test. |
| `scripts/setup-icons.sh` | Expand `app-icon.png` → full platform icon set. |
| `scripts/release.sh` | Tag + push to trigger the release workflow. |
| `src-tauri/src/updater.rs` | `check_for_update` / `install_update` IPC (compiled only under `--features tauri`). |

## One-time human prerequisites

1. **App icon.** Commit a 1024×1024 `app-icon.png` at the repo root. CI runs
   `pnpm tauri icon` to generate `src-tauri/icons/{icon.icns,icon.ico,32x32.png,…}`.
   `tauri.conf.json`'s `bundle.icon` array references that generated set — the
   build fails without it.

2. **Updater keypair.** Generate once and keep the private key secret:
   ```bash
   pnpm tauri signer generate -w ~/.tauri/caspian.key
   # prints the PUBLIC key — paste it into tauri.conf.json `plugins.updater.pubkey`
   ```
   Store the private key (PEM) as the `TAURI_SIGNING_PRIVATE_KEY` repo secret.
   If you set a password, also set `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.

## CI secrets

| Secret | Platform | Notes |
|--------|----------|-------|
| `TAURI_SIGNING_PRIVATE_KEY` | all | Updater artifact signing. |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | all | Only if the key has a password. |
| `TAURI_WINDOWS_CERTIFICATE` | Windows | Base64 of a `.pfx` Authenticode cert. |
| `TAURI_WINDOWS_CERTIFICATE_PASSWORD` | Windows | Cert password. |
| `APPLE_CERTIFICATE` | macOS | Base64 of the `.p12` Developer ID cert. |
| `APPLE_CERTIFICATE_PASSWORD` | macOS | Cert password. |
| `APPLE_SIGNING_IDENTITY` | macOS | e.g. `Developer ID Application: …`. |
| `APPLE_ID` | macOS | Apple ID email for notarization. |
| `APPLE_PASSWORD` | macOS | App-specific password. |
| `APPLE_TEAM_ID` | macOS | 10-char team id. |

## Update endpoint

`plugins.updater.endpoints` points at
`https://releases.caspianflow.app/{{target}}/{{arch}}/{{current_version}}`.
Replace with your host (a static bucket mirroring the release artifacts, or a
small server that returns the signed `.json` manifest). The endpoint URL is
baked into the signed binary — change it **before** the first release.

## Local build

```bash
scripts/setup-icons.sh      # if app-icon.png changed
scripts/build.sh            # host-platform installer
```

## Cut a release

```bash
scripts/release.sh v0.2.0   # tags + pushes → release.yml builds all 3 platforms
```

> Note: the real three-platform build needs the webview system libraries
> (webkit2gtk on Linux) and the signing certs/keys above. The sandbox used for
> development cannot run `tauri build` (no webkit2gtk), so the actual cross-platform
> build is verified in CI / on a developer machine — not in the sandbox.
