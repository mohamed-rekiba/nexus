# The reads for a game's page stay as separate flows

Status: accepted

## Context

Opening a game's page runs through five small modules:

- `navigation::open_game` (`navigation.rs`) refreshes the VPN for an offer, loads the details,
  selects the game, refreshes the disks and pushes the route.
- `navigation::select_game` selects a game, prefetches the art around it and, once the
  selection rests, loads the details.
- `library_reads::load_detail` asks the game's plugin for its details, compatibility, slots and
  settings, or, for a Catalog offer, builds them from the offer.
- `catalog_reads::offer_detail` builds an offer's details from its best release.
- `snapshot::detail_game` picks the game whose page the view shows.

A review on 2026-10-27 counted the hops and asked whether one module should own them. The
hops are the only evidence it found: no bug, and no step done twice.

## Decision

The flows stay separate. Each one crosses a different pair of areas, and no step repeats:

- `open_game` and `select_game` differ on purpose. A page opened by the player loads at once and
  waits on the disks; a selection waits `SELECT_SETTLE` first and loads only details nobody
  has read.
- `load_detail` has five callers: `navigate`, `open_game` and `select_game` in `navigation.rs`,
  and the first game after a refresh and the read of all details in `library_reads.rs`.
  Deleting it would put its steps in each of them.
- `offer_detail` has one caller, `load_detail`. Moving it next to that caller would move
  code and shorten no hop that a reader has to follow.
- `detail_game` reads `AppState` only and has one caller, the snapshot.

## Consequences

- A later review does not suggest merging these flows because of the hop count alone.
- Revisit this when a second caller needs `open_game`'s steps, when `load_detail` gains a third
  branch, or when a bug comes from two of these flows disagreeing.
