# Capabilities follow the capability, not the kind

Status: accepted

## Context

A plugin's manifest has a kind (emulator, service, theme, system) and a list of capabilities.
Today the kernel ties data capabilities to kinds: it rejects a service that declares anything
but `metadata`, `releases`, `slots` and `configurator`, and the emulator, service and system
kinds each have an `init` trait of their own. The Steam emulator plugin already needs
`releases` and `metadata`, which the kind check would put in a service. The check protects
nothing: data capabilities only give the core data.

## Decision

- One `init` trait, `CodePlugin`, replaces `EmulatorPlugin`, `ServicePlugin` and
  `SystemPlugin`. Themes keep having no `init`: they implement `ThemePlugin`. The host holds
  either as a `PluginBox` (`PluginBox::code` or `PluginBox::theme`).
- Any plugin with `init` may provide the data capabilities: `catalog`, `releases`,
  `metadata`, `slots`, `configurator` and the rest.
- The manifest `kind` stays, and it still governs privileges: the system capabilities,
  `Programs::stream` and polkit are for `system` plugins only.

## Consequences

- The kernel checks privileges by kind (`validate` in `crates/nexus-core/src/kernel.rs`).
  Beyond privileges, `validate` uses the kind only to tell a theme from a plugin with code.
- A manifest test shows that a plugin that isn't `system` can't declare system capabilities:
  `a_plugin_that_isn_t_system_can_t_declare_system_capabilities` in
  `tests/tests/seams/boot.rs`.
- The plugin API changes, so `API_VERSION` goes from 0.1.0 to 0.2.0, with every `plugin.toml`
  `host_api` in the same commit (step 8 of the architecture plan).
