# GRASP-06 Implementation Plan

**Target spec**: [GRASP-06](https://github.com/DanConwayDev/grasp/blob/main/06.md) (draft)
**Design doc**: [`docs/explanation/grasp-06-contributor-pr-submission.md`](../docs/explanation/grasp-06-contributor-pr-submission.md)
**Baseline**: master with the PR purgatory work already merged.
**Audience**: the implementing agent. Read the spec and design doc first, then this.

This document sequences the work. No tests are specified here — test planning is a separate workstream once the implementation shape is agreed.

---

## Ground rules

1. **Do not build features the spec does not require.** In particular: no NIP-98 auth on `/prs/`, no allowlist/quota/PoW in v1, no deletion-request hooks, no object-pool dedup, no `/prs/` replication.
2. **Do not regress the standard `/<npub>/<identifier>.git` endpoint.** Its auth model (`authorize_push` / maintainer set) is unchanged.
3. **Reuse what exists.** `PrPurgatoryEntry`, `refs/nostr/<event-id>` handling, `process_newly_available_git_data`, the existing receive-pack subprocess plumbing, `process_pr_with_git_data`, and the cross-owner sync in `src/git/sync.rs` already cover most of this feature. The new code path is thin on top.
4. **Gate everything behind `NGIT_GRASP06_ENABLE`.** When disabled, `/prs/*` returns 404 and the event-acceptance relaxation does not apply. Default must be `false`.
5. **Keep the `/prs/` tree on disk separate from the standard tree.** `<git_data_path>/prs/<hex-pubkey>/<identifier>.git`. This makes exclusion from other subsystems a single path check.
6. **Follow the existing style.** Config env var naming (`NGIT_GRASP06_*`), `AGENTS.md` config-sync rules (code + `.env.example` + docs + NixOS module — four places), inline authorization (no pre-receive hooks), NIP-11 advertisement.

---

## Phase order

Each phase is independently mergeable. Later phases assume earlier phases are in place. Run `cargo build` and `cargo clippy --all-targets` after every phase; do not commit phases that do not at least build.

1. Config + feature flag
2. Disk layout + path helpers
3. HTTP routing and empty-repo fetch synthesis
4. `/prs/` receive-pack handler
5. Purgatory placeholder scoping
6. Event-acceptance relaxation in PR policy
7. Cross-service mirror on purgatory release
8. Exclusion from cleanup / sync / listings
9. NIP-11 advertisement
10. Documentation hygiene and release notes

---

## Phase 1 — Config + feature flag

**Goal**: one bool flag, default `false`, wired into `Config`, `.env.example`, docs, NixOS module.

**Change**:

- `src/config.rs` — add:
  ```rust
  /// Enable GRASP-06 contributor PR submission endpoint at /prs/<npub>/<identifier>.git
  #[arg(long, env = "NGIT_GRASP06_ENABLE", default_value_t = false)]
  pub grasp06_enable: bool,
  ```
  Place it alongside the other `NGIT_*` flags, after the archive block is a natural home.

- `.env.example` — add the usual four-line comment block documenting CLI, default, and env var.

- `docs/reference/configuration.md` — add an entry under the same ordering convention as other flags. Describe: what it enables, the default (off), and a one-line summary of the security model ("unauthenticated — relies on signed PR/PR Update events for validity").

- `nix/module.nix` — add `grasp06Enable` to `instanceOptions` as `mkOption { type = types.bool; default = false; description = "..."; }` and wire it into the environment block as `NGIT_GRASP06 = toString cfg.grasp06Enable;`. Match the existing naming pattern for other env vars in that file.

**Deliverable**: a no-op flag that compiles, is plumbed through `Config`, and defaults to off.

**Do not**: add any downstream logic yet. Just the flag.

---

## Phase 2 — Disk layout + path helpers

**Goal**: a small module exposing the `/prs/` filesystem conventions, independent of HTTP.

**Create**: `src/grasp06/mod.rs` with submodules declared for later phases:

```rust
//! GRASP-06 — Contributor Pull Request Submission.
//!
//! Spec: https://github.com/DanConwayDev/grasp/blob/main/06.md
//! Design: docs/explanation/grasp-06-contributor-pr-submission.md

pub mod endpoint;      // URL parsing
pub mod paths;         // on-disk path helpers (this phase)
pub mod fetch;         // empty-repo synthesis (later phase)
pub mod receive;       // receive-pack handler (later phase)
pub mod mirror;        // cross-service mirror (later phase)
```

**In `paths.rs`**:

- Constant: `pub const PRS_URL_PREFIX: &str = "prs";` and `pub const PRS_DISK_PREFIX: &str = "prs";` (spec-fixed).
- `pub fn prs_repo_path(git_data_path: &Path, submitter_hex: &str, identifier: &str) -> PathBuf` returning `<git_data_path>/prs/<submitter_hex>/<identifier>.git`.
- `pub fn is_prs_repo_path(path: &Path, git_data_path: &Path) -> bool` — used by cleanup/sync exclusion checks.
- `pub fn prs_base_path(git_data_path: &Path) -> PathBuf` returning `<git_data_path>/prs`.

Use `hex` (not `npub`) on disk to match the existing scheme where pubkeys are stored as hex under the owner directory. (Look at `src/git/authorization.rs` and `src/git/sync.rs` for the convention to match.) Identifiers are stored verbatim on disk; percent-decoding happens in the URL parser.

**In `endpoint.rs`**:

- `pub struct PrsUrl { pub submitter: PublicKey, pub identifier: String, pub subpath: String }`.
- `pub fn parse_prs_url(path: &str) -> Option<PrsUrl>` — parses `/prs/<npub>/<id>.git/<subpath>` with percent-decoded identifier and bech32-decoded pubkey. Rejects hex pubkeys (spec is npub-only). Rejects `prs` as a literal first component if `<npub>` slot doesn't start with `npub1`.

**Do not**: touch `src/http/mod.rs` yet.

**Deliverable**: compiles. Unit-testable pure functions ready for later phases.

---

## Phase 3 — HTTP routing and empty-repo fetch synthesis

**Goal**: `GET/POST /prs/<npub>/<id>.git/*` resolve through the new module. Fetches against non-existent repos return an empty-repo advertisement. Everything else 404s until later phases land.

### Routing (`src/http/mod.rs`)

Add a branch in `HttpService::call` **before** the existing `parse_git_url` branch:

```rust
if self.config.grasp06_enable {
    if let Some(prs) = crate::grasp06::endpoint::parse_prs_url(&path) {
        // dispatch to grasp06::fetch or grasp06::receive
    }
}
```

If `grasp06_enable` is false, the path falls through to normal 404 handling.

Keep routing responsibilities narrow: the match above should dispatch on `(method, prs.subpath)` exactly as the existing git branch does, but route to the new module's handlers.

### Empty-repo fetch synthesis (`src/grasp06/fetch.rs`)

Implement:

```rust
pub async fn handle_prs_info_refs(prs: PrsUrl, service: GitService, git_protocol: Option<&str>, ...) -> ...
pub async fn handle_prs_upload_pack(prs: PrsUrl, body: Bytes, git_protocol: Option<&str>, ...) -> ...
```

Behaviour:

- If the real repo at `prs_repo_path(...)` exists on disk → delegate to the existing handlers in `src/git/handlers.rs` (`handle_info_refs`, `handle_upload_pack`). Consider refactoring those to accept a `PathBuf` directly if they don't already; the current signatures should be fine.
- Otherwise → synthesize an empty-repo response.

To synthesize:

1. `git init --bare` into a temp dir (tempfile crate, already a dep).
2. Run `git-upload-pack` or `git-http-backend`-equivalent against the temp dir.
3. Return the bytes. Clean up temp dir.

An acceptable v1 alternative if a temp-dir spawn per probe is too expensive: keep a process-wide single empty bare repo (lazily created at first `/prs/` fetch, under `<git_data_path>/prs/.empty-template.git`) and serve upload-pack from it for any missing `/prs/<npub>/<id>.git`. Because git-upload-pack is read-only, a shared template is safe for concurrent reads. **Do not reuse this template for any push.**

Either implementation is acceptable. Pick the shared-template approach if the temp-dir spawn cost is visible in manual testing; otherwise the temp-dir approach is simpler.

### Receive-pack stub

For Phase 3 only, wire `POST /prs/.../git-receive-pack` to return an `ERR` pkt-line saying "not yet implemented". Phase 4 replaces this.

**Deliverable**: `git clone https://host/prs/<any-valid-npub>/<any-id>.git` succeeds and produces an empty repo when GRASP-06 is enabled. Returns 404 when disabled. Pushes are explicitly rejected with a clear message.

---

## Phase 4 — `/prs/` receive-pack handler

**Goal**: accept `refs/nostr/<event-id>` pushes. Create the real bare repo on demand. Apply the spec validation against DB+purgatory.

### Implement `src/grasp06/receive.rs`

Structure mirrors `src/git/handlers.rs:handle_receive_pack` but with important differences:

1. **No `authorize_push` call.** Skip the maintainer-set check entirely — that's the whole point of this endpoint.
2. **Pre-scan the push body** using `parse_pushed_refs` (already exists in `src/git/authorization.rs` — lift it into `crate::git::pub use` if it isn't already public). Reject the entire push with an ERR pkt-line if any ref is not `refs/nostr/<64-hex-event-id>`.
3. **Create the real repo** at `prs_repo_path(...)` with `git init --bare --initial-branch=main` **before** running `git-receive-pack`. Use `std::fs::create_dir_all` for the parent — it's idempotent. Use a per-(submitter_hex, identifier) mutex around `git init` to avoid racing two simultaneous first pushes. A `DashMap<PathBuf, tokio::sync::Mutex<()>>` on the HTTP service struct is sufficient. Release the mutex once init is complete (not for the whole push — git's own locking handles intra-push concurrency).
4. **Run `git-receive-pack`** using the existing `GitSubprocess` helpers.
5. **Post-push processing**: parse `pushed_refs` (new_oids for each `refs/nostr/<event-id>`), then for each:
   - Validate against the DB first: if a matching PR/PR-Update event exists in the DB, check signer/d-tag/commit. On commit mismatch, `delete_ref(&repo_path, &format!("refs/nostr/{}", event_id))`.
   - Validate against purgatory: if an entry exists for this event-id, check the same invariants. On commit mismatch, delete the ref and remove the purgatory entry.
   - If no event is known, call `purgatory.add_pr_placeholder(event_id, commit)` — but see Phase 5 for the scoping change this requires.
6. **Repo cleanup**: after processing, count refs in the repo with `list_refs`. If zero, `std::fs::remove_dir_all` the repo directory. This discards probe-pushes that wrote nothing valid.
7. **Trigger release**: call `process_newly_available_git_data` on the new repo path, same as the standard endpoint does. This will promote any matching purgatory events and notify subscribers. Pass the `/prs/` repo path through; Phase 7 will branch on whether the source is a `/prs/` repo to trigger mirroring.

### Wire into Phase 3 routing

Replace the Phase 3 stub for `POST .../git-receive-pack` with a call into `receive::handle_prs_receive_pack`.

**Deliverable**: a contributor can push `refs/nostr/<event-id>` to `/prs/<npub>/<id>.git`. Ref is written. If the event is already in the DB, validation runs and mismatches are rejected. If not, a placeholder is created.

---

## Phase 5 — Purgatory placeholder scoping

**Goal**: when a PR event arrives for a placeholder created via `/prs/`, validate signer and d-tag against the placeholder's scope.

### Extend `PrPurgatoryEntry`

In `src/purgatory/types.rs`, add an optional field:

```rust
/// If set, this placeholder was created by a /prs/<submitter>/<identifier>.git push
/// under GRASP-06. When the corresponding event arrives, its signer must equal
/// `submitter` and it must carry an `a` tag with d-tag equal to `identifier`.
#[serde(default)]
pub prs_scope: Option<PrsPlaceholderScope>,
```

Define `PrsPlaceholderScope { pub submitter: PublicKey, pub identifier: String }`. `#[serde(default)]` keeps persistence backward-compatible with on-disk state files written before this field existed.

Also update the `SerializablePrPurgatoryEntry` in `src/purgatory/mod.rs` to carry the new field with `#[serde(default)]`.

### Extend the purgatory API

Add:

```rust
impl Purgatory {
    pub fn add_prs_pr_placeholder(
        &self,
        event_id: String,
        commit: String,
        submitter: PublicKey,
        identifier: String,
    ) { ... }
}
```

Internally this stores a `PrPurgatoryEntry { event: None, commit, prs_scope: Some(...), .. }` keyed by `event_id`.

Keep the existing `add_pr_placeholder` (no-scope) for the standard endpoint's flow.

### Enforce scope on event arrival

In `src/nostr/policy/pr_event.rs::git_data_check`, after finding a matching placeholder via `find_pr_placeholder`, also read the full entry via `find_pr` and check `prs_scope`:

- If `prs_scope` is `None` (standard flow) → existing behaviour (commit match is sufficient).
- If `prs_scope` is `Some(scope)`:
  - Verify `event.pubkey == scope.submitter`. On mismatch, delete the `/prs/<submitter>/<identifier>.git/refs/nostr/<event_id>` ref and discard the placeholder. The arriving event does NOT satisfy this placeholder; it may still go through the normal GRASP-01/06 flow.
  - Verify some `a` tag in the event has d-tag `== scope.identifier`. On mismatch, same treatment as above.
  - If both pass, proceed as normal.

### Wire from Phase 4

The Phase 4 receive handler now calls `add_prs_pr_placeholder` instead of `add_pr_placeholder`.

**Deliverable**: placeholders created by `/prs/` pushes correctly validate the later-arriving event against the URL's submitter and identifier.

---

## Phase 6 — Event-acceptance relaxation in PR policy

**Goal**: PR and PR Update events that would fail GRASP-01 acceptance (no matching accepted announcement) are instead accepted to purgatory when they meet the GRASP-06 invariant.

### Locate the rejection

Currently in `src/nostr/policy/pr_event.rs` (and/or `src/nostr/policy/related.rs` and the write policy chain) a PR event without a matching accepted announcement is ultimately rejected. Trace the actual rejection point — the PR policy's `git_data_check` returns early when `find_relevant_repo_paths` returns empty because there are no announcements; the caller in the write-policy chain then rejects.

### Add the relaxation branch

When GRASP-06 is enabled and the existing path would reject a kind 1618 or 1619 event because no matching accepted announcement exists, check the GRASP-06 invariant:

```
let qualifies_for_grasp06 =
    config.grasp06_enable
    && matches!(event.kind, Kind::Custom(1618) | Kind::Custom(1619)) // exact kind ids TBD — verify against nostr-sdk
    && event.tags has at least one `a` tag of form "30617:<hex>:<d>"
    && event.tags has at least one `clone` tag whose value resolves to this relay's /prs/<signer-npub>/<d>.git URL
       where <signer-npub> == bech32(event.pubkey) and <d> matches one of the a-tag d-tags
```

If all conditions hold:

- Add the event to purgatory via `add_pr(event, event_id, commit, from_sync)` (existing API), marking it as belonging to the GRASP-06 flow. To distinguish the GRASP-06 flow on the event-first side, extend `PrPurgatoryEntry` with a `from_grasp06: bool` (`#[serde(default)]`) or mirror the placeholder scoping by allowing an optional `prs_scope: Option<PrsPlaceholderScope>` on event-first entries too (preferred — same field, same type). The scope is derivable from the event at insertion time.
- Return `WritePolicyResult::Ok(None, Some(msg))` with the standard "purgatory: won't be served until git data arrives" message.

If any condition fails, fall through to the existing rejection.

### Clone-tag matching

Resolving "this relay's `/prs/<signer>/<d>.git` URL" means the clone tag's host must equal `config.domain` (or one of its aliases if the project tracks aliases — check existing code that handles the standard endpoint's clone tag validation) and the path must equal `/prs/<signer-npub>/<percent-encoded-d>.git`. Percent-encoding comparison should be normalised both sides.

There is already announcement-side `clone` tag validation code in the repository that matches host + path — reuse that comparator where possible.

### Do not relax for anything other than 1618/1619

Kind checks must be exact. This relaxation must not extend to patches (1617), issues, announcements, state events, or deletions.

**Deliverable**: a PR event naming our `/prs/` endpoint in its `clone` tag is accepted to purgatory even without a matching accepted announcement. A PR event not naming our endpoint is still rejected if it would have been rejected before.

---

## Phase 7 — Cross-service mirror on purgatory release

**Goal**: when a PR/PR-Update is released from purgatory via a `/prs/` push, copy `refs/nostr/<event-id>` into any matching accepted-announcement repos on this relay.

### Trigger point

The natural hook is inside `process_newly_available_git_data` (`src/git/sync.rs`) — it already runs after receive-pack on any endpoint, identifies released events, and syncs refs across owner repos. Read its existing implementation carefully before changing it.

Detect the `/prs/` source by checking whether the passed-in repo path is under `prs_base_path(git_data_path)` using `is_prs_repo_path`. If so:

- For each released PR event, iterate `event.tags` looking for `a` tags of the form `30617:<hex-pubkey>:<d-tag>`.
- For each `a` tag, compute `<git_data_path>/<maintainer-npub>/<d-tag>.git`. If the repo exists AND an announcement for that coord is in the DB (not purgatory), mirror `refs/nostr/<event-id>` and its reachable objects into that repo using the same mechanism already used for cross-owner sync. The simplest path is to reuse the existing `sync_oids` / `sync_refs` helpers invoked today when the same `refs/nostr/<id>` is authoritative for multiple repos.

### One-directional

Do not do the reverse: pushes to `/<maintainer>/<id>.git` must not be mirrored into `/prs/*`. This is already the default since `/prs/` won't be in the set of "authorized owner repos" that cross-owner sync considers; just make sure the mirror logic in `process_newly_available_git_data` is only entered when the source is under `/prs/`.

### Absent announcements

If no accepted announcement exists for any of the event's `a` tags, no mirror happens. That's fine — the ref remains at `/prs/` and clients can still fetch from there via the event's `clone` tag.

### Back-fill on later announcement (deferred)

If an announcement is accepted *after* a `/prs/` push has already occurred, the current design does not back-fill. Leave this as a follow-up; note it as a TODO in `mirror.rs` with a pointer to announcement-promotion code. Not required for v1.

**Deliverable**: when the mirror conditions are met, a PR released via `/prs/` becomes fetchable at `<maintainer>/<id>.git` automatically.

---

## Phase 8 — Exclusion from cleanup / sync / listings

**Goal**: existing subsystems do not treat `/prs/*` as standard repos.

### `src/cleanup_empty_repos.rs`

Skip any path under `prs_base_path`. Use `is_prs_repo_path` for the check. The `/prs/` endpoint has its own cleanup rules — implemented inline in the receive handler (zero-refs repos) and in a new periodic sweep (see below).

### `src/sync/*` (GRASP-02 proactive sync)

Repo discovery iterates `<git_data_path>/<owner>/<id>.git`. Skip the `prs` directory explicitly when walking. Also ensure `SelfSubscriber` / announcement promotion logic never constructs sync subscriptions for `/prs/` repos — they have no announcement.

### Repo listings / webpages

`src/http/mod.rs::parse_repo_url` (used for landing pages / 404 pages) should not match `/prs/...` paths. The new routing branch in Phase 3 intercepts `/prs/` earlier, so this should already be the case — but verify by inspection and add an early-return in `parse_repo_url` if `path` starts with `/prs/` to be defensive.

### Periodic `/prs/` cleanup

Add a periodic task (e.g. every 10 minutes) that walks `<git_data_path>/prs/` and:

- Removes any `.git` repo with zero refs AND no active placeholder in purgatory for that repo path.
- Logs the count of directories removed.

A conservative implementation: only remove if the dir's mtime is older than the purgatory TTL (30 minutes) to avoid racing with in-flight pushes. Wire this into the existing background-task scheduler (there's already a cleanup task registry; match its style).

**Deliverable**: enabling GRASP-06 does not pollute sync state, doesn't make `cleanup_empty_repos` thrash, doesn't show `/prs/` repos in landing pages, and orphan `/prs/` dirs get cleaned up.

---

## Phase 9 — NIP-11 advertisement

**Goal**: relay advertises GRASP-06 when enabled.

In `src/http/nip11.rs`, update `RelayInformationDocument::from_config`:

```rust
if config.grasp06_enable {
    supported_grasps.push("GRASP-06".to_string());
}
```

Place insertion order consistent with the existing list (see how GRASP-05 and GRASP-02 are ordered).

Also add an optional `contributor_pr_policy` field to `RelayInformationDocument`, serialised only when present. In v1, set it to `None` (no knobs yet). Future phases with allowlist/quotas fill this.

**Deliverable**: clients discover `/prs/` support by parsing NIP-11.

---

## Phase 10 — Documentation hygiene and release notes

**Goal**: keep `AGENTS.md`'s configuration-sync contract satisfied, update the architecture doc, add a how-to.

1. **Verify the 4-place config sync** (already done in Phase 1, re-verify here): `src/config.rs`, `.env.example`, `docs/reference/configuration.md`, `nix/module.nix`. Env var names match. Defaults match. Descriptions match.

2. **Update `docs/explanation/architecture.md`** to mention the `/prs/` endpoint as an optional subsystem, with a pointer to the GRASP-06 design doc.

3. **Add `docs/how-to/enable-grasp-06.md`** (short, task-oriented): steps for an operator to enable the feature, what it costs in storage, what abuse controls are planned, link to the spec.

4. **Update `CHANGELOG.md`** under a new unreleased section with a user-facing summary.

5. **Flip the design doc's status** from "PLANNED — NOT YET IMPLEMENTED" to "Implemented" once all earlier phases are complete.

**Deliverable**: an operator reading the docs can enable the feature confidently.

---

## Reference: module layout after completion

```
src/
  grasp06/
    mod.rs            # module root, public re-exports
    endpoint.rs       # PrsUrl struct, parse_prs_url
    paths.rs          # prs_repo_path, is_prs_repo_path, constants
    fetch.rs          # handle_prs_info_refs, handle_prs_upload_pack, empty-repo synthesis
    receive.rs        # handle_prs_receive_pack: init-on-push, validation, placeholder scoping
    mirror.rs         # mirror_to_matching_announcements (called from git::sync)
    cleanup.rs        # periodic sweep of zero-ref /prs/ repos
  http/
    mod.rs            # route /prs/* ahead of standard routing
    nip11.rs          # advertise GRASP-06
  nostr/policy/
    pr_event.rs       # relaxation branch; scope enforcement on event arrival
  purgatory/
    types.rs          # PrsPlaceholderScope; PrPurgatoryEntry.prs_scope field
    mod.rs            # add_prs_pr_placeholder; persistence with serde(default)
  git/
    sync.rs           # process_newly_available_git_data: mirror branch for /prs/ sources
  cleanup_empty_repos.rs  # skip <git_data_path>/prs/*
  config.rs           # grasp06_enable field
.env.example          # NGIT_GRASP06_ENABLE
docs/
  explanation/
    grasp-06-contributor-pr-submission.md
    architecture.md   # updated cross-reference
  how-to/
    enable-grasp-06.md
  reference/
    configuration.md  # NGIT_GRASP06_ENABLE entry
nix/
  module.nix          # grasp06Enable option, env wiring
plans/
  grasp-06-implementation-plan.md  # this file
```

---

## Things explicitly out of scope for this plan

- Tests — planned separately.
- NIP-98 or any HTTP-level auth on `/prs/`.
- Allowlist / quota / size cap / PoW. The config field exists for none of these; do not stub them.
- Deletion-request integration — on a separate branch.
- Object-pool dedup — later.
- Proactive sync / replication of `/prs/` repos between relays.
- Back-filling mirrors when an announcement is promoted after a `/prs/` push.
- Auto-mirroring standard-endpoint PRs into `/prs/` (the reverse direction).
- A `g`/`G` tag distinction in NIP-51 grasp-servers lists. Clients discover via NIP-11 `supported_grasps`.

---

## Verification checklist (for each phase)

Before marking a phase done:

- `cargo build` passes.
- `cargo clippy --all-targets -- -D warnings` passes.
- `cargo fmt --check` passes.
- No unrelated changes.
- If the phase added config: all four config-sync locations updated.
- If the phase touched architecture or added modules: the design doc reference is still accurate; update it if not.

---

## Open items to confirm with maintainers before implementation

1. The exact numeric kinds for PR (1618) and PR Update (1619). Cross-check against the `nostr-sdk` version in use and the in-tree `pr_event.rs` to be sure.
2. Whether the project already has a normalising `clone` tag URL comparator to match against `config.domain`. If not, implement one and use it from both the announcement side and the GRASP-06 relaxation.
3. Whether the shared empty-template approach or per-request temp-dir approach is preferred for fetch synthesis. Benchmark trivially during Phase 3 if unsure — both are small code.
4. Whether the PR purgatory entry's existing `SerializablePrPurgatoryEntry` versioning strategy accepts a new optional field via `#[serde(default)]` (it does for other optional fields — confirm by reading `src/purgatory/mod.rs`).
