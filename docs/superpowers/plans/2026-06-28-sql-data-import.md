# SQL Data Import Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让现有数据导入流程同时支持按文件名识别并直接执行 `.sql` 数据文件。

**Architecture:** 扩展现有导入文件模型，统一扫描 `.csv` / `.sql` 两类文件；后端在任务执行阶段按文件类型分派到 CSV 批量插入或 SQL 原样执行；前端仅补充类型展示和进度文案差异。

**Tech Stack:** React 19、TypeScript、Tauri 2、Rust、Cargo Test

---

### Task 1: 扩展后端文件模型与扫描能力

**Files:**
- Modify: `src-tauri/src/models.rs`
- Modify: `src-tauri/src/csv_parser.rs`
- Test: `src-tauri/src/csv_parser.rs`

- [ ] **Step 1: 写扫描 `.sql` 文件的失败测试**
- [ ] **Step 2: 运行指定测试并确认失败**
- [ ] **Step 3: 为文件模型增加类型字段，实现 `.sql` 文件识别与文件名解析**
- [ ] **Step 4: 运行扫描测试并确认通过**

### Task 2: 扩展后端导入执行能力

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Test: `src-tauri/src/commands.rs`

- [ ] **Step 1: 写 `.sql` 文件读取与导入目标识别的失败测试**
- [ ] **Step 2: 运行指定测试并确认失败**
- [ ] **Step 3: 实现 SQL 文件读取和按文件类型分派执行**
- [ ] **Step 4: 运行后端测试并确认通过**

### Task 3: 更新前端类型和展示

**Files:**
- Modify: `src/types.ts`
- Modify: `src/components/FileScanner.tsx`
- Modify: `src/components/ImportManager.tsx`
- Modify: `src/App.tsx`
- Modify: `src/App.css`

- [ ] **Step 1: 同步前端文件类型定义**
- [ ] **Step 2: 更新扫描页文案与文件类型展示**
- [ ] **Step 3: 更新任务列表对 `.sql` 文件的状态展示**
- [ ] **Step 4: 运行前端构建验证**

### Task 4: 整体验证

**Files:**
- Modify: `docs/superpowers/specs/2026-06-28-sql-data-import-design.md`
- Modify: `docs/superpowers/plans/2026-06-28-sql-data-import.md`

- [ ] **Step 1: 运行 `cargo test` 验证 Rust 侧**
- [ ] **Step 2: 运行 `npm run build` 验证前端**
- [ ] **Step 3: 汇总变更与剩余风险**
