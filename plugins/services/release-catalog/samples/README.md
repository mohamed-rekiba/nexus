# Sample catalogs

Catalog files in the catalog collector's format (see `src/format.rs`), one per console
(`ps5.json`, `ps4.json`, `pc.json`), for testing the Catalog screen, the Game Hub of a release,
and downloads. Each release's torrent is a free
Blender film under Creative Commons (Big Buck Bunny, Sintel, Tears of Steel, Cosmos
Laundromat), not a game: it downloads, and its install then fails with "not a package".
The Steam app ids and art addresses are made up: the art doesn't load from Steam, and the
screenshot test serves pictures at those addresses.

They test:

- two versions of one title id (`PPSA99901`), shown as one game with two releases;
- `published_at` as the date added;
- the New badge: releases added in the last 14 days before 2026-10-18;
- the regions `EUR`, `USA` and `ASIA`, which pick the store for the art;
- a release with no title id and no size, which groups by its name;
- a PS5 PKG release (`fpkg`), which the Game Hub warns about;
- Steam data (the `steam` object) on a console release with no title id, which gives its art;
- a PC release packed with Wine (`pc.json`), with Steam data and a ProtonDB tier.

To use them as the bundled catalogs, copy them into the project's `assets/catalog/`. To use them as
fetched catalogs, serve this folder over https from a host in the manifest's `network` list
and set the catalog address to it.
