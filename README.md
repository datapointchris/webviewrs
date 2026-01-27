# webviewrs

Minimal webview app that opens URLs in a native window using WebKitGTK. No browser chrome, persistent storage for logins, dark mode support.

## Requirements

- Linux with GTK
- `webkit2gtk-4.1`
- `noto-fonts-emoji` (for emoji rendering)

```bash
sudo pacman -S webkit2gtk-4.1 noto-fonts-emoji
```

## Install

```bash
cargo install --git https://github.com/datapointchris/webviewrs
```

## Usage

```bash
webviewrs <URL>
webviewrs -t "Title" -n appname --dark <URL>
```

### Options

- `-t, --title` - Window title (defaults to URL)
- `-n, --name` - App name for persistent storage (defaults to title)
- `-d, --dark` - Force dark mode
- `--width` - Window width (default: 1200)
- `--height` - Window height (default: 800)

### Examples

```bash
webviewrs https://example.com
webviewrs -n rss -t RSS --dark http://10.0.20.17
webviewrs -n youtube -t "New Music" --dark "https://www.youtube.com/playlist?list=ABC123"
```

## Data Storage

Persistent data (cookies, localStorage) stored in `~/.local/share/webviewrs/<name>/`
