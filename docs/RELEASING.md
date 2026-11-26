# Releasing KittyTooth

Pushing a `v*` tag (for example `v1.3.0`) runs `.github/workflows/release.yml`, which builds
and publishes a GitHub release with:

| Asset | What it is |
| --- | --- |
| `KittyTooth_<v>_x64-setup.exe` | Windows installer (also what installed copies update to) |
| `KittyTooth_<v>_x64_portable.exe` | Windows portable exe |
| `KittyTooth_<v>_amd64.AppImage` | Linux AppImage |
| `*.sig` | Minisign signatures of the installer and AppImage (in-app updates) |
| `latest.json` | The update manifest the app polls |
| `SHA256SUMS.txt` | Checksums of everything above |

Before tagging, bump the version in `Cargo.toml` (workspace), `package.json` and
`src-tauri/tauri.conf.json`.

## How in-app updates work

The app fetches
`https://github.com/SlushyP1e/KittyTooth/releases/latest/download/latest.json` on launch and
every 6 hours (Settings → Updates → "Check for updates automatically"). If the version there is
newer, a small "Update available" pill appears in the sidebar and Settings → Updates shows the
release notes.

- **Installed (setup.exe) and AppImage copies** get "Install and restart". The download is
  checked against the public key in `src-tauri/tauri.conf.json` before anything is replaced.
- **Portable copies** get "Open release page", because a running exe can't replace itself.
- Nothing installs without a click. Stable builds are never offered pre-releases.
- Draft releases and `workflow_dispatch` runs don't produce `latest.json`, and GitHub's
  `releases/latest` ignores drafts and pre-releases, so only published releases reach users.

The release notes shown in the app are the release body GitHub generates when the tag is
pushed. Editing the release body afterwards doesn't change `latest.json`.

## One-time setup: the update signing key

Update files are signed with a minisign key. The public half lives in `tauri.conf.json`; the
private half lives only in GitHub secrets and in your own backup. **Never commit the private
key.**

Until this is done, releases still build and publish normally. The workflow prints a warning
and skips the `.sig` files and `latest.json`, and the app treats every copy like the portable
one (it links to the release page instead of installing).

1. **Generate the key pair** (PowerShell, in the repo folder):

   ```powershell
   npx tauri signer generate -w "$env:USERPROFILE\.tauri\kittytooth.key"
   ```

   Pick a password when asked and store it in your password manager. This writes two files:
   `kittytooth.key` (private) and `kittytooth.key.pub` (public).

2. **Back up `kittytooth.key` and its password** somewhere safe and offline. If you lose either,
   installed copies can't verify future updates and everyone has to download one release by
   hand.

3. **Put the public key in the app.** Open `kittytooth.key.pub`, copy its whole contents (one
   long line), and paste it over the placeholder in `src-tauri/tauri.conf.json`:

   ```json
   "plugins": {
     "updater": {
       "pubkey": "<contents of kittytooth.key.pub>",
   ```

   Commit that change through a normal PR. The public key is safe to commit.

4. **Add the two repository secrets.** On GitHub: the repo → Settings → Secrets and variables →
   Actions → New repository secret.

   | Name | Value |
   | --- | --- |
   | `TAURI_SIGNING_PRIVATE_KEY` | The whole contents of `kittytooth.key` |
   | `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | The password from step 1 |

   Or with the GitHub CLI:

   ```powershell
   Get-Content "$env:USERPROFILE\.tauri\kittytooth.key" -Raw | gh secret set TAURI_SIGNING_PRIVATE_KEY
   ```

   ```powershell
   gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD
   ```

   (The second command asks for the value, so the password stays out of your shell history.)

5. **Check the next release.** After pushing the tag, the release should list the two `.sig`
   files and `latest.json`, and the workflow run should have no "No in-app update files"
   warning.

Copies older than the first release with a real public key (1.2.0 and earlier have no updater
at all) need one manual download; from then on they update in the app.
