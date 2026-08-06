# 保存数据库配置后跳转实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 数据库连接配置保存成功后立即进入数据库初始化页面。

**Architecture:** `DbConfigManager` 通过成功回调通知父组件，`App` 继续作为页签状态的唯一所有者。只有后端保存成功后触发回调，失败、测试连接和删除操作不触发。

**Tech Stack:** React 19、TypeScript、Vite

---

### Task 1: 接入保存成功导航

**Files:**
- Modify: `src/components/DbConfigManager.tsx`
- Modify: `src/App.tsx`

- [ ] **Step 1: 在 `DbConfigManager` 的 Props 中增加 `onSaved: () => void`，并在保存成功状态更新完成后调用。**
- [ ] **Step 2: 在 `App` 中传入 `onSaved={() => setActiveTab('schema')}`。**
- [ ] **Step 3: 运行 `npm run build`，预期 TypeScript 和 Vite 构建成功。**
- [ ] **Step 4: 运行 `npx oxlint src/App.tsx src/components/DbConfigManager.tsx`，预期无错误。**
