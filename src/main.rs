use std::path::PathBuf;

use clap::Parser;
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
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

fn main() -> wry::Result<()> {
    let args = Args::parse();

    let title = args.title.unwrap_or_else(|| args.url.clone());
    let app_name = args.name.unwrap_or_else(|| title.clone());
    let data_dir = get_data_dir(&app_name);

    // Ensure data directory exists
    std::fs::create_dir_all(&data_dir).expect("Failed to create data directory");

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

    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title(&title)
        .with_inner_size(tao::dpi::LogicalSize::new(args.width, args.height))
        .build(&event_loop)
        .expect("Failed to create window");

    let mut builder = WebViewBuilder::with_web_context(&mut web_context)
        .with_url(&args.url);

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
    let _webview = builder.build(&window)?;

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
        builder.build_gtk(vbox)?
    };

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } = event
        {
            *control_flow = ControlFlow::Exit;
        }
    });
}
