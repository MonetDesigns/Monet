# -*- coding: utf-8 -*-
"""Merge the UI inventory TSVs into one CSV with an EGUI / GPUI / source-file column."""
import csv, io, os, sys

BASE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(os.path.dirname(BASE), "ui-inventory.csv")

PARTS = ["menu1.tsv", "menu2.tsv", "menu3.tsv", "toolbar1.tsv", "canvas1.tsv",
         "panel1.tsv", "panel2.tsv", "dialog1.tsv", "motion.tsv"]

# 分类前缀 -> 源文件（先匹配最长的前缀）
SRC = [
    ("菜单·", "crates/engine/src/catalog.rs; crates/ui-egui/src/menus.rs"),
    ("标题栏", "crates/ui-egui/src/app/tabs.rs; crates/ui-egui/src/app/frame.rs"),
    ("右键菜单·画布", "crates/ui-egui/src/menus.rs; crates/ui-egui/src/canvas/input.rs"),
    ("右键菜单·图层", "crates/ui-egui/src/menus.rs; crates/ui-egui/src/panels/layers.rs"),
    ("右键菜单·页面", "crates/ui-egui/src/panels/layers.rs"),
    ("工具栏·主工具栏", "crates/ui-egui/src/toolbar.rs"),
    ("工具栏·画板工具栏", "crates/ui-egui/src/canvas/board_bar.rs"),
    ("工具栏·只读工具栏", "crates/ui-egui/src/toolbar.rs"),
    ("工具栏·矢量编辑工具栏", "crates/ui-egui/src/toolbar.rs"),
    ("工具栏·ShapeBuilder", "crates/ui-egui/src/toolbar.rs"),
    ("工具栏·Drawing 设置条", "crates/ui-egui/src/toolbar.rs"),
    ("画板选区栏", "crates/ui-egui/src/canvas/board_bar.rs"),
    ("部件条", "crates/ui-egui/src/canvas/widget_bar.rs"),
    ("画板便利栏", "crates/ui-egui/src/canvas/board_facilitate.rs"),
    ("画板连接", "crates/ui-egui/src/canvas/board_connect.rs"),
    ("画布覆盖·overlays", "crates/ui-egui/src/canvas/overlays.rs"),
    ("画布覆盖·handles", "crates/ui-egui/src/canvas/handles.rs"),
    ("画布覆盖·rulers", "crates/ui-egui/src/canvas/rulers.rs"),
    ("画布覆盖·guides", "crates/ui-egui/src/canvas/guides.rs"),
    ("画布覆盖·measure", "crates/ui-egui/src/canvas/measure_overlay.rs"),
    ("画布覆盖·frame_labels", "crates/ui-egui/src/canvas/frame_labels.rs"),
    ("画布覆盖·frame_stats", "crates/ui-egui/src/canvas/frame_stats.rs"),
    ("画布覆盖·comments", "crates/ui-egui/src/canvas/comments.rs"),
    ("画布覆盖·live_text", "crates/ui-egui/src/canvas/live_text.rs"),
    ("画布覆盖·outline", "crates/ui-egui/src/canvas/outline.rs"),
    ("画布覆盖·selection_spacing", "crates/ui-egui/src/canvas/selection_spacing.rs"),
    ("画布覆盖·proto_canvas", "crates/ui-egui/src/canvas/proto_canvas.rs"),
    ("画布覆盖·background", "crates/ui-egui/src/canvas/background.rs"),
    ("画布覆盖·cursors", "crates/ui-egui/src/canvas/cursors.rs"),
    ("浮层·", "crates/ui-egui/src/popover.rs"),
    ("左面板·图标栏", "crates/ui-egui/src/panels.rs"),
    ("左面板·文件头", "crates/ui-egui/src/panels/layers.rs"),
    ("左面板·Pages", "crates/ui-egui/src/panels/layers.rs"),
    ("左面板·Layers", "crates/ui-egui/src/panels/layers.rs"),
    ("左面板·Assets", "crates/ui-egui/src/panels/assets_panel.rs"),
    ("左面板·版本历史", "crates/ui-egui/src/panels/versions.rs"),
    ("左面板·快捷键", "crates/ui-egui/src/panels/shortcuts_panel.rs"),
    ("右面板·顶栏", "crates/ui-egui/src/panels/right.rs"),
    ("右面板·Design 排版", "crates/ui-egui/src/panels/typography.rs; typography/{axes,fields,glyphs,picker,spacing}.rs"),
    ("右面板·Design 描边", "crates/ui-egui/src/panels/right/stroke.rs"),
    ("右面板·Design 效果", "crates/ui-egui/src/panels/right/effects.rs"),
    ("右面板·Design 填充", "crates/ui-egui/src/panels/right/paints.rs"),
    ("右面板·Design 选区色", "crates/ui-egui/src/panels/selection_colors.rs"),
    ("右面板·Design 矢量", "crates/ui-egui/src/panels/vector_panel.rs"),
    ("右面板·Design 布局", "crates/ui-egui/src/panels/layout_panel.rs"),
    ("右面板·Design Scale", "crates/ui-egui/src/panels/scale_panel.rs"),
    ("右面板·Design Section", "crates/ui-egui/src/panels/section_panel.rs"),
    ("右面板·Design 实例", "crates/ui-egui/src/panels/component_panel.rs"),
    ("右面板·Design Frame 工具", "crates/ui-egui/src/panels/frame_presets.rs"),
    ("右面板·Design 变量模式", "crates/ui-egui/src/panels/variable_ui.rs"),
    ("右面板·Design 导出", "crates/ui-egui/src/panels/export_panel.rs"),
    ("右面板·Design", "crates/ui-egui/src/panels/right.rs"),
    ("右面板·Prototype 编辑器", "crates/ui-egui/src/panels/proto_editor.rs"),
    ("右面板·Prototype", "crates/ui-egui/src/panels/proto_panel.rs"),
    ("右面板·Inspect", "crates/ui-egui/src/panels/right/inspect.rs"),
    ("右面板·Dev compare", "crates/ui-egui/src/panels/dev_compare.rs"),
    ("对话框", "crates/ui-egui/src/dialogs.rs (+ dialogs/*)"),
    ("独立窗口", "crates/ui-egui/src/windows.rs; dialogs/variables_panel.rs"),
    ("首页", "crates/ui-egui/src/home.rs (+ home/*)"),
    ("Dev Mode", "crates/ui-egui/src/panels/dev_mode.rs"),
    ("动画·GPUI 特有", "crates/ui-gpui/src/motion/*.rs; canvas/motion_overlay.rs"),
    ("动画", "crates/ui-egui/src/panels/proto_panel.rs"),
    ("演示", "crates/ui-egui/src/present.rs"),
    ("通知", "crates/ui-egui/src/app.rs"),
    ("颜色选择器", "crates/ui-egui/src/widgets/colorpicker.rs"),
    ("画布覆盖", "crates/ui-egui/src/canvas/overlays.rs; canvas/board_facilitate.rs"),
]


DIALOG_SRC = {
    "About FigCraft (About tab)": "dialogs/credits.rs", "About > Contributors": "dialogs/credits.rs",
    "About > Models": "dialogs/credits.rs", "Preferences": "crates/ui-egui/src/app/preferences.rs",
    "Keyboard shortcuts": "panels/shortcuts_panel.rs", "Find and replace": "crates/ui-egui/src/menus.rs",
    "Rename layers (批量)": "dialogs/batch_rename.rs", "Menu input — Create link": "dialogs/menu_input.rs",
    "Menu input — Offset path": "dialogs/menu_input.rs", "Menu input — Move to page": "dialogs/menu_input.rs",
    "Export": "dialogs/export.rs", "Color profile": "dialogs/color_profile.rs",
    "Import report": "dialogs/import_report.rs", "Missing fonts": "dialogs/missing_fonts.rs",
    "Create from color": "dialogs/sampled_color.rs", "Plugins (Installed tab)": "dialogs/plugins_ui.rs",
    "Plugins > Browse tab": "dialogs/plugins_ui.rs", "Plugin install/update confirm": "dialogs/plugins_ui.rs",
    "Plugin parameters form": "dialogs/plugins_ui.rs", "Plugin panels": "dialogs/plugins_ui.rs",
    "Save file (插件请求保存)": "dialogs/plugins_ui.rs", "Unsaved changes (关闭窗口)": "windows.rs",
    "Unsaved changes (关闭 tab)": "windows.rs", "Pending open card": "crates/ui-egui/src/pending_open.rs",
    "BranchDialog": "dialogs/branch_ui.rs", "Updates (检查更新)": "dialogs/updates_ui.rs",
}



# 分类前缀 -> GPUI 源文件（ui-gpui 是单文件大杂烩，panels.rs 承载几乎全部检查器）
GPUI_SRC = [
    ("菜单·", "crates/ui-gpui/src/main_menu.rs; widgets/menu.rs"),
    ("标题栏", "crates/ui-gpui/src/title_bar.rs; tabs.rs"),
    ("右键菜单·画布", "crates/ui-gpui/src/context_menu.rs"),
    ("右键菜单·图层", "crates/ui-gpui/src/context_menu.rs; layer_tree.rs"),
    ("右键菜单·页面", "crates/ui-gpui/src/layer_tree.rs"),
    ("工具栏·主工具栏", "crates/ui-gpui/src/toolbar.rs"),
    ("工具栏·画板工具栏", "crates/ui-gpui/src/board_bar.rs"),
    ("工具栏·只读工具栏", "crates/ui-gpui/src/toolbar.rs"),
    ("工具栏·矢量编辑工具栏", "crates/ui-gpui/src/toolbar.rs"),
    ("工具栏·ShapeBuilder", "crates/ui-gpui/src/toolbar.rs"),
    ("工具栏·Drawing 设置条", "crates/ui-gpui/src/toolbar.rs"),
    ("画板选区栏", "crates/ui-gpui/src/board_bar.rs"),
    ("部件条", "crates/ui-gpui/src/widget_bar.rs"),
    ("画板便利栏", "crates/ui-gpui/src/board_facilitate.rs"),
    ("画板连接", "crates/ui-gpui/src/canvas/board_connect.rs"),
    ("画布覆盖·overlays", "crates/ui-gpui/src/canvas/overlays.rs"),
    ("画布覆盖·handles", "crates/ui-gpui/src/canvas/overlays.rs; canvas/mod.rs"),
    ("画布覆盖·rulers", "crates/ui-gpui/src/canvas/rulers.rs"),
    ("画布覆盖·guides", "crates/ui-gpui/src/canvas/guides.rs"),
    ("画布覆盖·measure", "crates/ui-gpui/src/canvas/measure_overlay.rs"),
    ("画布覆盖·frame_labels", "crates/ui-gpui/src/canvas/frame_labels.rs"),
    ("画布覆盖·frame_stats", "crates/ui-gpui/src/canvas/frame_stats.rs"),
    ("画布覆盖·comments", "crates/ui-gpui/src/comments.rs"),
    ("画布覆盖·live_text", "crates/ui-gpui/src/canvas/live_text.rs"),
    ("画布覆盖·outline", "crates/ui-gpui/src/canvas/outline.rs"),
    ("画布覆盖·selection_spacing", "crates/ui-gpui/src/canvas/selection_spacing.rs"),
    ("画布覆盖·proto_canvas", "crates/ui-gpui/src/canvas/proto_canvas.rs"),
    ("画布覆盖·background", "crates/ui-gpui/src/canvas/overlays.rs"),
    ("画布覆盖·cursors", "crates/ui-gpui/src/canvas/cursors.rs"),
    ("浮层·", "crates/ui-gpui/src/popovers.rs"),
    ("左面板·图标栏", "crates/ui-gpui/src/panels.rs"),
    ("左面板·文件头", "crates/ui-gpui/src/layer_tree.rs"),
    ("左面板·Pages", "crates/ui-gpui/src/layer_tree.rs"),
    ("左面板·Layers", "crates/ui-gpui/src/layer_tree.rs"),
    ("左面板·Assets", "crates/ui-gpui/src/panels/extra.rs"),
    ("左面板·版本历史", "crates/ui-gpui/src/panels/extra.rs"),
    ("左面板·快捷键", "crates/ui-gpui/src/panels/extra.rs"),
    ("右面板·顶栏", "crates/ui-gpui/src/panels.rs"),
    ("右面板·Design 排版", "crates/ui-gpui/src/panels.rs"),
    ("右面板·Design", "crates/ui-gpui/src/panels.rs (+ panels/extra.rs)"),
    ("右面板·Prototype", "crates/ui-gpui/src/panels.rs"),
    ("右面板·Inspect", "crates/ui-gpui/src/panels.rs; canvas/dev.rs"),
    ("右面板·Dev compare", "crates/ui-gpui/src/panels.rs"),
    ("对话框", "crates/ui-gpui/src/dialogs.rs"),
    ("独立窗口", "crates/ui-gpui/src/windows.rs"),
    ("首页", "crates/ui-gpui/src/home/mod.rs"),
    ("Dev Mode", "crates/ui-gpui/src/canvas/dev.rs"),
    ("动画·GPUI 特有", "crates/ui-gpui/src/motion/{inspector,timeline,keyframe_field,clip_popover,easing,host}.rs; canvas/motion_overlay.rs"),
    ("动画", "crates/ui-gpui/src/panels.rs"),
    ("演示", "crates/ui-gpui/src/present.rs (+ present/*)"),
    ("通知", "crates/ui-gpui/src/lib.rs (toast)"),
    ("颜色选择器", "crates/ui-gpui/src/popovers.rs"),
    ("画布覆盖", "crates/ui-gpui/src/canvas/overlays.rs"),
]


def gpui_source_of(cat: str) -> str:
    best = None
    for prefix, path in GPUI_SRC:
        if cat.startswith(prefix) and (best is None or len(prefix) > len(best[0])):
            best = (prefix, path)
    return best[1] if best else ""

def source_of(cat: str, name: str) -> str:
    if cat == "对话框" and name in DIALOG_SRC:
        f = DIALOG_SRC[name]
        return f if f.startswith("crates/") else "crates/ui-egui/src/" + f
    best = None
    for prefix, path in SRC:
        if cat.startswith(prefix) and (best is None or len(prefix) > len(best[0])):
            best = (prefix, path)
    return best[1] if best else ""


rows, warned = [], 0
for part in PARTS:
    path = os.path.join(BASE, part)
    with io.open(path, encoding="utf-8") as f:
        for n, line in enumerate(f, 1):
            line = line.rstrip("\n").rstrip("\r")
            if not line.strip():
                continue
            if line.lstrip("\ufeff").startswith("分类\t"):
                continue
            cols = line.split("\t")
            if len(cols) > 4:
                warned += 1
                print(f"WARN {part}:{n} has {len(cols)} cols: {line[:90]!r}")
            cols = (cols + ["", "", "", ""])[:4]
            cat, name, cmd, note = [c.strip() for c in cols]
            if not name:
                continue
            # GPUI does not have these yet (its dialogs list has no live/publish/plugin-run/
            # online-file panels; its Assets panel has no Libraries sub-view; its Inspect panel has
            # no Plugins section). The *menu items* still render (the menu is catalog-driven), so
            # only the dialog / panel rows are left unticked.
            GPUI_MISSING_DIALOGS = {
                "Join live session", "Publish to server", "File published", "Pending open card",
                "Plugin install/update confirm", "Plugin parameters form", "Plugin panels",
                "Save file (插件请求保存)", "Remove from server (在线文件)", "Rename on the server",
                "Create from color",
            }
            GPUI_MISSING_ASSETS = {
                "Libraries view back button", "Publish form groups (Components/Styles/Variable collections)",
                "Libraries in this file", "Add library file button",
            }
            # EGUI / GPUI columns
            if cat.startswith("动画·GPUI 特有"):
                egui, gpui = "", "✓"
            elif cat == "对话框" and name in GPUI_MISSING_DIALOGS:
                egui, gpui = "✓", ""
            elif cat == "左面板·Assets" and name in GPUI_MISSING_ASSETS:
                egui, gpui = "✓", ""
            elif cat.startswith("右面板·Inspect") and name == "Plugins section":
                egui, gpui = "✓", ""
            elif cat == "左面板·版本历史" and name.startswith("Branch menu"):
                egui, gpui = "✓", ""
            else:
                egui, gpui = "✓", "✓"
            if egui == "" :
                status = "GPUI 独有"
            elif gpui == "":
                status = "GPUI 未实现（上游 EGUI 有）"
            else:
                status = "已实现"
            rows.append([cat, name, cmd or "—", egui, gpui, note, source_of(cat, name), gpui_source_of(cat), status])

os.makedirs(os.path.dirname(OUT), exist_ok=True)
with io.open(OUT, "w", encoding="utf-8-sig", newline="") as f:
    w = csv.writer(f)
    w.writerow(["分类", "界面元素", "绑定命令", "EGUI", "GPUI", "行为 / 显示规则备注", "EGUI 源文件", "GPUI 源文件", "本地状态（Monet）"])
    w.writerows(rows)

cats = {}
for r in rows:
    cats[r[0]] = cats.get(r[0], 0) + 1
print(f"OK -> {OUT}")
print(f"rows={len(rows)} warnings={warned}")
print("--- per category ---")
for k in sorted(cats, key=lambda x: (-cats[x], x)):
    print(f"{cats[k]:4d}  {k}")
