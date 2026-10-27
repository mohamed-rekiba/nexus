# Changelog

## [0.7.0](https://github.com/mohamed-rekiba/nexus/compare/v0.6.2...v0.7.0) (2026-10-27)


### ⚠ BREAKING CHANGES

* **ui:** one source for the theme contract, UI API 0.2.0

### Bug Fixes

* **core:** one catalog read at a time, so an older read can't put back older copies ([cb89134](https://github.com/mohamed-rekiba/nexus/commit/cb89134172fe9e7f1857455cd1af0a24f57a3a05))
* log the errors the core and the app dropped, or say why they drop ([c2de39c](https://github.com/mohamed-rekiba/nexus/commit/c2de39c2827200519a21de2c073664d5f33c0d52))


### Code Refactoring

* **ui:** one source for the theme contract, UI API 0.2.0 ([b7b2c0f](https://github.com/mohamed-rekiba/nexus/commit/b7b2c0f2338667264eaf10da63278c57012647a5))

## [0.6.2](https://github.com/mohamed-rekiba/nexus/compare/v0.6.1...v0.6.2) (2026-10-26)


### Bug Fixes

* **app:** build on Windows: the login module names the process types in full ([f785932](https://github.com/mohamed-rekiba/nexus/commit/f78593238b8a17aab130f3d342ec2de8f1967285))

## [0.6.1](https://github.com/mohamed-rekiba/nexus/compare/v0.6.0...v0.6.1) (2026-10-26)


### Bug Fixes

* **app:** build on Windows: opening a link needs the process imports there too ([2d880be](https://github.com/mohamed-rekiba/nexus/commit/2d880be1cb11586a6d66a0c08d0c0c6fdb5bae73))

## [0.6.0](https://github.com/mohamed-rekiba/nexus/compare/v0.5.0...v0.6.0) (2026-10-26)


### Features

* Steam games in the Library, PC games copied in by hand, and GameMode ([84a90ce](https://github.com/mohamed-rekiba/nexus/commit/84a90cec587027df4da1ff7921cf88cd9ded880c))
* Steam's libraries are found by themselves, and each plugin describes its system ([bf8d1da](https://github.com/mohamed-rekiba/nexus/commit/bf8d1da3e663299f2fe4347cadbdf421d4eeaa68))
* **steam:** installs in Downloads, Steam's store details, and art Steam has ([b8f5d36](https://github.com/mohamed-rekiba/nexus/commit/b8f5d367f7cf7cc792d72b0897f0463087185f5c))
* the player's Steam library in the Catalog, and one schema for a game's actions ([ff6c6ce](https://github.com/mohamed-rekiba/nexus/commit/ff6c6cee6cc0e7766c4564620f5ce7300391e36c))


### Bug Fixes

* sharper wide covers, dedicated servers last, and no In-game filter in the Library ([19db0d3](https://github.com/mohamed-rekiba/nexus/commit/19db0d31204819eff90276b2770ba80954c6990b))
* update d-pad symbols in the UI to use new arrow characters ([c28b74f](https://github.com/mohamed-rekiba/nexus/commit/c28b74fc0aa05ce217e7c95c1c16c5e0a31476b9))

## [0.5.0](https://github.com/mohamed-rekiba/nexus/compare/v0.4.1...v0.5.0) (2026-10-26)


### Features

* **os:** the USB stick is named with its version, nexus-os-x86_64-&lt;version&gt;.iso ([bb95465](https://github.com/mohamed-rekiba/nexus/commit/bb9546590953e7789541545997c85b1652e096a8))

## [0.4.1](https://github.com/mohamed-rekiba/nexus/compare/v0.4.0...v0.4.1) (2026-10-26)


### Bug Fixes

* a game ends when its processes do, and Stop reaches the ones Proton's sandbox moved out ([ae8d613](https://github.com/mohamed-rekiba/nexus/commit/ae8d6137fdc21140392e072028b5e4d8fbde9c06))

## [0.4.0](https://github.com/mohamed-rekiba/nexus/compare/v0.3.1...v0.4.0) (2026-10-26)


### Features

* unpack script installers that carry their game in an archive, with unar for a RAR libarchive can't read ([4ceb62e](https://github.com/mohamed-rekiba/nexus/commit/4ceb62e90e2a37a66fddfc72459331db9f08ad72))


### Bug Fixes

* a PC game keeps its start script, links and empty folders when it installs ([745eff4](https://github.com/mohamed-rekiba/nexus/commit/745eff41da41dd58197dc218df2dbd92fdd5c1ff))
* after a restart, a finished download plays the game it installed instead of offering Install ([bb9a8c5](https://github.com/mohamed-rekiba/nexus/commit/bb9a8c539126604a3db700038c66e364a089ab9a))
* nexus --supports answers system-settings and os-install, so the Nexus OS stick builds ([320b758](https://github.com/mohamed-rekiba/nexus/commit/320b7586549f4a62878d5ac0cdfcc83f7f8767ef))
* the d-pad's ⮜  ⮞ and ⮝  ⮟ hints are the same size ([7381485](https://github.com/mohamed-rekiba/nexus/commit/7381485dccda56a9060911425bbc92316ad68b4c))

## [0.3.1](https://github.com/mohamed-rekiba/nexus/compare/v0.3.0...v0.3.1) (2026-10-25)


### Bug Fixes

* a release row's Download button has a column of its own, and only the row shows focus ([4766733](https://github.com/mohamed-rekiba/nexus/commit/4766733482c8197a6d0d76666b698059142c0942))

## [0.3.0](https://github.com/mohamed-rekiba/nexus/compare/v0.2.6...v0.3.0) (2026-10-25)


### ⚠ BREAKING CHANGES

* rewrite the PC plugin: Play starts the file the game's Starts setting picks

### Features

* a power menu, and the PS, Xbox or Logitech button closes the running game ([21ba27e](https://github.com/mohamed-rekiba/nexus/commit/21ba27ec179611cc2cf451bca7ea474bd3cb7ec4))
* rewrite the PC plugin: Play starts the file the game's Starts setting picks ([7ba759f](https://github.com/mohamed-rekiba/nexus/commit/7ba759f99b8f6ab7a68069426abaa77ca03767a5))
* the Nexus session: Nexus shows in its gamescope, Log out, and Restart Nexus in place of Quit ([8b83c32](https://github.com/mohamed-rekiba/nexus/commit/8b83c32c6729cacf5a36d8c242e5667a1b3a4138))


### Bug Fixes

* a game's session lasts until its whole process group ends, so a launcher script that backgrounds the game doesn't kill it ([c8f1bef](https://github.com/mohamed-rekiba/nexus/commit/c8f1beff04907a93ed793c0ef83f3d376ea1dfd8))
* cut a bare version or a date off PC titles, so installed PC games find their covers ([129972d](https://github.com/mohamed-rekiba/nexus/commit/129972d1a5d806dbc24acb3e9065d756ae3dfa96))

## [0.2.6](https://github.com/mohamed-rekiba/nexus/compare/v0.2.5...v0.2.6) (2026-10-24)


### Bug Fixes

* collect a release's files in release/, not dist/ ([37d8f8a](https://github.com/mohamed-rekiba/nexus/commit/37d8f8a46698a8e40d53aa66e89e8ac5f1413155))

## [0.2.5](https://github.com/mohamed-rekiba/nexus/compare/v0.2.4...v0.2.5) (2026-10-24)


### Bug Fixes

* read the rpm package's marker with bsdtar ([55e1bd7](https://github.com/mohamed-rekiba/nexus/commit/55e1bd7431cca3573933768daac53a617a4368be))

## [0.2.4](https://github.com/mohamed-rekiba/nexus/compare/v0.2.3...v0.2.4) (2026-10-24)


### Bug Fixes

* add nexus --version, which the AppImage check and Nexus OS need ([a42c8f8](https://github.com/mohamed-rekiba/nexus/commit/a42c8f8c2c001032c735c1b5d69fbfc9df25a3c0))

## [0.2.3](https://github.com/mohamed-rekiba/nexus/compare/v0.2.2...v0.2.3) (2026-10-24)


### Bug Fixes

* fetch mpv's licences by its tag on Linux ([69924aa](https://github.com/mohamed-rekiba/nexus/commit/69924aadf5308017c577ea0ab5c386e91c8ece51))

## [0.2.2](https://github.com/mohamed-rekiba/nexus/compare/v0.2.1...v0.2.2) (2026-10-24)


### Bug Fixes

* theme fonts on Windows, and the tests that assumed Unix paths ([95958a7](https://github.com/mohamed-rekiba/nexus/commit/95958a70161de854b6a666860c9b06f59c7a816c))

## [0.2.1](https://github.com/mohamed-rekiba/nexus/compare/v0.2.0...v0.2.1) (2026-10-24)


### Bug Fixes

* keep the plugin API's version apart from Nexus's version ([91df87a](https://github.com/mohamed-rekiba/nexus/commit/91df87a5242e3caae4ea8431e7f15fc8e1eb6857))

## [0.2.0](https://github.com/mohamed-rekiba/nexus/compare/v0.1.0...v0.2.0) (2026-10-24)


### Features

* **session:** keep the session log under a size limit ([09895e5](https://github.com/mohamed-rekiba/nexus/commit/09895e5f06469b5f106441ff4078d15a5488a678))

## [0.1.0](https://github.com/mohamed-rekiba/nexus/compare/v0.1.0...v0.1.0) (2026-10-24)


### Documentation

* how to try Nexus OS's features on a desktop ([b49024f](https://github.com/mohamed-rekiba/nexus/commit/b49024fe8fcb6232cacedb52710c6ae08f725d43))


### Miscellaneous Chores

* release Nexus 0.1.0, its first release ([0199f02](https://github.com/mohamed-rekiba/nexus/commit/0199f023093254df16e2ff868e10b052ec400cab))
