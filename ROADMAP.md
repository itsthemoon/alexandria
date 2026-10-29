# Roadmap

What exists today, what comes next, and what Alexandria will not do. Dates are not promised;
each item ships when it works and has been tested on every supported system.

## v0.1.0 — Lite (released)

- Desktop app for macOS, Windows (x64 and ARM) and Linux x64.
- The Lite library (about 8 GB): Simple English Wikipedia, WikiMed, iFixit, Appropedia,
  survival guides (post-disaster, water, food, field medicine), a world map and the LFM3 1.7B
  model that answers questions and translates on your computer.
- Every piece of every file is checked against a signed list each time it is read.
- One-line install command, a personal shelf for your own photos, PDFs and notes.

## Next

- **Text search inside scanned PDFs.** About 1 in 10 survival guides are scans; they show by
  title only today. Adds on-device text recognition (OCR) during setup.
- **Search by meaning, not just by words.** So a question finds the right article even when it
  uses different words than the article does.
- **More tested translation languages.** A language is only offered after real sentences are
  checked both ways.
- **Updated medical sources** where a newer free edition exists.

## v0.2 — Full

- All of English Wikipedia with images (about 127 GB).
- A street-level map of the whole planet (about 130 GB), with every named street searchable.
- Needs a large external drive; the Lite edition stays the default.

## Later, if they can be done well

- Signed and notarized app downloads, so Mac and Windows stop warning on first open.
  This costs money every year and depends on support for the project.
- Offline address search (house numbers), which needs a separate address dataset.
- More collections proposed by the community (see [CONTRIBUTING.md](CONTRIBUTING.md)).

## Not planned, by design

- Accounts, sign-ins, telemetry or analytics.
- Automatic updates or background syncing. A release is a snapshot; you update by running the
  install command again.
- Any feature that needs the internet after the library is downloaded.
- Paid content or paid libraries. Some content (iFixit) is licensed for non-commercial use only.
