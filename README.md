# koposu

A [Kopuz](https://github.com/Kopuz-org/kopuz) plugin that plays the songs in
your osu! libraries. Point it at osu!lazer, osu!stable, or both, and every
beatmap set shows up as an album you can play, search and add to playlists.

- **osu!lazer**: reads `client.realm` directly (read-only, so lazer can stay
  open) and plays the songs out of lazer's `files/` store.
- **osu!stable**: reads the `.osu` files in `Songs/` and plays the song files
  next to them, whatever case the map wrote their names in.

Each beatmap set becomes an album named after the song, with the set's
background as the cover. A song is listed once even when several sets or
difficulties use the same audio file. Titles and artists are the romanised
ones osu! shows by default. The original-language name is used only when a
map gives nothing else. Durations come from the audio files themselves.

## Using it

Install **koposu** from **Settings → Library → Plugins**. It is listed as
**Verified**: it asks to read the folders you pick for its sources, which Kopuz
only allows for plugins its maintainers have reviewed. Then add a source and
pick **osu!**. It asks for folders instead of a server address:

| Game | Folder to pick |
| ---- | -------------- |
| osu!lazer on Linux | `~/.local/share/osu` |
| osu!lazer on Windows | `%APPDATA%\osu` |
| osu!lazer on macOS | `~/Library/Application Support/osu` |
| osu!lazer with a custom location | the folder you moved it to (the one with `client.realm`) |
| osu!stable | the install folder (with `Songs` in it), or `Songs` itself |

One source can hold several folders, so lazer and stable can share one. Sync
the source after you add maps in the game to pick them up.

Kopuz mounts those folders read-only for the plugin and nothing else. koposu
has no network access, and it never writes anything.

## Building

koposu is a WebAssembly component built against the Kopuz plugin SDK:

```sh
rustup target add wasm32-wasip2
cargo build --release --target wasm32-wasip2
```

`cargo test` runs natively. To check a real library as well:

```sh
KOPOSU_TEST_LIBRARY=~/.local/share/osu cargo test real -- --nocapture
```

### Trying a build in Kopuz

A plugin that reads folders loads only as the build the marketplace verified,
so a build of your own needs a debug build of Kopuz that trusts it:

```sh
mkdir -p "$KOPUZ_PLUGIN_DIR/koposu"
cp kopuz-plugin.toml "$KOPUZ_PLUGIN_DIR/koposu/"
cp target/wasm32-wasip2/release/koposu.wasm "$KOPUZ_PLUGIN_DIR/koposu/plugin.wasm"
KOPUZ_TRUST_PLUGINS=koposu just serve
```

Release builds of Kopuz ignore `KOPUZ_TRUST_PLUGINS`.

## Releasing

Bump `version` in both `Cargo.toml` and `kopuz-plugin.toml`, then push a `v*`
tag. The release workflow builds the component, attaches
`kopuz-plugin.toml` and `plugin.wasm` to a GitHub release, and puts the
marketplace entry for `Kopuz-org/plugins`' `index.json` in the release notes.
`scripts/package.sh` prints the same entry locally. A maintainer reviews the
build before the entry, with `"verified": true`, goes into the index.

## License

EUPL-1.2, like Kopuz.
