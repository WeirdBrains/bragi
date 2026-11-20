# Releasing Bragi

Bragi releases with the shared [Weird Brains](https://weirdbrains.com) release script, vendored as
`scripts/weirdnet_release.py` and configured in `release.toml`. Never edit the vendored copy.

## Steps

1. On a clean, pushed `main`, with `CARGO_TARGET_DIR` exported to a scratch directory:
   `python3 scripts/weirdnet_release.py --dry-run "Title" "Added: ..."`, then without `--dry-run`. It runs
   the checks, commits `release: vX.Y.Z`, tags `vX.Y.Z` and pushes `main` and the tag in one push.
2. The tag starts `.github/workflows/release.yml`, which builds every platform on GitHub-hosted runners
   and uploads the files as workflow artifacts. Download them into `dist/`:
   `gh run download RUN -D dist/ && find dist -mindepth 2 -type f -exec mv -t dist/ {} +`.
3. Check every file for build-machine paths and other private details before uploading.
4. `python3 scripts/weirdnet_release.py github X.Y.Z` creates the GitHub release with the files and a
   `SHA256SUMS`, and `python3 scripts/weirdnet_release.py assets X.Y.Z` checks it.
5. Update `packaging/arch/bragi-bin` (`pkgver`, `_ver`, both tarballs' sha256 from `SHA256SUMS`,
   `.SRCINFO`) and `packaging/nix/release.json`, then
   `python3 scripts/weirdnet_release.py taur X.Y.Z --dry-run` and again without `--dry-run`.
6. Bump `latestVersion` in `site/lib/site.dart` and push; check https://bragi.weirdbrains.com/download/.

## What each platform gets

| Platform | Files | Signing |
| --- | --- | --- |
| Linux x86_64, ARM64 | `tar.gz`, `zip`; `deb`, `rpm`, Arch `pkg.tar.zst` as `bragi` (desktop entry, icons) and `bragi-standalone` (binary only); TAUR `bragi-bin`; Nix flake (`bragi`, `bragi-standalone`) | none |
| macOS (universal) | `Bragi.app` in a `zip` and a `dmg`; the bare binary in a `tar.gz` | ad-hoc only, not notarized |
| Windows x86_64, ARM64 | standalone `exe` and `zip`; per-user `msi` and a `setup.exe` built with WiX 7 | none |
| Android | one `apk` per ABI; an `aab` when the release key is set | release key from the `BRAGI_ANDROID_*` secrets, else the debug key (`*-debug-signed.apk`) |
| iOS | unsigned `ipa` for AltStore, SideStore or Sideloadly | none (you sign it with your own Apple ID) |

The BSDs are not built: the desktop shell uses GPUI, which does not build there yet, and the phone app is
Flutter.

## WiX

The Windows installers use WiX Toolset 7, which is licensed under the Open Source Maintenance Fee EULA v1.1
(https://docs.firegiant.com/wix/osmf/). Weird Brains LLC accepted it on 2026-11-12, so every `wix`
command passes `-acceptEula wix7`. The WiX CLI MSI is pinned by version and SHA-256 in the workflow.

## Android signing

The workflow signs with a release key only when these repository secrets exist:
`BRAGI_ANDROID_KEYSTORE_BASE64` (the keystore, base64), `BRAGI_ANDROID_KEYSTORE_PASSWORD`,
`BRAGI_ANDROID_KEY_ALIAS` and `BRAGI_ANDROID_KEY_PASSWORD`. Without them the apks are signed with the
runner's debug key, which changes between builds, so a new version has to be installed fresh rather than
over an older one.

## minisign

The `minisign` job signs every release file when the `MINISIGN_SECRET_KEY` and `MINISIGN_PASSWORD` repository secrets are set, and uploads the `.minisig` files as the `bragi-minisign` artifact to attach to the release. Without them it skips with a notice; there is no minisign key yet.
