# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- Repository identifiers containing characters that require percent-encoding in URLs (e.g. spaces, emoji) are now accepted and served correctly. NIP-01 places no restriction on `d` tag values and NIP-34 only recommends kebab-case without mandating it, so rejecting non-kebab identifiers was overly strict. Identifiers are stored verbatim on disk and percent-encoded when used in URLs, per the `nostr://` clone URL spec formalised in [NIP-34 PR #2312](https://github.com/nostr-protocol/nips/pull/2312) and the GRASP-01 HTTP path spec. The landing page clone URL now also correctly percent-encodes the identifier.

### Changed

- Remove arbitrary default max connections limit; when `NGIT_MAX_CONNECTIONS` is unset the relay imposes no connection cap, deferring to OS fd limits and infrastructure controls

## [1.0.1] - 2026-02-27

### Fixed

- Push authorization now correctly ignores `refs/tags/<name>^{}` peeled-tag entries in state events (kind 30618). These entries are git's internal notation for the dereferenced commit behind an annotated tag and are never sent as part of a push. Previously, their presence in the state event caused `can_satisfy_state` to reject valid annotated-tag pushes because the would-be ref state after the push did not include the spurious `^{}` entry, making the exact-equality check fail.

### Changed

- Push auth rejections now send the reason to the git client via ERR pkt-line (e.g. "authorisation failed: No state events in purgatory") instead of a generic HTTP 403, so users see actionable error messages directly in their terminal

## [1.0.0] - 2026-02-26

Initial release of ngit-grasp, a GRASP relay implementation in Rust.

[unreleased]: https://gitworkshop.dev/npub15qydau2hjma6ngxkl2cyar74wzyjshvl65za5k5rl69264ar2exs5cyejr/ngit-grasp/compare/v1.0.1...HEAD
[1.0.1]: https://gitworkshop.dev/npub15qydau2hjma6ngxkl2cyar74wzyjshvl65za5k5rl69264ar2exs5cyejr/ngit-grasp/compare/v1.0.0...v1.0.1
[1.0.0]: https://gitworkshop.dev/npub15qydau2hjma6ngxkl2cyar74wzyjshvl65za5k5rl69264ar2exs5cyejr/ngit-grasp/releases/tag/v1.0.0
