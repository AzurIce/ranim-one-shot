---
description: 启动一次 one-shot run（用法：/run <topic> <model-slug> [run序号]）
argument-hint: <topic> <model-slug> [run序号]
skills: ranim-one-shot
---

用户要为一个 topic 启动一次 one-shot run。参数：$ARGUMENTS，依次为
`<topic> <model-slug> [run序号]`。严格按 AGENTS.md 执行：

1. 参数不全时先向用户确认（run 序号缺省取 origin/main 上该 topic 现有
   run 数 +1）。
2. 确认 `topics/<topic>/prompt.md` 存在；不存在则停止，提示按 AGENTS.md
   的 "Adding a topic" 先向 base 提 PR。
3. 按 AGENTS.md 的 Starting a run：从 `origin/base` 创建 worktree/分支
   `<topic>-run<N>-<modelslug>`，并把 fork commit 记入 meta.toml。
4. 逐字执行 `topics/<topic>/prompt.md` 的内容作为本 run 的原始 prompt，
   不要改写、不要翻译；方法论遵循 `.agents/skills/ranim-one-shot/SKILL.md`。
5. 完成后按 AGENTS.md 的交付清单出 PR。
