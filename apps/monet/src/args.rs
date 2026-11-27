//! Command line for Monet: `monet [--control <port>] [files…] [figcraft://join?…]`.
//!
//! A Finder double-click or `open -a Monet.app file.figcraft` arrives as an Apple event, not on the
//! command line; `figcraft_ui_gpui` handles those through GPUI's `open_urls` callback. This parser
//! covers the launch form (files and `file://` URLs) and the existing `--control` flag.

/// What the app was started with.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Args {
    /// `--control <port>` or `--control=<port>`.
    pub control: Option<u16>,
    /// Files to open, as paths (a `file://` URL is decoded first).
    pub files: Vec<String>,
    /// The last `figcraft://` invite link; never treated as a file.
    pub join: Option<String>,
    /// Unknown `--flag`s, as given; startup refuses them instead of silently ignoring a typo.
    pub unknown: Vec<String>,
}

/// An invite link rather than a file (`figcraft:` links, in any case).
pub fn is_invite(arg: &str) -> bool {
    arg.get(..9).is_some_and(|s| s.eq_ignore_ascii_case("figcraft:"))
}

/// The path a local `file://` URL names (`%XX` escapes decoded); `None` for anything else.
pub fn file_url_path(arg: &str) -> Option<String> {
    let rest = arg.get(..7).filter(|s| s.eq_ignore_ascii_case("file://")).and(arg.get(7..))?;
    let path = rest.strip_prefix("localhost").unwrap_or(rest);
    if !path.starts_with('/') {
        return None;
    }
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if b == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b);
            i += 1;
        }
    }
    let path = String::from_utf8(out).ok()?;
    // `file:///C:/…` on Windows.
    let drive = matches!(path.as_bytes().get(1..3), Some([l, b':']) if l.is_ascii_alphabetic());
    Some(if cfg!(windows) && drive { path.get(1..).unwrap_or_default().to_string() } else { path })
}

/// Read the arguments after the program name.
pub fn parse(args: &[String]) -> Args {
    let mut out = Args::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--" {
            out.files.extend(it.by_ref().cloned());
            break;
        }
        let (flag, inline) = a.split_once('=').map_or((a.as_str(), None), |(f, v)| (f, Some(v.to_string())));
        if flag == "--control" {
            out.control = inline.or_else(|| it.next().cloned()).and_then(|p| p.parse().ok());
        } else if is_invite(a) {
            out.join = Some(a.clone());
        } else if let Some(path) = file_url_path(a) {
            out.files.push(path);
        } else if a.starts_with("--") {
            out.unknown.push(a.clone());
        } else {
            out.files.push(a.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Args, file_url_path, is_invite, parse};

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn files_and_control_port() {
        assert_eq!(parse(&args(&[])), Args::default());
        let a = parse(&args(&["--control", "4242", "a.figcraft", "b.svg"]));
        assert_eq!(a, Args { control: Some(4242), files: args(&["a.figcraft", "b.svg"]), ..Args::default() });
        assert_eq!(parse(&args(&["--control"])), Args::default());
        assert_eq!(parse(&args(&["--control", "nope"])).control, None);
    }

    #[test]
    fn control_port_accepts_the_equals_form() {
        let a = parse(&args(&["--control=7980", "a.figcraft"]));
        assert_eq!(a, Args { control: Some(7980), files: args(&["a.figcraft"]), ..Args::default() });
        assert_eq!(parse(&args(&["--control=nope"])).control, None);
    }

    #[test]
    fn unknown_flags_are_refused_never_ignored() {
        let a = parse(&args(&["--verbose", "--controlx=1", "a.figcraft"]));
        assert_eq!(a.unknown, args(&["--verbose", "--controlx=1"]));
        assert_eq!(a.files, args(&["a.figcraft"]));
        let a = parse(&args(&["--control", "1", "--", "--odd.figcraft", "--control=2"]));
        assert_eq!(a.files, args(&["--odd.figcraft", "--control=2"]));
        assert!(a.unknown.is_empty());
    }

    #[test]
    fn invite_links_join_instead_of_opening() {
        let link = "figcraft://join?host=192.168.1.20:47900&file=abc&token=AbCdEfGhIjKlMnOpQrStUv";
        let a = parse(&args(&[link]));
        assert_eq!(a.join.as_deref(), Some(link));
        assert!(a.files.is_empty(), "a link is never opened as a file");
        let a = parse(&args(&["design.figcraft", "FIGCRAFT://join/?host=h:1&file=f&token=t"]));
        assert_eq!(a.files, args(&["design.figcraft"]));
        assert_eq!(a.join.as_deref(), Some("FIGCRAFT://join/?host=h:1&file=f&token=t"));
        assert!(is_invite("figcraft:join") && !is_invite("figcraft.svg") && !is_invite("fig") && !is_invite("ü€figcraft:"));
    }

    #[test]
    fn file_urls_are_paths() {
        let a = parse(&args(&["file:///home/ada/My%20Designs/a+b.figcraft", "FILE://localhost/tmp/x.svg"]));
        assert_eq!(a.files, args(&["/home/ada/My Designs/a+b.figcraft", "/tmp/x.svg"]));
        assert_eq!(file_url_path("file:///%E2%82%AC.png").as_deref(), Some("/€.png"));
        let win = file_url_path("file:///C:/Designs/a.figcraft");
        assert_eq!(win.as_deref(), Some(if cfg!(windows) { "C:/Designs/a.figcraft" } else { "/C:/Designs/a.figcraft" }));
        for bad in ["file://server/share/a.svg", "file:///broken%2", "file:///bad%zz", "file:///%FF", "file:", "fi", "ü€file:///x"] {
            assert_eq!(file_url_path(bad), None, "{bad}");
        }
    }
}
