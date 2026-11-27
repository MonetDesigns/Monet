//! Monet desktop entry point.
//!
//! A thin shell around `figcraft_ui_gpui::run_with_launch()` so the GPUI front-end has a real
//! application target (`cargo run -p monet --`), not only an example. The library owns the window,
//! the document session, the `--control` server and the macOS `open_urls` callback; this binary
//! parses the command line, including files a shell or `open -a` passes.

// A released Monet opens a window, not a console; debug builds keep stderr/stdout attached so
// `cargo run -p monet -- --version` and panics stay readable.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

mod args;

use std::path::PathBuf;

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.iter().any(|a| a == "-h" || a == "--help") {
        print_help();
        return;
    }
    if argv.iter().any(|a| a == "-V" || a == "--version") {
        println!("Monet {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    let parsed = args::parse(&argv);
    if let Some(flag) = parsed.unknown.first() {
        eprintln!("Monet: unknown option {flag}\nUsage: monet [--control PORT] [FILE | file://URL | figcraft://join?...]...");
        std::process::exit(2);
    }

    let launch = figcraft_ui_gpui::LaunchOptions {
        control_port: parsed.control,
        files: parsed.files.into_iter().map(PathBuf::from).collect(),
        join: parsed.join,
    };
    figcraft_ui_gpui::run_with_launch(launch);
}

fn print_help() {
    println!(
        "\
Monet — a native design tool front-end built with GPUI

Usage:
  monet [--control <port>] [files...] [figcraft://join?...] [--help] [--version]

Options:
  --control <port>  Serve the JSON control channel on 127.0.0.1:<port> (screenshots and
                    input injection for automation; see docs/control-protocol.md)
  -h, --help        Print this help
  -V, --version     Print the version

Files may be plain paths or file:// URLs. A figcraft:// link is handed to the live-session
flow, never opened as a document. Finder and `open -a` documents arrive through GPUI's macOS
open_urls callback rather than this command line.

The document engine and the egui front-end live upstream at storytold/figcraft; this binary
ships the GPUI front-end."
    );
}
