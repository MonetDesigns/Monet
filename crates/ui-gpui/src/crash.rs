//! 崩溃兜底：主线程 panic 时写一份错误日志、尽量把当前工程存盘，并另起一个进程弹出
//! 「错误报告」窗口（错误信息可复制、显示日志路径、底部有「重启 / 关闭」）。
//!
//! ## 为什么另起进程
//!
//! panic 发生时 GPUI 的事件循环、渲染线程都已经不可信，在原进程里再开一个 GPUI 窗口
//! 很可能同样失败。这里用 `std::env::current_exe()` 以 `--crash-report <日志>` 重新
//! 启动自己：新进程的第一件事就是走 [`run_reporter_if_requested`]，只画错误报告窗口，
//! 不加载工程、不启动控制通道。原进程随后退出，新进程独立存活。
//!
//! ## 为什么钩子里不直接退出进程
//!
//! 画布渲染线程里刻意用 `catch_unwind` 兜住单帧渲染失败（`render_worker::guarded`）：
//! 那种 panic 不该让整个应用退出。所以钩子只在**主线程**（事件循环所在线程）的 panic
//! 上动作；主线程的 panic 会由 winit 兜住后在事件循环里 `resume_unwind`，最终被
//! [`run`](crate::run) 顶层的 `catch_unwind` 接住，再走 [`fatal`]。渲染线程的 panic 只
//! 是普通线程结束，这里的钩子不介入。
//!
//! ## 工程存盘
//!
//! panic 时拿不到 GPUI 的实体（也不该在此时碰它），所以画布每帧把引擎文档做一份轻量
//! 快照（[`capture`]，只在修订号/路径变化时重建）。紧急保存只读这份快照：
//! 有路径且未保存的文档原子写回原文件；没有路径的新文档写进 `CrashSaves/<时间>/`
//! 目录，并在日志里列出实际保存位置。

use std::any::Any;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::ThreadId;

use figcraft_doc::Document;
use figcraft_engine::Session;
use figcraft_engine::branches;
use figcraft_format::{Branch, Version};
use gpui::prelude::*;
use gpui::{
    div, px, App, Application, Bounds, ClickEvent, ClipboardItem, Context, Div, FontWeight, IntoElement, Render, SharedString, Stateful,
    TitlebarOptions, Window, WindowBounds, WindowOptions,
};

use crate::theme::Tokens;
use crate::widgets::{pt, rgba};

/// 错误报告进程的启动参数：`--crash-report <日志路径>`。
const CRASH_FLAG: &str = "--crash-report";
/// 重启时原样带回的参数：`--restart-arg <参数>`，可重复。
const RESTART_ARG: &str = "--restart-arg";

/// 设置目录名（与 egui 版 `services::config_dir` 一致，日志和它放在一起）。
const APP_ID: &str = "FigCraft";

/// 已经处理过这次崩溃（只处理第一次；后续 panic 不重复弹窗）。
static HANDLED: AtomicBool = AtomicBool::new(false);
/// 事件循环所在线程：只有这个线程的 panic 才算致命。
static MAIN: OnceLock<ThreadId> = OnceLock::new();

/// 最近一份工程快照，由画布每帧维护。
static SNAPSHOT: Mutex<Option<Snapshot>> = Mutex::new(None);

/// 一份工程快照：够用来在崩溃时把文件写回去。
#[derive(Clone)]
struct Snapshot {
    /// 文档修订/路径的指纹：未变化时不重新克隆文档。
    signature: Vec<u64>,
    docs: Vec<DocCopy>,
}

#[derive(Clone)]
struct DocCopy {
    uid: u64,
    name: String,
    path: Option<String>,
    dirty: bool,
    /// 分支标签不是独立文件（内容随主文件保存），紧急保存时跳过。
    main: bool,
    doc: Arc<Document>,
    versions: Vec<Version>,
    branches: Vec<Branch>,
}

/// 安装 panic 钩子。必须在事件循环所在线程调用（`main`）。
pub(crate) fn install() {
    let _ = MAIN.set(std::thread::current().id());
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // 先保持默认行为（stderr / 控制台里有完整 panic 输出）。
        previous(info);
        if MAIN.get() != Some(&std::thread::current().id()) {
            return;
        }
        if HANDLED.swap(true, Ordering::SeqCst) {
            return;
        }
        // 钩子里绝不能再让 panic 逃出去（会变成 abort），任何意外都咽掉。
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle_panic(info)));
    }));
}

/// 记录当前工程快照。画布每帧开头调用；只在修订号、脏标记、版本/分支数量或路径
/// 变化时真正克隆文档，正常情况下只是一次几十项的比较。
pub(crate) fn capture(session: &Session) {
    let signature = signature(session);
    {
        let guard = lock();
        if let Some(s) = guard.as_ref()
            && s.signature == signature
        {
            return;
        }
    }
    // 在锁外构建：克隆期间万一 panic，钩子还能拿锁写日志，不会同线程死锁。
    let docs = session
        .docs
        .iter()
        .map(|d| {
            // A main tab's recovery copy must include branch-tab edits that the engine has not
            // synchronized into `d.branches` yet; `d.document_itself()` alone is stale there.
            let (doc, versions, branches) = if d.branch_of.is_none() {
                branches::recovery_copy(session, d)
            } else {
                (d.document_itself().clone(), d.versions.clone(), d.branches.clone())
            };
            DocCopy {
                uid: d.uid,
                name: d.doc.name.clone(),
                path: d.path.clone(),
                // Branch tabs save with their main file, so a branch-only edit also dirties it.
                dirty: d.branch_of.is_none() && branches::needs_copy(session, d),
                main: d.branch_of.is_none(),
                doc,
                versions,
                branches,
            }
        })
        .collect();
    *lock() = Some(Snapshot { signature, docs });
}

/// 事件循环因 panic 结束后的兜底：保证已经报告过（钩子没跑到时补一次），然后退出。
pub(crate) fn fatal() {
    if !HANDLED.swap(true, Ordering::SeqCst) {
        let _ = std::panic::catch_unwind(|| handle_panic_unknown());
    }
    std::process::exit(70);
}

/// 若当前进程就是错误报告进程（带 `--crash-report`），画出报告窗口并返回 `true`。
/// 必须在 [`run`](crate::run) 的最开头调用，之后的所有启动逻辑都不会执行。
pub(crate) fn run_reporter_if_requested() -> bool {
    let Some((log, restart)) = parse_reporter_args(std::env::args_os().skip(1)) else { return false };

    // 崩溃可能发生在界面语言初始化之前：这里按系统语言选目录，与主界面一致。
    figcraft_i18n::set("system", crate::system_language().as_deref());
    let text = match std::fs::read_to_string(&log) {
        Ok(text) => text,
        Err(e) if log.as_os_str().is_empty() => format!("Monet quit unexpectedly, and its crash log could not be written.\n\n{e}"),
        Err(e) => format!("Monet quit unexpectedly.\n\nThe crash log could not be read:\n{}\n\n{e}", log.display()),
    };
    let log_path = if log.as_os_str().is_empty() { None } else { Some(log) };

    let app = Application::new();
    app.run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, gpui::size(px(760.0), px(560.0)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some("Monet".into()), ..Default::default() }),
            window_min_size: Some(gpui::size(px(520.0), px(360.0))),
            ..Default::default()
        };
        let text = text.clone();
        let log_path = log_path.clone();
        let restart = restart.clone();
        let _ = cx.open_window(options, move |_window, cx| {
            cx.new(move |_cx| CrashReport { text, log_path, restart, copied: false })
        });
        cx.activate(true);
    });
    true
}

/// 从命令行拆出日志路径和重启参数：`None` 表示这不是错误报告进程。
fn parse_reporter_args(mut args: impl Iterator<Item = OsString>) -> Option<(PathBuf, Vec<OsString>)> {
    let mut log = None;
    let mut restart = Vec::new();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some(CRASH_FLAG) => log = args.next().map(PathBuf::from),
            Some(RESTART_ARG) => {
                if let Some(value) = args.next() {
                    restart.push(value);
                }
            }
            _ => {}
        }
    }
    log.map(|log| (log, restart))
}

// ── 崩溃处理 ────────────────────────────────────────────────────────────────

fn handle_panic(info: &PanicHookInfo<'_>) {
    let message = payload_message(info.payload());
    let location = info
        .location()
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_else(|| "(unknown)".into());
    let thread = std::thread::current().name().unwrap_or("main").to_string();
    report(&message, &location, &thread);
}

fn handle_panic_unknown() {
    report("unknown panic", "(unknown)", "main");
}

/// 写日志 → 紧急保存 → 起报告进程。
fn report(message: &str, location: &str, thread: &str) {
    // 每一步都单独兜住：哪怕存盘或写日志出了岔子，也要把报告窗口弹出来。
    let saved = std::panic::catch_unwind(emergency_save).unwrap_or_else(|_| vec!["(saving the project failed)".into()]);
    let log = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();
        write_log(message, location, thread, &saved, &backtrace)
    }))
    .ok()
    .flatten();
    spawn_reporter(log.as_deref());
}

/// 紧急保存：脏文档一律写进 `CrashSaves/<时间>/` 的**新副本**，绝不覆盖原文件
/// （别的进程可能在快照之后改过它）。日志里记下每条副本对应的原始路径。
/// 返回逐条结果，写进日志里显示。
fn emergency_save() -> Vec<String> {
    let snapshot = {
        let Ok(guard) = SNAPSHOT.try_lock() else { return vec!["(could not read the project snapshot)".into()] };
        guard.clone()
    };
    let Some(snapshot) = snapshot else { return vec!["(no project snapshot was available)".into()] };
    if snapshot.docs.is_empty() {
        return vec!["(no open documents)".into()];
    }
    let crash_dir = config_dir().map(|d| d.join("CrashSaves").join(timestamp_compact()));
    let mut lines = Vec::new();
    for doc in snapshot.docs.iter().filter(|d| d.main && d.dirty) {
        let Ok(bytes) = figcraft_format::save_bytes_with_history(&doc.doc, None, &doc.versions, &doc.branches) else {
            lines.push(format!("  {} — could not be serialized", display_name(doc)));
            continue;
        };
        let target = match &crash_dir {
            Some(dir) => dir.join(format!("{}-{}.figcraft", crash_stem(doc), doc.uid)),
            None => match &doc.path {
                // No recoverable folder: writing the original is the only place left, but keep
                // the old path in the log so the user can tell it was a crash overwrite.
                Some(path) => PathBuf::from(path),
                None => {
                    lines.push(format!("  {} — no path and no folder to save it in", display_name(doc)));
                    continue;
                }
            },
        };
        let result = if crash_dir.is_some() { write_crash_atomic(&target, &bytes) } else { write_atomic(&target, &bytes) };
        match result {
            Ok(()) => match &doc.path {
                Some(original) => lines.push(format!("  {} — saved as {} (original: {original})", display_name(doc), target.display())),
                None => lines.push(format!("  {} — saved", target.display())),
            },
            Err(e) => lines.push(format!("  {} — failed: {e}", target.display())),
        }
    }
    if lines.is_empty() {
        lines.push("  (nothing was unsaved)".into());
    }
    lines
}

/// A filesystem-safe stem for a crash copy; the uid appended by the caller keeps it unique.
fn crash_stem(doc: &DocCopy) -> String {
    let name = doc.path.as_deref().and_then(|p| Path::new(p).file_stem()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| doc.name.clone());
    let stem: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') { c } else { '_' }).take(64).collect();
    if stem.is_empty() { "document".into() } else { stem }
}

fn display_name(doc: &DocCopy) -> String {
    doc.path.clone().unwrap_or_else(|| format!("{} (unsaved)", doc.name))
}

/// Atomic replacement through the format crate's exclusive staging files (never a predictable
/// `*.crash-tmp`, which a shared directory could pre-create as a symlink).
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    figcraft_format::write_file_atomic(path, bytes)
}

/// A new crash copy is private: restrict the containing `CrashSaves` folder before writing, then
/// tighten the file itself. `write_file_atomic` preserves an existing destination's permissions,
/// but a brand-new file would otherwise follow the umask (commonly 0644).
fn write_crash_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "crash copy has no parent"))?;
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    figcraft_format::write_file_atomic(path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn write_log(message: &str, location: &str, thread: &str, saved: &[String], backtrace: &str) -> Option<PathBuf> {
    let mut text = String::new();
    let _ = writeln!(text, "Monet {} — crash report", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(text, "Time: {}", timestamp_display());
    let _ = writeln!(text, "Thread: {thread}");
    let _ = writeln!(text, "Panic: {message}");
    let _ = writeln!(text, "Location: {location}");
    let _ = writeln!(text);
    let _ = writeln!(text, "Project files:");
    for line in saved {
        let _ = writeln!(text, "{line}");
    }
    let _ = writeln!(text);
    let _ = writeln!(text, "Command line: {}", std::env::args_os().map(|a| a.to_string_lossy().into_owned()).collect::<Vec<_>>().join(" "));
    let _ = writeln!(text);
    let _ = writeln!(text, "Backtrace:");
    let _ = writeln!(text, "{backtrace}");
    if text.len() > MAX_LOG_BYTES {
        text.truncate(nearest_boundary(&text, MAX_LOG_BYTES));
        text.push_str("\n…(truncated)\n");
    }

    let primary = log_dir().join(format!("crash-{}.log", timestamp_compact()));
    if std::fs::create_dir_all(log_dir()).is_ok() && std::fs::write(&primary, text.as_bytes()).is_ok() {
        return Some(primary);
    }
    let fallback = std::env::temp_dir().join("monet-crash.log");
    std::fs::write(&fallback, text.as_bytes()).ok().map(|()| fallback)
}

/// 日志上限：backtrace 可能很长，留 8 MiB 足够排查又不至于写爆磁盘。
const MAX_LOG_BYTES: usize = 8 * 1024 * 1024;

fn nearest_boundary(text: &str, at: usize) -> usize {
    let mut i = at.min(text.len());
    while i > 0 && !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn spawn_reporter(log: Option<&Path>) {
    let Ok(exe) = std::env::current_exe() else { return };
    let mut cmd = std::process::Command::new(exe);
    cmd.arg(CRASH_FLAG);
    cmd.arg(log.map_or_else(String::new, |p| p.to_string_lossy().into_owned()));
    for arg in std::env::args_os().skip(1) {
        cmd.arg(RESTART_ARG).arg(arg);
    }
    // 不继承调用方的管道 / 控制台句柄：原进程正在退出，报告进程要独立活着。
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    let _ = cmd.spawn();
}

fn payload_message(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".into()
    }
}

// ── 快照指纹 / 目录 / 时间 ──────────────────────────────────────────────────

fn signature(session: &Session) -> Vec<u64> {
    let mut sig = Vec::with_capacity(session.docs.len() * 6);
    for d in &session.docs {
        sig.push(d.uid);
        sig.push(d.revision);
        sig.push(u64::from(d.is_dirty()));
        sig.push(d.versions.len() as u64);
        sig.push(d.branches.len() as u64);
        sig.push(hash_path(d.path.as_deref().unwrap_or("")));
        // Branch-tab edits advance only the tab's revision, so include every tab's stamp or a
        // branch-only change never refreshes the crash snapshot.
        sig.extend(branches::stamp(session, d));
    }
    sig
}

fn hash_path(path: &str) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    for byte in path.bytes() {
        h ^= u64::from(byte);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn lock() -> std::sync::MutexGuard<'static, Option<Snapshot>> {
    SNAPSHOT.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// 与 egui 版一致：`FIGCRAFT_CONFIG_DIR` 优先，其次平台配置目录下的 `FigCraft`。
fn config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("FIGCRAFT_CONFIG_DIR") {
        return (!dir.is_empty()).then(|| PathBuf::from(dir));
    }
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"));
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(all(unix, not(target_os = "macos")))]
    let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
    base.map(|b| b.join(APP_ID))
}

fn log_dir() -> PathBuf {
    config_dir().unwrap_or_else(std::env::temp_dir).join("Logs")
}

fn now() -> chrono::DateTime<chrono::Utc> {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    i64::try_from(secs)
        .ok()
        .and_then(|s| chrono::DateTime::<chrono::Utc>::from_timestamp(s, 0))
        .unwrap_or(chrono::DateTime::<chrono::Utc>::UNIX_EPOCH)
}

fn timestamp_compact() -> String {
    now().format("%Y%m%d-%H%M%S").to_string()
}

fn timestamp_display() -> String {
    now().format("%Y-%m-%d %H:%M:%S UTC").to_string()
}

// ── 错误报告窗口 ────────────────────────────────────────────────────────────

struct CrashReport {
    text: String,
    log_path: Option<PathBuf>,
    restart: Vec<OsString>,
    copied: bool,
}

impl Render for CrashReport {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Tokens::dark();
        let log = self
            .log_path
            .as_ref()
            .map_or_else(|| figcraft_i18n::tr("(the crash log could not be written)").to_string(), |p| p.to_string_lossy().into_owned());
        let log_for_copy = log.clone();
        let restart = self.restart.clone();
        let copied = self.copied;

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgba(t.bg))
            .p(pt(16.0))
            .gap(pt(10.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(pt(2.0))
                    .child(
                        div()
                            .text_size(pt(13.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgba(t.text))
                            .child(figcraft_i18n::tr("Monet quit unexpectedly")),
                    )
                    .child(
                        div()
                            .text_size(pt(11.0))
                            .text_color(rgba(t.text_secondary))
                            .child(figcraft_i18n::tr("The details below were saved to a log file.")),
                    ),
            )
            .child(
                div()
                    .id("crash-log")
                    .flex_1()
                    .min_h(px(0.0))
                    .w_full()
                    .overflow_y_scroll()
                    .rounded(pt(6.0))
                    .bg(rgba(t.field))
                    .p(pt(10.0))
                    .text_size(pt(11.0))
                    .text_color(rgba(t.text))
                    .child(SharedString::from(self.text.clone())),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(pt(4.0))
                    .child(
                        div()
                            .text_size(pt(9.0))
                            .text_color(rgba(t.text_tertiary))
                            .child(figcraft_i18n::tr("Log file")),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(pt(8.0))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_size(pt(11.0))
                                    .text_color(rgba(t.text_secondary))
                                    .child(SharedString::from(log)),
                            )
                            .child(
                                action_button(t, "crash-copy-path", figcraft_i18n::tr("Copy path"), false).on_click(
                                    cx.listener(move |_this, _: &ClickEvent, _window, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(log_for_copy.clone()));
                                    }),
                                ),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(pt(8.0))
                            .child(
                                action_button(t, "crash-copy", figcraft_i18n::tr("Copy details"), false).on_click(
                                    cx.listener(move |this, _: &ClickEvent, _window, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(this.text.clone()));
                                        this.copied = true;
                                        cx.notify();
                                    }),
                                ),
                            )
                            .when(copied, |d| {
                                d.child(
                                    div()
                                        .text_size(pt(11.0))
                                        .text_color(rgba(t.text_tertiary))
                                        .child(figcraft_i18n::tr("Copied")),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(pt(8.0))
                            .child(
                                action_button(t, "crash-close", figcraft_i18n::tr("Close"), false)
                                    .on_click(|_: &ClickEvent, _: &mut Window, cx: &mut App| cx.quit()),
                            )
                            .child(action_button(t, "crash-restart", figcraft_i18n::tr("Restart"), true).on_click(
                                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                    restart_program(&restart);
                                    cx.quit();
                                },
                            )),
                    ),
            )
    }
}

fn action_button(t: Tokens, id: &'static str, label: &str, primary: bool) -> Stateful<Div> {
    div()
        .id(id)
        .h(pt(28.0))
        .px(pt(14.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(pt(5.0))
        .cursor_pointer()
        .text_size(pt(11.0))
        .text_color(rgba(if primary { t.accent_on } else { t.text }))
        .bg(rgba(if primary { t.accent } else { t.field }))
        .hover(move |s| s.bg(rgba(if primary { t.accent_hover } else { t.hover })))
        .child(SharedString::from(label.to_string()))
}

fn restart_program(args: &[OsString]) {
    let Ok(exe) = std::env::current_exe() else { return };
    let _ = std::process::Command::new(exe).args(args).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 报告进程的参数：日志路径单独取出，重启参数按顺序保留（包括以 `--` 开头的）。
    #[test]
    fn reporter_args_split_the_log_from_the_restart_arguments() {
        let args = ["--crash-report", "/tmp/c.log", "--restart-arg", "a.figcraft", "--restart-arg", "--control"]
            .map(OsString::from)
            .into_iter();
        let (log, restart) = parse_reporter_args(args).expect("reporter mode");
        assert_eq!(log, PathBuf::from("/tmp/c.log"));
        assert_eq!(restart, vec![OsString::from("a.figcraft"), OsString::from("--control")]);
        assert!(parse_reporter_args(std::iter::empty::<OsString>()).is_none());
    }

    /// 紧急保存用的原子写：第二次写会替换掉第一次的内容，而不是追加。
    #[test]
    fn an_atomic_write_replaces_the_file() {
        let dir = std::env::temp_dir().join(format!("monet-crash-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("a.figcraft");
        write_atomic(&path, b"one").expect("first write");
        write_atomic(&path, b"two").expect("replace");
        assert_eq!(std::fs::read(&path).expect("read"), b"two");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

