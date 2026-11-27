# Release and support lifecycle

This is the published release contract for Rillcache: how versions are numbered,
what a support window promises, how features are deprecated and what an operator
may assume when upgrading. It is the L2 deliverable and it is enforced by
`scripts/cut-release.sh`, which refuses to publish a release that does not satisfy
its own gates.

## Version scheme

Rillcache follows Semantic Versioning `MAJOR.MINOR.PATCH` for the server and its
container images:

- **MAJOR** — a wire-visible or on-disk incompatible change: a command that used
  to succeed now fails, RESP framing changes, the journal or snapshot format
  changes without automatic migration, or a documented configuration key is
  removed.
- **MINOR** — backward-compatible capability: new commands, new configuration
  keys, new observability fields, new platforms. Existing data files and clients
  keep working.
- **PATCH** — backward-compatible fixes only: bug, performance, safety and
  documentation corrections with no format or contract change.

Pre-release identifiers (`0.2.0-rc.1`) are allowed for release candidates and are
never treated as stable by the support policy. The current package version is
`0.1.0`; `latest` and `main` container tags are convenience builds and carry no
stability promise, while a release tag (`vMAJOR.MINOR.PATCH`) identifies the exact
candidate that passed the release gates.

## Support window

- **Stable releases:** the latest MINOR of the current MAJOR is supported for
  **12 months** from its release date, and receives PATCH releases for security
  and data-integrity defects for that whole window.
- **Previous MINOR:** supported for **90 days** after the next MINOR ships,
  receiving security and data-integrity fixes only.
- **Older releases:** unsupported; no backports. Operators must upgrade.
- **Pre-releases** (`-rc`, `-alpha`, `-beta`): no support promise at all.
- **Release owner:** the repository maintainer (`macel94`) is the sole release
  owner and is the only identity permitted to publish a stable tag. No automated
  job may create one.

A PATCH on an older supported MINOR is published only when the defect is security
or data-integrity relevant and the fix is reproducible on that branch.

## Deprecation process

1. Announce the deprecation in `docs/production-readiness-gaps.md`-adjacent
   release notes and in the affected area document with the target removal
   version.
2. Keep the deprecated behavior working for **at least one MINOR release** and
   emit a startup or command-level warning where one exists.
3. Remove it only in a MAJOR release, and list it in that release's migration
   notes.
4. A command or configuration key can be removed without a MINOR cycle only when
   it is a security hazard; that removal ships in a PATCH with a security note.

## Upgrade expectations

- **Data compatibility:** a MINOR or PATCH upgrade replays the previous release's
  journal and snapshot format without manual conversion. A MAJOR upgrade that
  changes the on-disk format ships a tested migration and rollback note, and the
  old binary must still be able to start from the pre-migration data.
- **Rollback:** every release documents whether rolling back to the previous
  release is supported for the data written by the new one. Rolling back across
  a format change is documented as unsupported rather than discovered.
- **Configuration:** keys are additive within a MAJOR. Removing or renaming a key
  is a MAJOR change and is listed in the migration notes.
- **Wire compatibility:** the supported compatibility profile is the one recorded
  in `docs/compatibility.md` for that release. Clients pinned to that profile can
  upgrade within a MAJOR without application changes.

## Release gates

`scripts/cut-release.sh` refuses to tag unless all of the following hold for the
exact candidate commit:

1. `main` contains the candidate and its required workflows passed on that head.
2. `docs/production-readiness-gaps.md` has no open P0 item that the release
   claims to satisfy, and the items still open are listed in the release notes.
3. The version in `Cargo.toml` matches the tag being created.
4. `CHANGELOG.md` has an entry for that version describing changes, migration and
   rollback notes.
5. The release notes state the support window, the compatibility profile and any
   accepted limitation (for example: standalone only, no replication).

Passing the merge gate is not release acceptance; the release notes must name the
evidence that backs each claim, and items that only have merge-gate evidence say
so explicitly.

## Release notes contents

Every release publishes, under `docs/releases/vMAJOR.MINOR.PATCH.md`:

- the exact commit and image digests that were tested;
- the compatibility profile and the deltas against the previous release;
- migration and rollback instructions, or an explicit statement that none are
  needed;
- the support window dates computed from this policy;
- every open readiness gap that affects the release, stated as an accepted
  limitation with its issue number.

## Current state

`0.1.0` has not been tagged yet: the release action is reserved for the release
owner and is gated on the checks above. Until that tag exists, the project remains
**NOT PRODUCTION READY** and no support window is in force.

## Evidence

- `scripts/cut-release.sh` — mechanical enforcement of the gates above.
- `CHANGELOG.md` — per-release change, migration and rollback record.
- `docs/releases/` — published release notes starting with `v0.1.0`.
