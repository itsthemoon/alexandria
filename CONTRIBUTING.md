# Contributing to Alexandria

Thanks for helping. Alexandria is meant for people who may have no internet when they need it,
so every change is judged by one question: does it keep the app simple, offline and trustworthy?

## Ways to help

- **Report a bug.** Open an issue with your system (macOS / Windows / Linux, and chip), the app
  version, what you did, what you expected and what happened. A screenshot helps. Don't include
  personal information or files you would not want public.
- **Propose a collection.** See "Adding a collection" below.
- **Test on your computer.** Reports from real machines (older laptops, Intel Macs, Windows on
  ARM) are especially useful.
- **Fix something.** Small, focused pull requests are easiest to review.
- **Security problems** go through [SECURITY.md](SECURITY.md), never a public issue.

## Adding a collection

Every file in the library is one line in [`library/lite.txt`](library/lite.txt):

```
<name> <size in bytes> <sha256> <https download link>
```

A collection can be added when:

1. its license allows free redistribution (for example CC BY, CC BY-SA, public domain, or a
   non-commercial license, since Alexandria is free);
2. the download link is a stable, versioned file (for example a dated
   [Kiwix](https://library.kiwix.org) archive), so the hash never changes under it;
3. it is useful when the internet is down: practical, reference or medical knowledge first;
4. it is in a format the app reads: ZIM archives, PDF or plain text.

Open a pull request with the new line, the license, and one or two sentences on why it belongs.
Once a maintainer approves a release, the file is downloaded, checked against your hash and
signed into the library.

## Code

- Run the tests before opening a pull request:

  ```
  cargo test --workspace
  node crates/app/ui-tests/rank.test.mjs
  node crates/app/ui-tests/map.test.mjs
  ```

- **Keep it simple.** Prefer deleting code to adding it. New dependencies need a strong reason.
- **No network code in the app,** no telemetry, no accounts, no background tasks.
- **Every byte shown to the user must come through the verified reader** (`crates/core`).
  Unsigned local files (the search index, the personal shelf) may find or list things, but
  never supply the text people read.
- **No comments in code.** Use clear names instead; explain decisions in the pull request.
- **Never claim more than the code does.** Words in the app and README must be literally true.
- Match the existing style; `cargo fmt` for Rust.

## Releases

Releases are built by GitHub Actions from a version tag. The library is signed only after a
maintainer approves the release in the protected `release` environment. The root key that
approves release keys is kept offline.

## License

By contributing, you agree that your contribution is licensed under the
[Apache License 2.0](LICENSE), the same as the rest of the code.

Please follow the [code of conduct](CODE_OF_CONDUCT.md).
