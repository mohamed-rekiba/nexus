# Glossary

Terms used in the code and docs, in alphabetical order.

**Archive error** (`ArchiveError`): why an archive didn't unpack, typed: `Password`, `Escapes`,
`NoRoom`, `Unsupported`, `Missing`, `Corrupt`, `Io` or `Cancelled`. The core shows its text;
an adapter picks a fallback reader by its kind, never by its text.

**Archive plan** (`Plan`): what an unpack may write, decided from an archive's listing before
anything is written. `plan_unpack` in nexus-core makes it, and every `Archives` adapter writes
only what it holds.

**Archives contract** (`contract::archives`): the refusals every `Archives` adapter must make,
checked by the test kit against the app's `HostArchives` and the fake `FakeArchives`.

**Format reader**: the code that lists and streams one archive format's entries: `zip`, `tar`,
`sevenz-rust2`, libarchive, or `unar`. It decides nothing about safety; the archive plan does.

**Listing** (`ArchiveEntry`): an archive's entries as a format reader gives them, unchecked: each
name as the archive spells it, its size, its kind, and whether it needs a password.
