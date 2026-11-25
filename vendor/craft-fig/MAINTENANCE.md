# 本地维护说明

本目录是 `.fig` 导入器与容器层（wire 层）的一份**本地维护副本**，与 `crates/format/src/fig/` 放在一起，供导入器对照、取证和同步使用。

## 这份副本是什么

- `src/`：`.fig` 的容器层（archive / canvas / kiwi schema / images / vector network / path）和 rest 导入器（`src/rest/`）。
- `vendor/kiwi`、`vendor/figma-rest`：容器层依赖的两个 wire crate，保持原样，便于整包同步。
- 与 `crates/format/src/fig/` 的分工：本地导入器（map 层）做实际转换；这份副本是 wire 层与 rest 映射的参照实现，两者按下面的约定同步。

## 为什么要放在本地单独维护

本地导入器在 wire 层之上多带了若干支持（逐点 `radius` / `mirror` / `cap` / `join`、区域填充、逐点圆角参与命中与导出等），这些能力上游副本没有。因此：

1. 不能直接把导入器切到外部依赖，否则本地多出的支持会丢失。
2. 这份副本作为本地基线保存，改动先落在本地，再按需回灌。

## 同步约定

目标：**持续从上游拿最新实现来改进本地导入器，同时保住本地多出的支持**。

1. 拉取上游最新：把上游仓库 `storytold/craft-libs` 中 `crates/fig`（含其 `crates/kiwi`、`crates/figma-rest`）覆盖到本目录对应位置。
2. diff 取证：先 `git diff` 看上游相对本地基线改了什么，按**文件级 / 函数级**逐项判断，不做整包 merge。
3. 只取 wire 层与解析逻辑的改动：容器格式、schema 字段、解码边界、样式行解析等。逐点样式、区域填充等本地增强以本地实现为准，不被上游覆盖。
4. 把可借鉴的改动落到实际运行代码 `crates/format/src/fig/`，本目录只保留同步基线，不参与构建、不进 workspace。
5. 每次同步在提交信息里记下上游版本标识与取用范围，便于回溯。

> 命名：目录名沿用上游 crate 名 `craft-fig`，便于与上游对应；它在本仓库只是源码基线，不作为 Rust 依赖被 `Cargo.toml` 引用。

## 逐点样式字段的当前状态

- 逐点 `radius` / `mirror` / `cap` / `join`：本地导入器已支持并落到 `doc::Vertex`。
- 逐点 `cornerSmoothing`、逐段 `strokeCapSize`（端点/箭头尺寸）：本地正在补，见 `crates/format/src/fig/map/layer.rs` 的样式行处理与 `crates/doc/src/vector.rs` 的 `Vertex`。
