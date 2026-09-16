# 解法 / 代码图与同类记忆机制对照

- 日期：2026-09-16
- 问题：把 cam 的 benchmark 挪到 README 靠前，并和同类记忆机制对比后做成表和折线图
- 状态：已结论

## 目标

确认能公平对照的系统与数字，把结果写进 README 靠前位置。

## 探索路径

1. 读 `docs/index.md` + [2026-09-15-multi-turn-token-benchmark.md](2026-09-15-multi-turn-token-benchmark.md) → 本仓只有 cam 自己的三套分，没有对外对照
2. 核对本仓 LongMemCode / coding-life 数字，并对齐 agentmemory 已发表 scorecard（v0.9.26）与 codegraph clap 分 → 两面正交，必须拆成两场
3. 复跑 `eval/coding_life/run.py --adapter grep` → R@5 0.967 / P@5 0.227，只 miss temporal，与 agentmemory 文档一致
4. 沿用本树 cam RRF / sum（2026-09-16）和 LongMemCode clap vs codegraph（2026-09-14）
5. agentmemory hybrid 用已发表 v0.9.26 分数（本机未复跑 iii + sandbox）
6. Mem0 / Zep / Letta 没有 coding-life 或 LongMemCode 数字，不进分数表

## 关键发现

- **不能一张总分比所有记忆系统。** 解法面比 agentmemory / grep；代码图面比 codegraph。
- **解法面（同一 `score.ts`）：** grep 0.967；cam sum 0.933；cam RRF **1.000**（天花板）；hybrid 文档 1.000。
- **RRF 的 lift：** 找回 q-011 causal 与 q-015 temporal 的第二枚 gold。grep 只在 temporal miss。
- **代码图面：** clap weighted cam 0.704 vs codegraph 0.702；callers 0.397 vs 0.379。
- **图表：** `eval/charts/generate.py` 读已完成的 JSON，画出柱状图（headline / 分操作 / token）和折线图（题型 R@5 / 分操作 / token）。RRF 结果文件：`eval/coding_life/results/cam-life-cam-rrf-20260916-094945.json`。

## 结论

README 评测段放到「为什么需要 cam」之后。分数表只放同语料数字；机制表说明 cam / agentmemory / codegraph 各管哪一面。

## 仍开放的问题

- agentmemory hybrid 1.000 未在本机复跑
- potion-multilingual 的 cam life 仍未跑
- Mem0 / Zep 没有接到 coding-life

## 相关路径

- `README.md` / `README.zh.md` — 靠前评测段
- `eval/charts/` — 折线图与生成脚本
- `eval/coding_life/run.py` — `--adapter grep|cam`
- `eval/longmemcode/README.md` — clap vs codegraph
