# edit2ripple 优化计划

- 日期：2026-10-08
- 状态：进行中（步骤 1、2 已完成，结果见探索文档「续探：第一步」）
- Issue：https://github.com/HanochZhu/codeagent_memory/issues/5
- 问题：在不对 58 题过拟合的前提下，把 cam 的 `edit2ripple` 从 R@5 0.292 提到超过 lexical（0.412），R@20 靠近 Qwen3-4B（0.711），MRR 不低于 0.29

依据见 `docs/explore/eval/2026-10-08-edit2ripple-optimization.md`。

## 目标
cam 以图为主、融合词法和测试映射，在 ARB 上给出可复现的分数，且每一步单独报增益。

## 步骤
1. **复现与诊断** — 下载数据，复跑当前 adapter，结果对上 README。逐题记录：gold 是源码还是测试；在一跳 / 两跳图里吗；lexical 找得到吗。得到“图能到达的上限”。
2. **事后 RRF** — 用 `arb eval-baseline --ranker lexical` 和 RepoMap 的 details，与 cam top-20 做 RRF（k=60），不改 cam。完成标准：知道融合能涨多少，再决定后续投入。
   步骤 1–2 的结论改变了后续优先级：cam 的瓶颈是源码 gold 的可达性，不是排序，方向和边权消融对 R@20 帮助有限；在融合里 cam 主要贡献 MRR。步骤 3（种子）和步骤 6（自带词法通道）优先；步骤 4 的 PageRank 主要用来给两跳候选排序；另需评估是否把配置 / 文档文件纳入候选（5 个 gold 不在索引里）。
3. **种子** — 用 diff 的 `@@` 旧行号区间找包含它的符号（基准是 base commit）；diff 和 intent 里的标识符在全仓解析定义（不限锚点文件）；按 RepoMap 规则加权（被提及 ×10，长名 ×10，私有名、定义超过 5 处的名字 ×0.1）。
4. **排序** — 一跳计数换成以种子为 personalization 的 PageRank；边权 `sqrt(次数)`；入边、出边、边类型权重各自做消融后再定。
5. **测试通道** — 命名约定（`foo.go` ↔ `foo_test.go`、`test_foo.py`、`tests/**/foo*.rs`，以及 NC / NCC）加“引用了种子符号的测试文件”。
6. **词法通道** — path-aware TF-IDF：query = intent + diff 标识符，加 path / basename / symbol 加分。
7. **融合与调参** — 图、测试、词法三路 RRF；权重用 leave-one-repo-out 交叉验证选，报交叉验证分数。
8. **可选：embedding 通道** — 先试 cam 自带的 potion；有 GPU 再对照 Qwen3-Embedding-4B。
9. **产品侧：共改** — 有 `.git` 时挖文件级共改（TARMAQ 式），只在真实仓库评估，不进 ARB 分数。

## 相关路径
- `eval/agent_retrieval/run.py` — adapter，步骤 3–7 主要改这里
- `eval/agent_retrieval/diagnose.py` — 覆盖上限、按 gold 类型拆分、事后 RRF；每步改完重跑
- `src/code/index.rs` — 若步骤 4 证明需要 import / extends 边，在这里补
- `README.md` / `README.zh.md` — 更新评测表
