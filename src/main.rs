use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::thread;

use clap::{Parser, Subcommand};
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    window::WindowBuilder,
};
use wry::{WebContext, WebViewBuilder};

#[cfg(gtk)]
use gtk::prelude::*;

#[derive(Parser)]
#[command(name = "webviewrs", version)]
#[command(about = "Open a URL in a minimal native webview window")]
// A URL and a subcommand never mix, so `webviewrs <URL>` opens a window and `webviewrs paths` runs
// the subcommand. The URL is required only when no subcommand is given. clap's generated `help`
// subcommand is off, so `paths` is the one word a URL cannot be.
#[command(
    args_conflicts_with_subcommands = true,
    subcommand_negates_reqs = true,
    disable_help_subcommand = true
)]
#[command(after_help = "\
Name every app you log in to with -n. The name holds its cookies and storage.
Without one the title or URL is the name, so a changed -t opens an empty
profile. Launching a name that is already open focuses its window.

Examples:
  webviewrs -n rss -t RSS --dark https://rss.example.com
      a site you log in to, kept under the name rss
  webviewrs https://example.com
      a one-off window, named for its URL
  webviewrs paths -n rss
      where the name rss keeps its profile, socket and cache")]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,

    /// URL to open
    #[arg(required = true)]
    url: Option<String>,

    /// Window title (defaults to URL)
    #[arg(short, long)]
    title: Option<String>,

    /// Window width
    #[arg(long, default_value = "1200")]
    width: u32,

    /// Window height
    #[arg(long, default_value = "800")]
    height: u32,

    /// App name for persistent storage (defaults to sanitized title/URL)
    #[arg(short, long)]
    name: Option<String>,

    /// Force dark mode
    #[arg(short, long)]
    dark: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Print where an app keeps its profile, socket and cache
    #[command(after_help = "\
Each line is a label, then a path: profile, socket and cache. The app is named
as a launch names it: -n, else -t, else the URL. Nothing is created.

Examples:
  webviewrs paths -n rss
      where the app named rss keeps its cookies and storage
  webviewrs paths https://example.com
      the paths of a window launched with only that URL")]
    Paths(PathsArgs),
}

#[derive(clap::Args)]
#[command(group = clap::ArgGroup::new("app").required(true).multiple(true))]
struct PathsArgs {
    /// URL the app is launched with
    #[arg(group = "app")]
    url: Option<String>,

    /// Window title the app is launched with
    #[arg(short, long, group = "app")]
    title: Option<String>,

    /// App name the app is launched with
    #[arg(short, long, group = "app")]
    name: Option<String>,
}

/// The app name a profile is kept under: `-n`, else `-t`, else the URL. The second value names
/// which one it came from, for the error a name with nothing usable in it gets.
fn resolve_app_name(name: Option<&str>, title: Option<&str>, url: &str) -> (String, &'static str) {
    match (name, title) {
        (Some(name), _) => (name.to_owned(), "-n"),
        (None, Some(title)) => (title.to_owned(), "-t"),
        (None, None) => (url.to_owned(), "the URL"),
    }
}

/// Sanitize a string to be safe for use as a directory name
fn sanitize_name(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .to_lowercase()
}

/// The longest name one path component may hold on Linux, in bytes.
const NAME_MAX: usize = 255;

/// FNV-1a over 64 bits, as 16 hex digits. The algorithm fixes its output, unlike std's hasher,
/// whose output may differ between Rust releases, so every build finds the same directory.
fn fnv1a_hex(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// The profile directory's name. A sanitized name that fits in one path component is used whole.
/// A longer one, such as a long URL, keeps the start that fits beside a hyphen and the FNV-1a hash
/// of the whole sanitized name.
fn profile_dir_name(name: &str) -> String {
    let sanitized = sanitize_name(name);
    if sanitized.len() <= NAME_MAX {
        return sanitized;
    }
    let suffix = format!("-{}", fnv1a_hex(sanitized.as_bytes()));
    let mut end = NAME_MAX - suffix.len();
    while !sanitized.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{suffix}", &sanitized[..end])
}

/// The directory holding every profile, under the platform's data directory. None where there is
/// no data directory, as on Linux with neither `XDG_DATA_HOME` nor a home directory.
fn profiles_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("webviewrs"))
}

/// Print a failure on stderr, under the program's name, and exit 1.
fn exit_with_error(message: &str) -> ! {
    eprintln!("webviewrs: {message}");
    std::process::exit(1);
}

/// The directory holding every profile, and this app's profile directory inside it. A name that
/// sanitizes to nothing exits 2, and a platform with no data directory exits 1, each with the line
/// saying how to launch again.
fn locate_profile(app_name: &str, name_source: &str) -> (PathBuf, PathBuf) {
    // An empty directory name would make this profile the directory that holds every profile.
    let dir_name = profile_dir_name(app_name);
    if dir_name.is_empty() {
        eprintln!(
            "webviewrs: the app name {app_name:?}, from {name_source}, has no letter, digit or hyphen to name a profile after; pass -n with one"
        );
        std::process::exit(2);
    }
    let Some(profiles) = profiles_dir() else {
        exit_with_error("cannot find a data directory to keep profiles in; set XDG_DATA_HOME or HOME and launch again");
    };
    let data_dir = profiles.join(dir_name);
    (profiles, data_dir)
}

/// WebKit's HTTP cache, which every profile shares: `webviewrs` under the platform's cache
/// directory. WebKit creates and fills it, so this only names it.
fn webkit_cache_dir() -> Option<PathBuf> {
    dirs::cache_dir().map(|dir| dir.join("webviewrs"))
}

/// Print where an app keeps its profile, socket and cache, one labeled path a line. It resolves
/// them through the functions a launch uses, and creates nothing.
fn print_paths(args: &PathsArgs) {
    let (app_name, name_source) = resolve_app_name(args.name.as_deref(), args.title.as_deref(), args.url.as_deref().unwrap_or_default());
    let (profiles, data_dir) = locate_profile(&app_name, name_source);
    println!("profile  {}", data_dir.display());
    println!("socket   {}", get_socket_path(&profiles, &data_dir).display());
    if let Some(cache) = webkit_cache_dir() {
        println!("cache    {}", cache.display());
    }
}

/// Create a profile's directory, and the directory holding every profile, if missing.
fn create_profile_dir(data_dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(data_dir)
}

/// The socket a profile's window takes focus requests on, in `$XDG_RUNTIME_DIR/webviewrs/` or
/// beside the profiles where that is unset. A Unix socket path holds at most 107 bytes, so the file
/// is named for the FNV-1a hash of the profile directory's path rather than for the app name.
/// Hashing the whole path keeps a profile under another data directory from answering for this one.
fn get_socket_path(profiles: &Path, data_dir: &Path) -> PathBuf {
    let dir = dirs::runtime_dir().map_or_else(|| profiles.to_path_buf(), |runtime| runtime.join("webviewrs"));
    dir.join(format!("{}.sock", fnv1a_hex(data_dir.as_os_str().as_bytes())))
}

/// Custom event for the event loop
#[derive(Debug, Clone)]
enum UserEvent {
    FocusWindow,
}

/// Try to connect to an existing instance and request focus
fn try_focus_existing(socket_path: &Path) -> bool {
    if let Ok(mut stream) = UnixStream::connect(socket_path) {
        let _ = stream.write_all(b"focus");
        true
    } else {
        false
    }
}

/// Bind the profile's socket. Only a launch whose connect failed reaches this, so a socket file
/// already there is stale and is removed first.
fn bind_instance_socket(socket_path: &Path) -> std::io::Result<UnixListener> {
    if let Some(dir) = socket_path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    remove_instance_socket(socket_path);
    UnixListener::bind(socket_path)
}

/// Delete a profile's socket file. A missing file is the goal, so a failed removal is ignored.
fn remove_instance_socket(socket_path: &Path) {
    let _ = std::fs::remove_file(socket_path);
}

/// Hand a URL to the system's default browser. A failure leaves the link doing nothing, which is
/// what the webview would do without a handler.
fn open_in_system_browser(url: &str) {
    let _ = open::that(url);
}

/// Start listening for focus requests from other instances
fn start_instance_listener(listener: UnixListener, proxy: EventLoopProxy<UserEvent>) {
    thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut buf = [0u8; 5];
            if stream.read(&mut buf).is_ok() && &buf == b"focus" {
                let _ = proxy.send_event(UserEvent::FocusWindow);
            }
        }
    });
}

fn main() {
    let args = Args::parse();

    if let Some(Command::Paths(paths)) = &args.command {
        print_paths(paths);
        return;
    }
    let url = args.url.expect("clap requires the URL when no subcommand is given");

    let (app_name, name_source) = resolve_app_name(args.name.as_deref(), args.title.as_deref(), &url);
    let title = args.title.unwrap_or_else(|| url.clone());
    let (profiles, data_dir) = locate_profile(&app_name, name_source);

    if let Err(err) = create_profile_dir(&data_dir) {
        exit_with_error(&format!(
            "cannot create the profile directory {}: {err}; make its parent writable, or set XDG_DATA_HOME to a writable directory",
            data_dir.display()
        ));
    }

    let socket_path = get_socket_path(&profiles, &data_dir);

    // Check if another instance is running. The line is a status message, so it goes to stderr.
    if try_focus_existing(&socket_path) {
        eprintln!("Focused existing instance");
        return;
    }

    // Bound before the window is built, so a launch racing this one queues on the socket rather
    // than opening a second window.
    let listener = match bind_instance_socket(&socket_path) {
        Ok(listener) => Some(listener),
        Err(err) => {
            eprintln!("webviewrs: cannot listen for focus requests on {}: {err}", socket_path.display());
            eprintln!("This window opens, but the next launch of this name opens a second window.");
            None
        }
    };
    // A launch whose bind failed leaves the socket alone on close, because another instance may be
    // listening on it.
    let owned_socket = listener.as_ref().map(|_| socket_path.clone());

    // tao initializes GTK only when the event loop is built, and gtk-rs panics on a GTK call
    // before that: WebContext::new and the --dark settings lookup both are.
    #[cfg(gtk)]
    {
        if let Err(err) = gtk::init() {
            exit_with_error(&format!(
                "cannot start GTK: {err}; it needs a display, so launch from a graphical session where DISPLAY or WAYLAND_DISPLAY is set"
            ));
        }

        // Set dark mode preference if requested
        if args.dark {
            if let Some(settings) = gtk::Settings::default() {
                settings.set_gtk_application_prefer_dark_theme(true);
            }
        }
    }

    // Create web context with persistent data directory
    let mut web_context = WebContext::new(Some(data_dir));

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    // Start listening for other instances
    if let Some(listener) = listener {
        start_instance_listener(listener, proxy);
    }

    let window = WindowBuilder::new()
        .with_title(&title)
        .with_inner_size(tao::dpi::LogicalSize::new(args.width, args.height))
        .build(&event_loop)
        .unwrap_or_else(|err| {
            exit_with_error(&format!(
                "cannot create the window: {err}; launch from a graphical session where DISPLAY or WAYLAND_DISPLAY is set"
            ))
        });

    let mut builder = WebViewBuilder::with_web_context(&mut web_context)
        .with_url(&url)
        .with_new_window_req_handler(|url| {
            open_in_system_browser(&url);
            false
        });

    // Force dark mode if requested
    if args.dark {
        builder = builder.with_initialization_script(
            r#"
            // Set color-scheme meta tag
            let meta = document.querySelector('meta[name="color-scheme"]');
            if (meta) {
                meta.content = 'dark';
            } else {
                meta = document.createElement('meta');
                meta.name = 'color-scheme';
                meta.content = 'dark';
                document.head.appendChild(meta);
            }
            // Force dark mode on html element
            document.documentElement.style.colorScheme = 'dark';
            "#,
        );
    }

    #[cfg(not(gtk))]
    let webview = builder.build(&window);

    #[cfg(gtk)]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let vbox = window.default_vbox().expect("tao builds each window with a default GTK box");
        builder.build_gtk(vbox)
    };

    let _webview = webview.unwrap_or_else(|err| {
        exit_with_error(&format!(
            "cannot build the webview: {err}; it needs WebKitGTK 4.1, installed with `sudo pacman -S webkit2gtk-4.1` on Arch or `sudo apt-get install libwebkit2gtk-4.1-0` on Debian and Ubuntu"
        ))
    });

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::UserEvent(UserEvent::FocusWindow) => {
                window.set_focus();
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                if let Some(path) = &owned_socket {
                    remove_instance_socket(path);
                }
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_opens_a_window_and_paths_runs_the_subcommand() {
        let open = Args::try_parse_from(["webviewrs", "https://example.com"]).unwrap();
        assert!(open.command.is_none());
        assert_eq!(open.url.as_deref(), Some("https://example.com"));

        let paths = Args::try_parse_from(["webviewrs", "paths", "-n", "rss"]).unwrap();
        let Some(Command::Paths(paths)) = paths.command else {
            panic!("paths parsed as a URL")
        };
        assert_eq!(paths.name.as_deref(), Some("rss"));

        assert!(Args::try_parse_from(["webviewrs"]).is_err());
        assert!(Args::try_parse_from(["webviewrs", "paths"]).is_err());
    }

    #[test]
    fn resolve_app_name_takes_the_name_then_the_title_then_the_url() {
        assert_eq!(
            resolve_app_name(Some("rss"), Some("RSS"), "https://a.example"),
            ("rss".to_owned(), "-n")
        );
        assert_eq!(resolve_app_name(None, Some("RSS"), "https://a.example"), ("RSS".to_owned(), "-t"));
        assert_eq!(
            resolve_app_name(None, None, "https://a.example"),
            ("https://a.example".to_owned(), "the URL")
        );
    }

    #[test]
    fn sanitize_name_keeps_word_characters_and_folds_the_rest() {
        assert_eq!(sanitize_name("RSS"), "rss");
        assert_eq!(sanitize_name("New Music"), "new_music");
        assert_eq!(sanitize_name(" my-app_1 "), "my-app_1");
        assert_eq!(sanitize_name("https://example.com"), "https___example_com");
        assert_eq!(sanitize_name("///"), "");
    }

    // A changed hash moves every long-named profile and every socket, so the published FNV-1a
    // 64-bit vectors pin it.
    #[test]
    fn fnv1a_hex_matches_the_published_vectors() {
        assert_eq!(fnv1a_hex(b""), "cbf29ce484222325");
        assert_eq!(fnv1a_hex(b"a"), "af63dc4c8601ec8c");
        assert_eq!(fnv1a_hex(b"foobar"), "85944171f73967e8");
    }

    #[test]
    fn profile_dir_name_keeps_a_name_that_fits_whole() {
        let name = "a".repeat(NAME_MAX);
        assert_eq!(profile_dir_name(&name), name);
    }

    #[test]
    fn profile_dir_name_cuts_a_long_name_and_appends_its_hash() {
        let name = "a".repeat(300);
        let dir = profile_dir_name(&name);
        assert_eq!(dir.len(), NAME_MAX);
        assert_eq!(dir, format!("{}-{}", "a".repeat(238), fnv1a_hex(name.as_bytes())));

        let other = format!("{}b", "a".repeat(299));
        assert_ne!(profile_dir_name(&other), dir);
    }

    #[test]
    fn profile_dir_name_cuts_on_a_character_boundary() {
        // Three bytes each, so byte 238 falls inside a character and the cut steps back to 237.
        let name = "日".repeat(100);
        let dir = profile_dir_name(&name);
        assert_eq!(dir, format!("{}-{}", "日".repeat(79), fnv1a_hex(name.as_bytes())));
    }
}
