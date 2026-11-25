# Orion

A desktop app that shows every publicly catalogued object orbiting Earth on a 3D globe, at its computed position for any chosen moment.

**Status:** early. The app draws a textured globe with a live day/night terminator, which you can turn by dragging and zoom by scrolling. About 29,000 Tracked Objects are drawn at their current Positions. Element Sets are fetched from CelesTrak when the ones on hand are more than six hours old, kept in a cache on disk, and fall back to a Snapshot bundled with the app when there is no connection.

## Run it

Requires a recent stable Rust toolchain (1.95 or newer) and a GPU with Metal, Vulkan or DirectX 12.

```bash
cargo run
```

## Layout

- `crates/orion-core`: headless orbital maths (propagation, time, coordinate conversion), with no graphics dependencies.
- `crates/orion-data`: fetching Element Sets and caching them on disk, with the rules for how often a source may be asked.
- `crates/orion-app`: the desktop app, built on `wgpu`, `winit` and `egui`.
- `GLOSSARY.md`: the project's vocabulary. Code and docs use these terms.
- `docs/adr/`: decision records.

## Credits

- Earth imagery: NASA Blue Marble Next Generation and NASA Black Marble 2016, courtesy of NASA Earth Observatory. Both are in the public domain.
- Orbital data: Element Sets from [CelesTrak](https://celestrak.org), derived from the catalogue published by the U.S. Space Force at Space-Track.org.

## Licence

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT licence](LICENSE-MIT), at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this work by you, as defined in the Apache-2.0 licence, shall be dual licensed as above, without any additional terms or conditions.
