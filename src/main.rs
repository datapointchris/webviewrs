use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::thread;

use clap::Parser;
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
struct Args {
    /// URL to open
    url: String,

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

/// The directory holding every profile.
fn profiles_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("webviewrs")
}

/// The socket a profile's window takes focus requests on, in `$XDG_RUNTIME_DIR/webviewrs/` or
/// beside the profiles where that is unset. A Unix socket path holds at most 107 bytes, so the file
/// is named for the FNV-1a hash of the profile directory's path rather than for the app name.
/// Hashing the whole path keeps a profile under another data directory from answering for this one.
fn get_socket_path(data_dir: &Path) -> PathBuf {
    let dir = dirs::runtime_dir().map_or_else(profiles_dir, |runtime| runtime.join("webviewrs"));
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
    let _ = std::fs::remove_file(socket_path);
    UnixListener::bind(socket_path)
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

    let name_source = match (&args.name, &args.title) {
        (Some(_), _) => "-n",
        (None, Some(_)) => "-t",
        (None, None) => "the URL",
    };
    let title = args.title.unwrap_or_else(|| args.url.clone());
    let app_name = args.name.unwrap_or_else(|| title.clone());

    // An empty directory name would make this profile the directory that holds every profile.
    let dir_name = profile_dir_name(&app_name);
    if dir_name.is_empty() {
        eprintln!(
            "webviewrs: the app name {app_name:?}, from {name_source}, has no letter, digit or hyphen to name a profile after; pass -n with one"
        );
        std::process::exit(2);
    }
    let data_dir = profiles_dir().join(dir_name);

    // Ensure data directory exists
    std::fs::create_dir_all(&data_dir).expect("Failed to create data directory");

    let socket_path = get_socket_path(&data_dir);

    // Check if another instance is running
    if try_focus_existing(&socket_path) {
        println!("Focused existing instance");
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
    // Only the instance that bound the socket removes it on close.
    let owned_socket = listener.as_ref().map(|_| socket_path.clone());

    // Initialize GTK (required wherever wry builds on GTK, before creating WebContext)
    #[cfg(gtk)]
    {
        gtk::init().expect("Failed to initialize GTK");

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
        .expect("Failed to create window");

    let mut builder = WebViewBuilder::with_web_context(&mut web_context)
        .with_url(&args.url)
        .with_new_window_req_handler(|url| {
            let _ = open::that(&url);
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
    let _webview = builder.build(&window).expect("Failed to build webview");

    #[cfg(gtk)]
    let _webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let vbox = window.default_vbox().unwrap();
        builder.build_gtk(vbox).expect("Failed to build webview")
    };

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
                    let _ = std::fs::remove_file(path);
                }
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
}
