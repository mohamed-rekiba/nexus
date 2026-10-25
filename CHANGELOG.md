# Changelog

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
