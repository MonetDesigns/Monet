//! 撤销覆盖审计：上游的设计（`storytold/figcraft`）是**每一个操作都进撤销/重做栈**。
//!
//! 这里的审计不看命令表里的 `history` 标记，而是直接跑文档：对每条 `history: false`
//! 的命令执行一次，如果它**改了文档却没产生撤销步骤**，就是缺口 —— 这正是「一大把
//! 操作进不了撤销栈」能在代码里被自动抓住的地方。
//!
//! 注意两点：
//! * `history: false` 本身不是错误：命令可以自己 `grouped(..)` / `record(..)` 恰当地
//!   记一步（如 `vector.edit.begin`、`data.fill`、`widget.*`），审计只看最终结果。
//! * 会碰磁盘 / 网络 / 会话的命令在这里跳过（它们不是文档编辑，多开一个会话会互相干扰）。

use crate::app::{CanvasApp, sample_doc};
use figcraft_engine::cmd::all_commands;
use serde_json::json;

/// 这些前缀会碰磁盘 / 网络 / 会话，审计里跳过（不是文档编辑）。
const SKIP: &[&str] = &[
    "file.",
    "branch.",
    "plugin.",
    "plugin_directory.",
    "libraries.",
    "library.",
    "collab.",
    "online.",
    "live.",
    "invite",
    "templates.",
    "version.",
    "window.",
    "doc.",
    "download",
    "image.place",
];

/// 整条命令表：凡改了文档的命令，必须留下可撤销的一步。
#[test]
fn every_document_editing_command_leaves_an_undo_step() {
    let mut gaps = Vec::new();
    for spec in all_commands() {
        if spec.history || SKIP.iter().any(|p| spec.id.starts_with(p)) {
            continue;
        }
        let mut app = CanvasApp::new(sample_doc());
        let blue = {
            let ds = app.session.ds().expect("sample doc");
            ds.doc.find_by_name("Blue", Some(ds.page)).into_iter().next().expect("Blue")
        };
        app.session.set_selection(vec![blue]);
        let before = app.session.ds().expect("sample doc").doc.clone();
        let depth0 = app.session.undo_depth().0;
        let res = app.session.execute(spec.id, &json!({}));
        let ds = app.session.ds().expect("sample doc");
        // `before` 是同一份文档的 Arc 快照；写时复制保证比较的是内容。
        let changed = *ds.doc != *before;
        let depth1 = app.session.undo_depth().0;
        if changed && depth1 == depth0 {
            gaps.push(format!("{:<34} ok={:<5} undo {}->{}", spec.id, res.is_ok(), depth0, depth1));
        }
    }
    assert!(gaps.is_empty(), "改了文档却没有撤销步骤的命令：\n{}", gaps.join("\n"));
}
