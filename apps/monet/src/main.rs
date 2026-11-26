//! Monet desktop entry point.
//!
//! A thin shell around `figcraft_ui_gpui::run()` so the GPUI front-end has a real application
//! target (`cargo run -p monet --`), not only an example. `run()` owns the window, the document
//! session and the `--control <port>` server; this binary adds the command-line surface the
//! packaging and automation docs refer to.

fn main() {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                return;
            }
            "-V" | "--version" => {
                println!("Monet {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            // `run()` parses the control flag itself; skip its value so it is not mistaken for
            // an unknown positional argument here.
            "--control" => {
                let _ = args.next();
            }
            other if other.starts_with("--control=") => {}
            _ => {}
        }
    }
    figcraft_ui_gpui::run();
}

fn print_help() {
    println!(
        "\
Monet — a native design tool front-end built with GPUI

Usage:
  monet [--control <port>] [--help] [--version]

Options:
  --control <port>  Serve the JSON control channel on 127.0.0.1:<port> (screenshots and
                    input injection for automation; see docs/control-protocol.md)
  -h, --help        Print this help
  -V, --version     Print the version

The document engine and the egui front-end live upstream at storytold/figcraft; this binary
ships the GPUI front-end."
    );
}
