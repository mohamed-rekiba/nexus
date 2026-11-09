# Themes stand alone, with a shared kit

Status: accepted

## Context

A theme could extend another theme (`extends` in `theme.toml`) and take the screens and
components it didn't ship from its parent (`screens`, `components`). Default extended Classic.
So a change to Classic could change Default, and a third-party theme depended on two themes it
didn't control. Behaviour that every theme needs (Settings and its tabs, the Backup tab, the
on-screen keyboard, the menus, Downloads, the Game Hub's sections) lived in Classic's files, so
the only way to get it was to extend Classic. The host also had a hard-coded base theme as its
fallback.

## Decision

- Every theme stands alone. A theme ships `shell.slint` and every screen. The host refuses the
  keys `extends`, `screens` and `components` in `theme.toml`, and the error says what to do
  instead (`crates/nexus-ui-api/src/theme.rs`). It doesn't accept them with a warning: a theme
  that relies on a parent would load with screens missing.
- The shared parts move into a kit. The host serves the kit's files under `kit/`, beneath each
  theme's own files (`KIT` in `crates/nexus-ui-api/src/contract.rs`, files in
  `crates/nexus-ui-api/kit/`). A theme imports `kit/<file>.slint`. A theme replaces a kit file
  by shipping a file at the same path.
- A kit file is behaviour and widgets, not a look. It takes its colours from tokens only, and it
  imports only the host's files (`RESERVED_FILES`: `types.slint`, `tokens.slint`,
  `assets.slint`, `options.slint`) and other kit files.
- Every token has a default: Classic's value. A theme sets only the tokens it changes.
- One built-in theme says `fallback = true` in its `theme.toml`. The host checks that exactly
  one does. It starts that theme when the player chose none, in safe mode, and when the chosen
  one fails (`start_with` in `crates/nexus-ui-host/src/host.rs`). Default is that theme. This
  replaces the hard-coded base theme.
- The Game Hub's compatibility-layer rating has generic names in the view model: `layer_source`,
  `layer_rating`, `layer_url`. The kit draws it, so it must not name one source. The names avoid
  `compat`, which already names an emulator's compatibility list.

## Consequences

- A theme can change only by its own files and the kit. A kit change reaches every theme that
  doesn't replace that file, so kit changes are tested in every built-in theme
  (`tests/tests/seams/every_theme.rs`).
- Tests guard the rules: `kit_files_pass_the_theme_rules_and_draw_only_from_tokens`,
  `kit_files_import_only_the_host_s_files_and_each_other` and `every_kit_file_on_disk_is_served`
  in `crates/nexus-ui-api/src/contract.rs`; `a_theme_that_extends_another_is_refused`,
  `a_theme_without_every_screen_is_refused_with_the_missing_file_named`,
  `a_theme_gets_the_kit_beneath_its_files_and_its_own_kit_file_wins`,
  `exactly_one_theme_is_the_fallback` and `the_default_theme_works_with_the_classic_theme_removed`
  in `tests/tests/seams/themes.rs`.
- Default carries its own fonts, sounds and logo, about 1.6 MB more in the binary. We accept the
  cost: a theme that shares files with another theme doesn't stand alone.
- A third-party theme written for `extends` no longer loads. Its author ships the missing screens,
  or imports them from the kit.
