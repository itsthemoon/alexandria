---
name: alexandria-release
description: Use when cutting an Alexandria release (Lite or Full): code, library packs, signing, installers, publish.
---

# Releasing Alexandria

The release is a chain of gates. Each one proves something the next one relies on, so do them
in order and don't skip one to save time.

Placeholders: `<ver>` is the version without the `v` (e.g. `0.2.0`), `<lib>` is the library directory
on the release drive, `<dev-pub>`/`<dev-seed>` are a throwaway signing key made with `alex keygen`.
The release key never leaves the `release` environment in CI.

## 0. Before anything goes public

- Every change is tested: `cargo test --workspace` and every `crates/app/ui-tests/*.mjs` with `node`.
- The version comes from `crates/app/Cargo.toml`. `scripts/package-macos.sh` reads it, so bump the crate,
  never the script. After packaging, read `CFBundleShortVersionString` and the DMG name.
- For each feature and pack in the edition, check the README, ROADMAP and the decision records for it,
  and fix file counts and total sizes from the final edition list:
  `awk '!/^\s*(#|$)/{s+=$2} END {print s/1e9}' library/full.txt`.
- Test fixtures must not point at anyone's home: no real home streets, neighbourhoods or personal paths.
  `scripts/sync-public.sh` greps for private-looking text and refuses; fix the source, never the grep.

## 1. Packs on the drive (Full)

- Rebuild any pack that changed (e.g. `alex roads-build <extract.osm.pbf> roads-us.roads`).
- Copy it into `<lib>`, then in this order: `alex manifest <lib> v<ver>-full <date>` (re-hashes every file,
  minutes for ~280 GB), `alex sign <lib> <dev-seed>`, `alex verify <lib> <dev-pub>`.
- Copy `<lib>/MANIFEST` to `library/full-MANIFEST`.
- Update the pack's line in `library/full.txt`: size, `shasum -a 256` of the whole file, and one URL per
  1900 MiB part (`ceil(size / 1992294400)` parts named `<name>.partNN`). Run `scripts/test-hash-list.sh`.
- On exFAT, strip AppleDouble files before signing: `xattr -cr <lib>` and delete `._*`.

## 2. Prove it locally, end to end

- Build the app with the throwaway key: `ALEXANDRIA_PUBKEY=<dev-pub> scripts/package-macos.sh`.
- Run the real installer against the drive with no download: a scratch release dir of symlinks to the
  drive files, a `full-FILES.txt` with `file://` URLs into it, and `Alexandria-macos.tar.gz` from `dist/`.
  Run a scratch copy of `site/install.sh` with `ROOT_KEY` set to `<dev-pub>` (never edit the real one) with
  `ALEXANDRIA_BASE=file://<dir> ALEXANDRIA_DIR=<drive parent> ALEXANDRIA_EDITION=full </dev/null`.
  Every file must report "already here", verify must pass, finished search indexes must be skipped.
- Open the installed app and check each new feature by hand: a search, an article with images, the map
  at street level, a driving route with its steps. macOS asks once per new build for access to a
  removable drive; a person has to click Allow.

## 3. Push the code

1. Push the dev repo.
2. `scripts/sync-public.sh` copies the tree into the public checkout and greps for private text.
3. Read the public diff yourself (`git -C <public> diff --stat`, then the files), rerun the tests in the
   public checkout, commit there as the maintainer, push `main`.
4. Wait for CI to pass on every OS: `gh run watch <id> --exit-status`.
   Windows-only failures seen before: a test deleting a temp dir while a cached SQLite connection still
   holds a file open (os error 32; make temp cleanup best-effort), and Windows PowerShell 5.1 turning a
   child's stderr into a terminating error under `$ErrorActionPreference = 'Stop'` when the output is
   captured with `2>&1` (switch to `Continue` before calling a command that is expected to fail).
   Shell installer tests must build a fake app archive for every OS they run on, not only macOS.

## 4. Mirror and host the big files

- `library-data` is a GitHub release that holds files we host: 2 GiB per asset, 1000 assets, no total cap.
- Upstream files that expire (Wikipedia maxi, the Protomaps planet build) go through `mirror.yml`, one
  dispatch per file: `gh workflow run mirror.yml -f name=<name in full.txt> -f url=<upstream URL>`.
  Protomaps build URLs rotate, so mirror the planet before its dated build disappears. Check both URLs
  answer `200` with the expected `content-length` first (`curl -sI`). Each run waits on the `release`
  environment; approve it the same way as the signing runs (section 5). A run has a 6 hour cap, so use
  a fast upstream mirror for the biggest file.
- `mirror.yml` must already be on the public `main`: dispatch only after the code push.
- `scripts/mirror.sh` works inside a temp dir, not the checkout, so `gh` can't infer the repo from git
  there ("failed to run git: not a git repository"). The workflow sets `GH_REPO`; keep it on any step
  whose script changes directory before calling `gh`.
- Files we make ourselves (street index, US roads) are uploaded from the drive in 1900 MiB parts:
  `gh release upload library-data <name>.partNN --clobber`. Check every part's size after upload.

## 5. Tag, build, sign

1. Tag `v<ver>` and push it: `release.yml` builds macOS, Linux and Windows and creates a draft release.
   `v*` tags are immutable; a bad tag means a new version number.
2. Dispatch `library.yml -f tag=v<ver>` (Lite) and `full.yml -f tag=v<ver>` (Full). Both wait on the
   `release` environment. Approve with a JSON body (`-f` sends strings and fails with HTTP 422):
   `printf '{"state":"approved","comment":"signing v<ver>","environment_ids":[<id>]}' | gh api -X POST
   repos/<owner>/<repo>/actions/runs/<run>/pending_deployments --input -`, then confirm
   `pending_deployments` returns `[]`. `full.yml` streams every part, checks size and SHA-256, and refuses
   unless every file matches `library/full-MANIFEST` before it signs.
3. Dispatch `platform-test.yml -f run_id=<release run>` and read every screenshot.

## 6. Pin the installers

- From the draft release: `gh release view v<ver> --json assets --jq '.assets[] | "\(.name) \(.digest)"'`.
- Put the app archive digests in `site/install.sh` (`MAC_SHA`, `LINUX_SHA`) and `site/install.ps1`
  (`$AppSha`), together with `VERSION`/`$Version` in the same edit. Never publish a version bump without its
  pins: the live installer would ask for a release that isn't there yet.
- Run the installer tests, commit, sync, push, then push the site to `gh-pages`. After a Pages push, poll
  `gh api repos/<owner>/<repo>/pages/builds/latest --jq .status` until `built` before comparing files.

## 7. Review, publish, prove

- Security review before publishing: secrets and personal data in every tracked file and release asset,
  the installers, the verification core and IPC surface, and the CI workflows. Fix findings with tests.
- Publish the release. Run the real one-line install. For Full, reuse the drive: pass the drive's parent
  as `ALEXANDRIA_DIR` with the library already named `Alexandria`; it fetches the CI-signed MANIFEST and
  KEYS, skips files whose size and hash match, and verifies every byte against the release key.

## Directions pack: accuracy gate

A routing change ships only after a held-out comparison against OSRM on fresh random trips (a new seed
each round, run once): zero access, road-class, one-way or turn-restriction violations from the checker,
and distance/duration within a few percent for most trips, with every outlier explained. Develop on
other trips; never tune against the held-out set.
