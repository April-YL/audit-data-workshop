---
name: duckdb-docs
description: "DuckDB 语法、类型、精度或版本行为需要官方依据时"
argument-hint: <question or keyword>
allowed-tools: Bash
---

## 项目职责与交接（2026-10-28 修订）

本文件为项目适配版本，来源及修改见 [安装清单](../skills-lock.json)。
共同授权及专项选择见 [统一协作入口](../../../docs/03_HANDOFF.md#skill-coordination)；使用前按任务选择专项，不默认加载全部 Skill。下方上游方法不得扩大已批准范围；职责交接仅指任务衔接，不代表自动派发子代理。

- **触发条件**：DuckDB 语法、类型、精度或版本行为需要官方依据时；简单已验证查询不反复检索。
- **输入**：具体技术问题、实际嵌入引擎版本及已有代码证据。
- **输出**：官方原始来源链接、版本适用性、行为结论及不确定项；在线只读查询，不发送客户数据。
- **交接**：将来源和限制交回 SQL／Rust 实现；仍不确定则提议经授权的虚构小样例验证。
- **停止条件**：找不到对应版本证据时标为未核实；不自动安装 CLI、扩展、索引或建立用户目录缓存。
- **必要边界**：写入先确认，Git 另行授权；保持现有技术栈，核对实际版本。脚本、示例、依赖、额外文档及外部操作均不因 Skill 建议而自动获授权。

# DuckDB documentation for this project

## Workflow

1. Read the project's Cargo lockfile, engine setup and existing build records to establish the actual embedded DuckDB version; the current baseline is 1.4.4. Do not install or upgrade a separate CLI.
2. Search official DuckDB documentation at https://duckdb.org/docs/ using only technical terms. Prefer the matching version archive and versioned release notes/source when available. A current/LTS search label does not prove compatibility with 1.4.4.
3. Open the specific primary-source page. Report the supported behavior, its source URL and any version uncertainty. Do not infer exact monetary conversion, null ordering, date parsing or pivot behavior solely from a snippet.
4. Where version-specific behavior remains uncertain, propose a minimal synthetic probe against the project's existing engine. Obtain approval for any generated inputs, run database or report before executing. Never use customer data as a network query.
5. No local full-text index, httpfs/fts installation, credential discovery, DuckLake setup, home-directory cache, persistent state or network ATTACH is part of this adapted Skill. Use existing web tools; if unavailable, report the documentation check as unverified.
6. Give implementers a concise result: exact question, engine version, source, supported syntax/semantics, limitations and proposed verification. Documentation evidence is separate from executed tests and desktop acceptance.
