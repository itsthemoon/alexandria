# Security

## Reporting a problem

Please report security issues privately through GitHub's
[security advisories](https://github.com/itsthemoon/alexandria/security/advisories/new)
rather than in a public issue. You'll get a reply within a week.

## What Alexandria protects against

**A changed library file.** Every file is listed in a `MANIFEST` with its size and a
SHA-256 hash for each 1 MiB piece. The app checks each piece against that list every time it
reads it, so a corrupted, edited or swapped file is refused at the moment of use, not just at
install. Symlinks are refused.

**A forged library.** The `MANIFEST` is signed with Ed25519. The app has one public key built
in, the *root key*. A library is accepted when:

- the root key signed the `MANIFEST`, or
- a release key signed it, and that key is listed in a `KEYS` file the root key signed.

The two kinds of signature use different domain prefixes, so one can't be passed off as the
other. The root key is kept offline by the maintainer. Release keys are GitHub Actions
secrets that can only be used after a maintainer approves a release.

**Hostile content in an article.** Articles are cleaned before display (no scripts, styles,
forms, frames or event handlers), shown in a frame that cannot run scripts, under a
content-security policy that blocks network loads. The reading, map and search code checks the
size of every structure it reads from a file, and has limits on decompression.

**Network use.** The app contains no networking code. The local AI runner
(`llama-completion` from llama.cpp, built from a pinned commit) is started with an empty
environment and opens no sockets. CI checks the socket part on Linux.

## What it does not protect against

- **The first download.** The install command checks the app download against a SHA-256
  written into the install script, then the app verifies the library. Both the script and the
  app come from the project's GitHub account, so this only works if GitHub and that account
  are not compromised, like any `curl | bash` installer. Apps are not signed with paid Apple or
  Microsoft certificates. If you download from the Releases page instead, compare the file's
  SHA-256 with the one in `site/install.sh` or `site/install.ps1`.
- **Malware on your computer.** A program running as you can change the app, or the files
  below, and can change what you see.
- **Files made on your computer.** The search index (built during setup) is only used to find
  and order results. Every title, preview, PDF text and Ask excerpt you see is read from the
  verified library, so a changed index can hide or reorder results but cannot change what they
  say. The working copy of the AI model is checked against the signed hashes every time it is
  used; a program running as you could still swap it in the moment between that check and the
  AI reading it.
- **Wrong content.** Verified means "exactly what the project published". It does not mean
  true.
- **A stolen key.** A stolen release key can sign a bad library until the root key removes
  it from `KEYS`. A stolen root key is a full compromise.
- **Older signed copies.** The app does not yet refuse an older `KEYS` or `MANIFEST` that was
  validly signed. Someone who can write to the drive could put back an older library, or an
  older `KEYS` that still lists a removed key. v0.1.0 is the first release, so no older copy
  exists yet; a minimum release version will be built into the app before any key is removed.
- **My files.** Files you add to `Alexandria-Personal` are not signed or checked. Photos there
  are decoded by the system web view and PDFs open in your usual viewer, so a crafted file put
  there by someone with access to the drive reaches that code.
- **Bugs in third-party parsers.** llama.cpp parses the model file, pdf-extract parses PDF
  guides, and the system web view renders articles. They only ever see verified library files,
  but llama.cpp and the web view are memory-unsafe code outside this project.

There is no operating-system sandbox around the app. Its protections come from its own code
and from the web view's content policy.

## Checking a library yourself

```
alex verify <library-folder> <root-public-key-hex>
```

This re-reads every byte and checks it against the signed manifest. The root public key
is printed in `site/install.sh`.
