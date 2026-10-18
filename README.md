# webviewrs

Opens one URL in a native window with no browser chrome: no address bar, no tabs, no toolbar. Each
app name keeps its own cookies and local storage, so a site you log into stays logged in. Launching
a name that is already open focuses that window instead of opening a second one.

It is a window around one site, never a browser. The page's own links navigate inside the window. A
link that asks for a new window opens in the default browser. The webview comes from wry and the
window from tao, and on Linux the engine underneath is WebKitGTK.

## Install

```bash
sudo pacman -S webkit2gtk-4.1 noto-fonts-emoji   # Arch: WebKitGTK 4.1 brings GTK 3; the font draws emoji
cargo binstall webviewrs                          # the prebuilt binary attached to the GitHub release
cargo install webviewrs                           # or build the crates.io release from source
```

The binary links WebKitGTK 4.1 and GTK 3 when it loads, so the system packages come before either
cargo command. Arch packages carry their headers, so the `pacman` line also covers a source build.
On Debian and Ubuntu, a source build needs the development packages CI installs, listed in the
`install system dependencies` step of [validate.yml](.github/workflows/validate.yml).

The release attaches one prebuilt binary, for `x86_64-unknown-linux-gnu`. `cargo binstall` is a
separate tool, installed with `cargo install cargo-binstall`.

Linux is the one platform CI builds, and the window runs under X11 or Wayland. Windows cannot
compile it, because the single-instance socket is a Unix socket.

## Starting from scratch

```bash
webviewrs --help                                       # loads WebKitGTK and GTK; a missing library fails here
webviewrs -n example -t Example https://example.com &  # a window on example.com, under the name "example"
webviewrs -n example https://example.com               # that name is open: prints "Focused existing instance"
```

The third command opens nothing. A name that is already open takes the launch, so its window comes
forward, the process exits 0, and the URL on that command line is ignored. Close the window and the
same command opens it again, with the profile's cookies and storage intact.

`webviewrs --help` lists every option.

## An app name is one profile and one window

```bash
webviewrs -n rss -t RSS --dark https://rss.example.com    # a reader, dark, under the name "rss"
webviewrs -n youtube -t "New Music" --dark "https://www.youtube.com/playlist?list=ABC123"
```

The name is `-n`, else the title `-t`, else the URL. It names a directory under `webviewrs/` in
`$XDG_DATA_HOME`, which defaults to `~/.local/share`. That directory holds the webview's cookies
and local storage, and the instance socket. Changing `-t` on an app launched without `-n` starts an
empty profile, and the old logins stay in the old directory. Pass `-n` for anything you log into.

The name is sanitized before it becomes a directory. Every character that is not alphanumeric, `-`
or `_` becomes `_`, outer underscores are trimmed, and the result is lowercased. Names that sanitize
alike, such as `RSS` and `rss`, share one profile and one window.

Two stores are shared by every name. WebKit's HTTP cache is in `$XDG_CACHE_HOME/webviewrs/`, which
defaults to `~/.cache/webviewrs/`. Its HSTS list is `hsts-storage.sqlite`, beside the profile
directories.

## A name that is open takes the launch, through its socket

A launch first connects to `instance.sock` in the profile directory. A live instance takes the
`focus` message and raises its window. The new process prints `Focused existing instance` and exits
0 without opening its URL. When the connect fails, any socket file there is stale, so the new
instance deletes it and binds its own. A clean close removes the socket.

A failed bind is silent, and the next launch of that name opens a second window. A socket path past
the 107-byte Unix limit fails that way. A URL used as the name is often long enough: a YouTube
playlist URL gives a socket path of about 120 bytes. That is one more reason to pass `-n`.

## A new-window request opens the system browser

A `target="_blank"` link or a `window.open` call goes to the default browser through the `open`
crate, and the webview refuses the window. Without that handler, wry drops such a request silently
and the link does nothing. The app never opens a second webview window.

## `--dark` asks for dark, and the page decides how

On Linux, `--dark` sets GTK's dark-theme preference before the window opens. It also injects a
script that marks the page's `color-scheme` as dark, through a `<meta>` tag and the root element's
style. A page keeps any colors it sets itself.

## Working on it

```bash
cargo build                                   # needs the system packages under Install
cargo run -- -n scratch https://example.com   # a throwaway name, kept apart from real profiles
```

The crate has no tests, so a change is checked by launching it on a display. A name that is already
open, such as an installed copy's, gets focused, and the new build exits before drawing anything. A
throwaway name also keeps the run out of a real profile's cookies.

CI cuts a release from every push to `main` that carries a releasable conventional commit. The
release writes the version into `Cargo.toml`, so a change never edits it. The notes are each GitHub
release's body, and there is no changelog file.
