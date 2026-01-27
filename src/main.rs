use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::thread;

use clap::Parser;
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    window::WindowBuilder,
};
use wry::{WebContext, WebViewBuilder};

#[cfg(target_os = "linux")]
use gtk::prelude::*;

#[derive(Parser)]
#[command(name = "webviewrs")]
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

/// Get the data directory for persistent storage
fn get_data_dir(name: &str) -> PathBuf {
    let base = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("webviewrs");
    base.join(sanitize_name(name))
}

/// Get the socket path for single-instance communication
fn get_socket_path(data_dir: &PathBuf) -> PathBuf {
    data_dir.join("instance.sock")
}

/// Custom event for the event loop
#[derive(Debug, Clone)]
enum UserEvent {
    FocusWindow,
}

/// Try to connect to an existing instance and request focus
fn try_focus_existing(socket_path: &PathBuf) -> bool {
    if let Ok(mut stream) = UnixStream::connect(socket_path) {
        let _ = stream.write_all(b"focus");
        true
    } else {
        false
    }
}

/// Start listening for focus requests from other instances
fn start_instance_listener(socket_path: PathBuf, proxy: EventLoopProxy<UserEvent>) {
    // Remove stale socket file if it exists
    let _ = std::fs::remove_file(&socket_path);

    thread::spawn(move || {
        if let Ok(listener) = UnixListener::bind(&socket_path) {
            for stream in listener.incoming() {
                if let Ok(mut stream) = stream {
                    let mut buf = [0u8; 5];
                    if stream.read(&mut buf).is_ok() && &buf == b"focus" {
                        let _ = proxy.send_event(UserEvent::FocusWindow);
                    }
                }
            }
        }
    });
}

fn main() {
    let args = Args::parse();

    let title = args.title.unwrap_or_else(|| args.url.clone());
    let app_name = args.name.unwrap_or_else(|| title.clone());
    let data_dir = get_data_dir(&app_name);

    // Ensure data directory exists
    std::fs::create_dir_all(&data_dir).expect("Failed to create data directory");

    let socket_path = get_socket_path(&data_dir);

    // Check if another instance is running
    if try_focus_existing(&socket_path) {
        println!("Focused existing instance");
        return;
    }

    // Initialize GTK (required on Linux before creating WebContext)
    #[cfg(target_os = "linux")]
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
    start_instance_listener(socket_path.clone(), proxy);

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

    #[cfg(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "ios",
        target_os = "android"
    ))]
    let _webview = builder.build(&window).expect("Failed to build webview");

    #[cfg(not(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "ios",
        target_os = "android"
    )))]
    let _webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let vbox = window.default_vbox().unwrap();
        builder.build_gtk(vbox).expect("Failed to build webview")
    };

    // Clean up socket on exit
    let socket_path_cleanup = socket_path.clone();

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
                let _ = std::fs::remove_file(&socket_path_cleanup);
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
}
