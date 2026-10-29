# Alexandria

**An offline library for your computer.** Wikipedia, medical references, repair guides,
survival guides, a world map, translation and a small AI that answers from those books —
all stored on your own computer or drive, and all working with **no internet at all**.

Built for power cuts, travel, bad connections and emergencies. No account, no tracking,
nothing sent anywhere.

> **Preview release (v0.1.0).** It works, it is tested, and it has rough edges. Read
> [Known limits](#known-limits) before you rely on it for anything important.

## Install

You need about **12 GB free** (8 GB of library plus 4 GB for search) and an internet
connection for the download. After that, Alexandria never goes online.

**Mac or Linux** — open Terminal and paste:

```
curl -fsSL https://itsthemoon.github.io/alexandria/install.sh | bash
```

**Windows** — open PowerShell and paste:

```
irm https://itsthemoon.github.io/alexandria/install.ps1 | iex
```

The installer:

1. installs the app for you only (no admin password needed);
2. asks where to keep the library — your home folder, or an external drive;
3. downloads the library (about 8 GB; it resumes if your connection drops);
4. checks **every byte** against the release signature, and stops if anything is wrong;
5. builds search on your computer (5 to 30 minutes, depending on the computer).

Run the same command again later to update. Only changed files are downloaded.

You can read exactly what the installer does before running it:
[install.sh](site/install.sh) · [install.ps1](site/install.ps1).

### Prefer a normal download?

Download your system's file from the [Releases page](https://github.com/itsthemoon/alexandria/releases),
then run the install command above to get the library.

These downloads are not signed with a paid Apple or Microsoft certificate, so your
computer will warn you the first time:

- **Mac:** double-click the app; when macOS blocks it, open **System Settings → Privacy &
  Security**, scroll down, and click **Open Anyway**.
- **Windows:** when "Windows protected your PC" appears, click **More info → Run anyway**.

The install command avoids these warnings because files downloaded from a terminal are not
flagged.

## What's inside (Lite edition)

| Collection | What it is | Size |
|---|---|---|
| Simple English Wikipedia | 286,000 articles in plain English | 1.1 GB |
| WikiMed | 85,000 medical articles | 1.0 GB |
| iFixit | 246,000 pages of repair guides | 3.6 GB |
| Appropedia | 22,000 pages on sustainable, low-tech living | 0.6 GB |
| Survival guides | post-disaster, water, food preparation, field medicine | 0.8 GB |
| World map | countries, cities and coastlines (overview zoom) | 15 MB |
| LFM3 1.7B | the local AI model for Ask and Translate | 1.0 GB |

A **Full edition** (all of English Wikipedia with images and a street-level map of the
whole planet, about 280 GB) is planned for v0.2.

## Using it

- **Read / Browse** — search everything as you type, or browse a collection A–Z. Links
  inside articles work.
- **Ask** — ask a question in plain words. Alexandria finds the articles that cover it and
  the local AI writes a short answer, with the sources listed underneath. Always check the
  sources for anything important.
- **Map** — the world overview, with place search.
- **Translate** — paste text in about 30 languages and get it translated on your computer.
- **My files** — keep your own documents and photos next to the library. These are yours
  and are not checked against the signature.

## Privacy

- No account, no analytics, no update checks. The app contains no networking code.
- The local AI runs as a separate program on your computer, and it opens no network
  connections either (we checked: no listening or outgoing sockets).
- Nothing you search, ask or translate leaves your computer.

This is a promise backed by how the app is built, not by an operating-system sandbox.
On macOS, the system's web view can technically make connections on its own, outside the
app's control. Alexandria never asks it to.

## What "verified" means

Every library file is listed in a `MANIFEST` signed by the project's release key. Every
time Alexandria reads from the library, it checks that piece against the signed hash
first. A changed, corrupted or swapped file is refused — the app shows an error instead
of the content.

Signing uses two keys, so no single person or leak can quietly take over:

- a **root key**, kept offline by the maintainer, which only signs the list of allowed
  release keys (`KEYS`);
- a **release key**, held by GitHub and usable only after a maintainer approves a release,
  which signs each library.

"Verified" means the files are exactly what the project published. It does **not** mean
the content is true, that a computer with malware is safe, or that a stolen key could not
sign a bad release.

Two files are made on your computer rather than signed by us: the search index (built
during setup from verified files) and the working copy of the AI model. Both are checked
when they are written, not every time they are read.

## Known limits

- **The AI is small.** LFM3 1.7B is fast on ordinary laptops, but its answers can be
  incomplete or slightly wrong. It is told to answer only from the library, and it says
  "The library excerpts do not cover that" when they don't. Check the sources.
- **Translation is good, not perfect.** Spanish and French were tested both ways and are
  reliable for simple sentences; one test turned "boil the water" into "cook the water" in
  Spanish. Lingala and other low-resource languages are not offered, because every model
  we tested got them wrong.
- **The Lite map stops at country and city level.** Street maps come with the Full edition.
  Map search finds places and street names, not house numbers.
- **Scanned PDFs** in the survival guides (about 1 in 10) show by title only; their text
  can't be searched yet.
- **Unsigned builds:** see the warnings above. Apple and Microsoft certificates cost money
  every year; the project may add them later.
- **Tested on:** macOS on Apple Silicon (and the Intel build under Rosetta), Windows x64,
  Windows on ARM and Linux x64 in automated builds. Not yet tested on a physical Intel Mac
  or Windows-on-ARM laptop.

## Add a collection

Every file in the library is one line in [`library/lite.txt`](library/lite.txt): name,
size, SHA-256 and download link. To propose a new collection — for example a
[Kiwix](https://library.kiwix.org) archive, the open project that packages Wikipedia and
other sites for offline use — add a line and open a pull request. Once a maintainer
approves a release, it is downloaded, checked and signed into the library.

## Build from source

```
cargo test --workspace                 # unit and integration tests
cargo build --release -p alexandria-cli
scripts/build-llama.sh macos-arm64 target/llama/macos-arm64   # the local AI runner
scripts/e2e.sh <library> <root-key-hex>                        # end-to-end checks
```

The pieces: `crates/core` (signatures and verified reads), `crates/zim` (offline archive
reader), `crates/pmtiles` (map reader), `crates/search` (search index), `crates/cli`
(`alex`, the publisher tool) and `crates/app` (the desktop app, built with Tauri).
Security reports: see [SECURITY.md](SECURITY.md). What's planned: [ROADMAP.md](ROADMAP.md).
How to help: [CONTRIBUTING.md](CONTRIBUTING.md).

## Licenses

Alexandria's code is Apache-2.0. The content keeps its own licenses: Wikipedia, WikiMed
and Appropedia (CC BY-SA), iFixit (CC BY-NC-SA), map data © OpenStreetMap contributors
(ODbL) via Protomaps, offline archives packaged by Kiwix. The LFM3 model is under the
LFM Open License v1.0 (free to use unless your company earns over $10M a year).

Alexandria is an independent project. It is not affiliated with or endorsed by the Wikimedia
Foundation, iFixit, Appropedia, Kiwix, OpenStreetMap, Protomaps or Liquid AI; their names are
used only to say where the content comes from. Because iFixit's license is non-commercial,
Alexandria and its libraries are free and must stay free.
