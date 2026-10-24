# Test archives

`version.rar` and `crypted.rar` come from the test data of the `unrar` crate 0.5.8
(https://github.com/muja/unrar.rs, MIT OR Apache-2.0). RAR archives can't be written from
Rust, so the archives adapter's tests read these through libarchive's RAR reader. `version.rar` holds one file, `VERSION`;
`crypted.rar` needs a password.

`game.tar.xz`, `game.tar.bz2`, `game.tar.zst` and `bzip2.7z` were made for these tests with
bsdtar and 7-Zip from two small files, `Game/eboot.bin` ("elf") and `Game/data.bin` ("data"):
they test the formats libarchive reads for the adapter.
