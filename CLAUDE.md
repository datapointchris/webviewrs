# webviewrs

A Rust CLI that opens one URL in a native window with no browser chrome. It keeps cookies and
local storage per app name. Launching a name that is already open focuses that window instead. It
is built on wry, for the webview, and tao, for the window and event loop.

## A build needs the WebKitGTK development packages

On Linux, wry and tao link against the system's WebKitGTK 4.1 and GTK 3. Without their development
headers, `cargo build` fails in a `-sys` crate's build script, such as `glib-sys`. The
`cargo-clippy` pre-commit hook builds the crate, so a commit fails the same way on a machine
missing them. On Arch, the README's `pacman` line is enough. On Debian and Ubuntu, the tested
package set is the `install system dependencies` step in `.github/workflows/validate.yml`.

That apt step is copied into every workflow job that compiles the crate.
`rg -n libwebkit2gtk .github/` lists every copy, and a change to the package set edits all of
them. The publish job needs the packages too, because `cargo publish` builds the packaged crate
before uploading it.

## Linux and the BSDs take GTK's path through `main`

wry builds its webview on GTK on Linux, DragonFly, FreeBSD, NetBSD and OpenBSD. `build.rs` emits
those five as the `gtk` cfg, which `main.rs` reads, so a BSD runs the Linux path whole. The `gtk`
dependency's target table in `Cargo.toml` names them again, because Cargo reads that table before
any build script runs.

- **`gtk::init` runs by hand, before `WebContext::new`.** tao initializes GTK when the event loop
  is built, and the web context is created before that. gtk-rs panics on a GTK call made before
  initialization, and the `--dark` settings lookup is one.
- **The webview goes into tao's default GTK box through `build_gtk`.** wry's `build`, which takes
  the window itself, supports only X11 on Linux. `build_gtk` works under Wayland as well.
- **`--dark` sets GTK's dark-theme preference on the GTK platforms only.** On every platform it
  also injects a script setting the page's `color-scheme` to dark.
- **Only Linux is built.** CI runs on Ubuntu, and the release builds `x86_64-unknown-linux-gnu`
  alone, so no workflow compiles the `build` branch for macOS, iOS and Android, or the GTK branch
  for a BSD. Windows cannot compile at all, because the single-instance socket imports
  `std::os::unix` unconditionally.

## An app name is one profile and one window

The app name is `-n`, else the title, else the URL. It names a directory under `webviewrs/` in the
platform's data directory, which holds both the webview's storage and the instance socket. So
changing `-t` on an app launched without `-n` starts an empty profile. The old logins stay in the
old directory.

The name is sanitized before it becomes a directory. Every character that is not alphanumeric,
`-` or `_` becomes `_`, outer underscores are trimmed, and the result is lowercased. Names that
sanitize alike, such as `RSS` and `rss`, share one profile and one window.

A launch first connects to `instance.sock` in that directory. A live instance takes the `focus`
message and focuses its window, and the new process exits 0 without opening its URL. When the
connect fails, any socket file there is stale, so the new instance deletes it and binds its own. A
failed bind is silent, and the next launch of that name opens a second window. A clean close
removes the socket.

## A new-window request opens the system browser

A `target="_blank"` link or a `window.open` call goes to the default browser through the `open`
crate, and the handler refuses the window. wry drops these requests silently when no handler is
set. The app never opens a second webview window.

## A change is checked by running it

The crate has no tests, so `cargo test` in CI runs nothing. Behavior is checked by launching the
binary on a display, under a throwaway name: `cargo run -- -n scratch https://example.com`. A name
that is already open, such as an installed copy's, gets focused, and the new build exits before
drawing anything. A throwaway name also keeps the run out of a real profile's cookies.

## A release is cut from every push to `main`

- **`release.yml` calls `validate.yml` first**, so a commit that fails validation is never
  released.
- **The version in `Cargo.toml` belongs to the release.** semantic-release picks it from the
  conventional commits, and `cargo set-version` from cargo-edit writes it into `Cargo.toml` and
  `Cargo.lock`. The release then pushes a `chore(release)` commit to `main`, so a local `main` is
  behind after every release.
- **There is no changelog file.** The notes go in the GitHub release body.
- **The archive name is a contract with cargo-binstall.** The build job tars the binary at the
  archive root as `webviewrs-<version>-<target>.tar.gz`, beside a `.sha256`.
  `[package.metadata.binstall]` in `Cargo.toml` describes that name and layout. Changing one
  without the other breaks `cargo binstall webviewrs`.
- **The publish job runs `cargo publish` to crates.io** after the build. `Cargo.lock` is committed
  so the release build runs with `--locked`.

## A config file with a stamp on its first line is generated

Those files are generated outside this repo, and a hand edit to one is lost at the next
regeneration. A block opened by a `# > custom:` comment is kept, and both WebKitGTK steps in
`validate.yml` are such blocks. `release.yml`, `.releaserc.json` and `Cargo.toml` are written by
hand. `rust-toolchain.toml` is generated, and rustup reads it on every cargo call, so no workflow
installs a toolchain.
