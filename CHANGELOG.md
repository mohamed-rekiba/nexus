# Changelog

## [0.15.1](https://github.com/mohamed-rekiba/nexus/compare/v0.15.0...v0.15.1) (2026-11-10)


### Bug Fixes

* **kyty:** a game's patch folder beside it doesn't stop its install ([ab29abc](https://github.com/mohamed-rekiba/nexus/commit/ab29abc31252ebf6b88bbb69f5583e20e2bf4327))
* **theme-default:** the counts in the genre and sort lists line up ([6ee54e0](https://github.com/mohamed-rekiba/nexus/commit/6ee54e08407142d47589957ed68943a40f839532))

## [0.15.0](https://github.com/mohamed-rekiba/nexus/compare/v0.14.0...v0.15.0) (2026-11-10)


### ⚠ BREAKING CHANGES

* **navigation:** the theme contract is 0.3.0. Themes must ship screens/library.slint exporting LibraryScreen, and no screen is called "running".

### Features

* **emulators:** a folder whose name can't be a path hides no game ([37d209b](https://github.com/mohamed-rekiba/nexus/commit/37d209ba6025b007b1d805b459b14b81da36f68c))
* **library:** a game named from its file links to the offer whose name it starts ([ed3db8f](https://github.com/mohamed-rekiba/nexus/commit/ed3db8fa76dc656c98a52bf6c6a6e4a98d9f7e5f))
* **library:** one game file per game, with its details and art in its folder ([a872764](https://github.com/mohamed-rekiba/nexus/commit/a8727643934a808445eb65c819334ba08c1231cb))
* **navigation:** the core owns where the player is, and the theme contract is 0.3.0 ([2befc62](https://github.com/mohamed-rekiba/nexus/commit/2befc624a25842f5f8f05b4616da1fe25ce10ac8))

## [0.14.0](https://github.com/mohamed-rekiba/nexus/compare/v0.13.0...v0.14.0) (2026-11-09)


### Features

* **pc:** a Windows game on Linux runs in a sandbox that sees only its own folders ([cf011c8](https://github.com/mohamed-rekiba/nexus/commit/cf011c80bde42d60942abb15ff3ff9d4d0f63b37))


### Bug Fixes

* **steam:** Nexus follows the game, not the install scripts Steam runs before it ([04dc030](https://github.com/mohamed-rekiba/nexus/commit/04dc0309d56fbf412c596fd5fc8a64fee350c6fc))

## [0.13.0](https://github.com/mohamed-rekiba/nexus/compare/v0.12.0...v0.13.0) (2026-11-09)


### ⚠ BREAKING CHANGES

* **themes:** themes stand alone; extends, screens and components are refused

### Features

* **backup:** a backup capability, the backup subsystem and its archive, and the Backup view model ([f27ed9b](https://github.com/mohamed-rekiba/nexus/commit/f27ed9b0ee6ad273e1219d8bdde31077f3385ec6))
* **backup:** delete from a backup's menu, restore from another folder through a browser, and the tab's words follow the plugins ([ae1a047](https://github.com/mohamed-rekiba/nexus/commit/ae1a0478330d91acc9f19066230dda0e0c7e3787))
* **backup:** Lichii, shadPS4, KytyPS5 and PC name their keys, firmware, settings and saves for a backup ([e2ff3eb](https://github.com/mohamed-rekiba/nexus/commit/e2ff3eb355f874929242c7ced105edaf35bd752e))
* **backup:** one click in the backup browser opens a place or a folder, or asks to restore ([7d03860](https://github.com/mohamed-rekiba/nexus/commit/7d03860179468c9cf7982b5af4a1dd3a769fc790))
* **backup:** Settings has a Backup tab: back up now, large items, the backup folder, and a restore on a second press ([79121c4](https://github.com/mohamed-rekiba/nexus/commit/79121c451c72c27b01bfe12d75f515ca62631500))
* **backup:** the backup browser is a sheet in the kit; the Backup tab's browser rows go ([85276d0](https://github.com/mohamed-rekiba/nexus/commit/85276d00734e65503ac3d71f95f988656d214fca))
* **backup:** the browser's core: places by kind, folder details, filter, hidden folders, recent folders ([b189427](https://github.com/mohamed-rekiba/nexus/commit/b189427e41503d9c88b545b71e7d58db22fe29fb))
* **contract:** the Game Hub's compatibility-layer rating has generic fields ([b5a09d1](https://github.com/mohamed-rekiba/nexus/commit/b5a09d1ad5059c5b056ca95b270d4b8d089b32e2))
* **kyty:** builds with --data-folder share one saves folder, with the build's saves copied there once ([5c02e59](https://github.com/mohamed-rekiba/nexus/commit/5c02e59b0cad82e534e610cdf6b77acdd1649a1f))
* **themes:** Classic's behaviour and token-drawn widgets move into the kit ([ad5a379](https://github.com/mohamed-rekiba/nexus/commit/ad5a3797cdcf53a40d92446fff5ec5d821dd6fa5))
* **themes:** the Default theme stands alone ([01d0211](https://github.com/mohamed-rekiba/nexus/commit/01d02114906f78980d4d5e5680873ada314587d1))
* **themes:** the host serves a kit under kit/ beneath every theme, tokens have defaults, and fallback = true replaces BASE_THEME ([bdea7e2](https://github.com/mohamed-rekiba/nexus/commit/bdea7e28289588b3e39ba8943ea4ceb4270d562e))
* **themes:** themes stand alone; extends, screens and components are refused ([834372b](https://github.com/mohamed-rekiba/nexus/commit/834372bf65cad49f4734711e287b2dfad6b225e6))


### Bug Fixes

* **backup:** a held Enter in the backup browser doesn't answer the dialog it opened ([c376d2d](https://github.com/mohamed-rekiba/nexus/commit/c376d2d7b422a9c075ea105445f5cff67c980d24))
* **backup:** a restore checks kinds, sizes and free space first, writes each file whole, keeps one job, and leaves its settings for the next start ([debe629](https://github.com/mohamed-rekiba/nexus/commit/debe629bf12f9f28952ded6f94ca8212b310e906))
* **backup:** the browser's filter also finds a backup by the date its row shows ([9e60bae](https://github.com/mohamed-rekiba/nexus/commit/9e60bae20481a52c5e2e9eb4bf9bc68ee1078ac3))
* **backup:** the browser's places start at the folder's place, drives say TB, and an empty folder shows no Enter hint ([dfe2752](https://github.com/mohamed-rekiba/nexus/commit/dfe275284f114248caea66c963f6a6624c622937))
* **backup:** the host restores into the folders a plugin names, so Lichii's keys and firmware folders stay read only for the plugin ([06e2d9b](https://github.com/mohamed-rekiba/nexus/commit/06e2d9b1ba644dd2931133be8ea216891d13ac07))
* **sessions:** keep the game's new windows until the player brings the launcher, and tag each new one once ([bd73f97](https://github.com/mohamed-rekiba/nexus/commit/bd73f977cfe9f6fe7f54c6561b0102d71143e6f5))
* **sessions:** under gamescope, a game in front keeps the windows it opens after it showed, with logs of the focus ([dc9c337](https://github.com/mohamed-rekiba/nexus/commit/dc9c3376b589d66e82da2682886f9dcea2580e2a))

## [0.12.0](https://github.com/mohamed-rekiba/nexus/compare/v0.11.1...v0.12.0) (2026-11-08)


### Features

* **default theme:** a shorter Library header, one card size, and the system's cover behind its games ([35b1363](https://github.com/mohamed-rekiba/nexus/commit/35b1363e8c5beada00a69fd8aa29087d1472ab4c))
* **themes:** Settings is the last icon in the top bar, and a hint for two pad buttons fits them ([1e38752](https://github.com/mohamed-rekiba/nexus/commit/1e38752fa0b38d8823d2c32a244f4b22a9a5d757))
* **themes:** the Default theme is first in the Themes list ([2bfbd1e](https://github.com/mohamed-rekiba/nexus/commit/2bfbd1ee9022225f29a7417aede41546ad56442c))


### Bug Fixes

* **build:** run-session gives the X11 settings to Nexus only, and opens gamescope full screen ([39e805d](https://github.com/mohamed-rekiba/nexus/commit/39e805db6c8afbd50abea7e7304773d832fa66d2))

## [0.11.1](https://github.com/mohamed-rekiba/nexus/compare/v0.11.0...v0.11.1) (2026-11-08)


### Bug Fixes

* **package:** the Windows package names the FFmpeg commit its libmpv was built from ([e6aa138](https://github.com/mohamed-rekiba/nexus/commit/e6aa13854df8a8a435e2544fe84b05cb3e9525e7))

## [0.11.0](https://github.com/mohamed-rekiba/nexus/compare/v0.10.0...v0.11.0) (2026-11-08)


### ⚠ BREAKING CHANGES

* a player whose saved theme is `default` now sees the systems list, and a third-party theme with `extends = "default"` now extends the Default theme (which extends Classic). Use `extends = "classic"` for the old base.

### Features

* each emulator ships its system's icon and cover, and a theme can replace them ([305b4fa](https://github.com/mohamed-rekiba/nexus/commit/305b4fab75b9ed3482ba2d2afd4960e903c31d67))
* **systems theme:** the project owner's All games icon and picture, and a new Switch icon ([0f6940f](https://github.com/mohamed-rekiba/nexus/commit/0f6940f77868298bae65a12441ff798083ee0740))
* the systems list is the Default theme, and the old look is the Classic theme ([6bbf738](https://github.com/mohamed-rekiba/nexus/commit/6bbf7385702c4b943c3ae4c139db7789a2e4f998))
* the Systems theme, where Home is a list of the systems ([e0b5534](https://github.com/mohamed-rekiba/nexus/commit/e0b5534e73d5e4aca8c6509c601000834a799a02))


### Bug Fixes

* **deps:** pulldown-cmark 0.13.6 for the wikilinks advisory, and accept pin-utils through Slint ([b10ec18](https://github.com/mohamed-rekiba/nexus/commit/b10ec1877108087a5224bf7c965e82e39143b97c))
* **package:** build the Apple Silicon app on macos-15, as GitHub retired macos-14 ([bd25130](https://github.com/mohamed-rekiba/nexus/commit/bd251306e117e1637b3ea94caeeaf61db61b8f02))
* **systems theme:** the list after a game starts, the cover's wait, and the review's findings ([cd902ed](https://github.com/mohamed-rekiba/nexus/commit/cd902ed726c6fd6237164bca5f89c2d19af1d324))

## [0.10.0](https://github.com/mohamed-rekiba/nexus/compare/v0.9.0...v0.10.0) (2026-11-08)


### Features

* a game that restarts itself through its launcher is started again in the same session ([5d37692](https://github.com/mohamed-rekiba/nexus/commit/5d3769239d2e4a311b950be0afe1586d060362fe))
* a Switch catalog from the Switch forum, without its NRO homebrew topics ([749c624](https://github.com/mohamed-rekiba/nexus/commit/749c624fd3ba241852b2d0c8c5b6eb53853ef196))
* an emulator with a window of its own gets an Open button on its page ([1425b6e](https://github.com/mohamed-rekiba/nexus/commit/1425b6ea89b320db548d5130a8ad55cf54bd846c))
* CONCURRENCY=4 make catalog reads up to 4 topic pages at once ([5c957d1](https://github.com/mohamed-rekiba/nexus/commit/5c957d15a1511a2bbccfb22a9aec09f982d6de0b))
* **emulator-kit:** Switch game files, an optional compatibility list, and builds without a test run ([d1c361d](https://github.com/mohamed-rekiba/nexus/commit/d1c361d1bcf5f0ec51d5772f56552c91dd29170a))
* Lichii games start in eden-cli with players 1 to 4 on the connected controllers ([e26ef11](https://github.com/mohamed-rekiba/nexus/commit/e26ef1108c55c8b2e7a878d9f3d2a108fd6e8482))
* **lichii:** a GPU split setting, one thread by default ([a6851c1](https://github.com/mohamed-rekiba/nexus/commit/a6851c1b148aaf1e2c16637b3a1072ddd174cf8c))
* **lichii:** a solid NSZ installs as the NSP it was made from ([1ef4ece](https://github.com/mohamed-rekiba/nexus/commit/1ef4ece210f279c90ddfe41a2d716f482dde4f31))
* Switch and Switch 2 games with Lichii ([c8919dd](https://github.com/mohamed-rekiba/nexus/commit/c8919dd67cbe37b4135d2f1691f1fc886c673717))
* the bundled Switch catalog, 7011 topics from the Switch forum ([fa32d4c](https://github.com/mohamed-rekiba/nexus/commit/fa32d4cd73063ac8434f8f615b7740f22af17880))
* the host unpacks a bare AppImage download, as it does one in a zip ([bd07df8](https://github.com/mohamed-rekiba/nexus/commit/bd07df8ae1ea8ebbdcf2a22d8932ca13f115feb5))


### Bug Fixes

* a Switch file tagged DLC is not a game, though a standalone app has a game's title id ([49aadee](https://github.com/mohamed-rekiba/nexus/commit/49aadeea1d05cf2abf9d35f3eeb3cdf32530c047))
* a Switch game is its install folder with its update and DLC, and Lichii installs from the packages folder ([bc7c207](https://github.com/mohamed-rekiba/nexus/commit/bc7c2070eeb86c1a88c3d01b01baf424804947eb))
* **default theme:** one click on a Home tile opens the game or the Library ([7de96e6](https://github.com/mohamed-rekiba/nexus/commit/7de96e6f1122d8664ecaf02230457cc899b089bd))
* make catalog goes on from a run that stopped while it read topic pages ([b47bac8](https://github.com/mohamed-rekiba/nexus/commit/b47bac8538e2c348cc67e17b829fa53026baab75))

## [0.9.0](https://github.com/mohamed-rekiba/nexus/compare/v0.8.0...v0.9.0) (2026-11-05)


### Features

* a plugin's folders can be rows in its Settings, and L1 and R1 choose a sheet's tab ([f1b45c5](https://github.com/mohamed-rekiba/nexus/commit/f1b45c5eb91ae413acc699a8ff7a1ba7e1c52da2))
* KytyPS5 gets --addcont-dir &lt;game folder&gt;/addcont when a build knows it ([8736721](https://github.com/mohamed-rekiba/nexus/commit/8736721b5b4396d387f50ba1749a7f5875f0a772))


### Bug Fixes

* L1 and R1 go round every tab of a sheet, the home top bar goes round its ends, and L1 from Home opens Settings on its last tab ([0ed75bb](https://github.com/mohamed-rekiba/nexus/commit/0ed75bbead11f70a9d3a5163a69bb59cd7063313))
* L1 and R1 on Home move along the top bar, round its ends ([99a9e46](https://github.com/mohamed-rekiba/nexus/commit/99a9e4645948dc2ad50b58c29024f064e89c2526))

## [0.8.0](https://github.com/mohamed-rekiba/nexus/compare/v0.7.0...v0.8.0) (2026-10-28)


### Features

* a plugin's page has tabs for its settings and folders ([fbf5f68](https://github.com/mohamed-rekiba/nexus/commit/fbf5f68e791b1fb7ccfe9dddcf2bfeef92f96075))
* a theme option can be hidden, and the default theme hides Layout direction for now ([1531a11](https://github.com/mohamed-rekiba/nexus/commit/1531a11e38d59fe5d449c67cf4fa9c78343bdf58))
* action buttons and path lines in the Settings rows, and one frame for every sheet ([22415ad](https://github.com/mohamed-rekiba/nexus/commit/22415add9bf15384749c346984360c25af5dcbf3))
* Launcher is the first tab, each tab's row places are in settings-layout.slint, and the Settings screenshot is from the app ([23ef1d6](https://github.com/mohamed-rekiba/nexus/commit/23ef1d6d014c92f496566570f6f44cd513f23a49))
* Settings has a Power tab, one component for each tab, and This PC scrolls with the keys ([b9e107f](https://github.com/mohamed-rekiba/nexus/commit/b9e107fb57ff540627d35a6372914c80845fab43))
* Settings shows one section at a time under tabs ([0c6f96b](https://github.com/mohamed-rekiba/nexus/commit/0c6f96b1e5edd5ed7242d330c752e4185b706f4d))
* THIS PC rows are inert, and the screenshots show Settings at 80% and 140% ([d0e1a59](https://github.com/mohamed-rekiba/nexus/commit/d0e1a594e96479789f949def913fd7b88e2cd4df))


### Bug Fixes

* the tabs stay silent at their ends, and a tab that goes away leaves the last one ([e2ae500](https://github.com/mohamed-rekiba/nexus/commit/e2ae5001de3329175c5b1671c15d8e608e08e3f6))

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
