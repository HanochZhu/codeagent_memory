# edit2ripple 优化：论文与开源方案调研

- 日期：2026-10-08
- 问题：读相关论文和开源项目，给出提升 cam 在 Agent Retrieval Bench `edit2ripple` 上表现的方案
- 状态：已结论（方案未实施、未复跑）

## 目标
弄清 cam 图邻居（R@5 0.292 / R@20 0.364 / MRR 0.291）为什么覆盖低，以及同类工作用什么信号补上。

## 探索路径
1. 读 `eval/agent_retrieval/run.py` → 种子只取锚点文件里定义的符号；diff 里出现、但定义在别的文件的标识符被丢掉；只数一跳 `calls/references/implements` 邻居文件，方向和边类型同权；不读 `intent`。
2. 读 ARB 论文（arXiv 2607.24882）→ 官方 lexical 是带 path / basename / symbol 加分的 TF-IDF，不是 BM25；`edit2ripple` 的 gold 是锚点之外受影响的源码**和测试**文件。附录 D：Qwen3-Embedding-4B R@5 0.481 / R@20 0.711，lexical 0.412 / 0.588，RepoMap 0.364 / 0.616，BM25 0.207 / 0.499。论文在另外三项任务上把 Qwen3-8B 和 RepoMap 的 top-20 做 RRF（k=60），MRR 0.230 → 0.271、R@20 0.707 → 0.733。
3. 论文的 clap 示例：锚点 `parser.rs`，intent 是 “Add default_values_if”，gold 是 API 声明 `clap_builder/src/builder/arg.rs` 和 `tests/builder/default_vals.rs`。前者是锚点**依赖**的一侧，后者是测试 → 不能预设“入边比出边重要”。
4. 读 Aider `aider/repomap.py` → 文件级有向图，边权 = `sqrt(引用次数) × mul`；被提及的标识符 ×10，长的 snake/camel 名 ×10，`_` 开头 ×0.1，定义处超过 5 个 ×0.1，来自当前文件的引用 ×50；对被提及文件做 personalized PageRank。
5. 读 RepoGraph（ICLR 2025）→ 以搜索词为中心取 k 跳 ego-graph；直接展开 2 跳效果最差（26.00%，低于基线），1 跳展开最好 → 多跳要靠衰减或摘要，不能平铺。
6. 读 LocAgent（ACL 2025）→ 异构图（目录 / 文件 / 类 / 函数；contain / import / invoke / inherit），配实体名索引和 BM25 做入口，再沿图遍历。
7. 读 ROSE（Zimmermann ICSE 2004 / TSE 2005）与 TARMAQ（SANER 2016）→ 从提交历史挖共改规则，能发现静态分析看不到的耦合；文件粒度召回高于实体粒度。ARB 语料是冻结快照，没有 git 历史，基准上用不了。
8. 读 TCTracer（EMSE 2022）→ 测试与被测代码靠命名约定（NC：去掉 test 后同名；NCC：测试名包含函数名）加静态调用，类级 MAP 0.92。

## 关键发现
- **主要缺口在种子，不在排序**：diff / intent 里提到、定义在别处的符号（示例里的 `default_values_if`）进不了候选。RepoMap 把它们作为 `mentioned_idents` ×10 处理。
- **单一信号都不够**：ARB 结论是语义、词法、结构互补，RRF 融合直接涨分。cam 现在只交了结构这一路。
- **测试文件是一类独立 gold**：要有源码→测试的映射通道，单靠调用图找不全。
- **多跳要有衰减**：RepoGraph 平铺 2 跳会退化；PageRank 的阻尼天然给远处降权。
- **方向权重要测出来**：声明侧（出边）和调用侧（入边）都会出现在 gold 里。

## 结论
按收益 / 成本排序：先不改代码做诊断和事后 RRF；再补种子（diff 行号映射到符号 + 全仓解析被提及的标识符）；把一跳计数换成 personalized PageRank；加源码→测试通道和 path-aware 词法通道；最后多路 RRF。embedding 通道收益最大但成本最高，放最后单独评估。共改挖掘只在产品侧（真实仓库有 `.git`）有意义。计划见 `docs/tasks/2026-10-08-edit2ripple-optimization.md`。

## 续探：第一步（复现、诊断、事后 RRF）

日期 2026-10-08。脚本 `eval/agent_retrieval/diagnose.py`，结果 `eval/agent_retrieval/results/diagnose-20261008-183649.json`。

### 复现
- 环境：`arb` 装在 `eval/agent_retrieval/vendor/.venv`（uv，Python 3.12），同环境装 `huggingface_hub`，否则 `arb download-benchmark` 会调用全局那个和 typer 不兼容的 `hf` 而失败。
- `arb eval-baseline` / `eval-repomap` 必须在 `eval/agent_retrieval/` 下运行：manifest 里的 `chunks_path` 是相对它写的，在仓库根运行会静默得到 `missing_corpus: 58`。
- cam 现在是 R@5 0.300 / R@20 0.378 / MRR 0.297（README 记录的是 0.292 / 0.364 / 0.291）；建 44 个快照索引用时 128 秒。lexical 0.412 / 0.588 / MRR 0.237，与论文一致；RepoMap 0.369 / 0.585，与论文（0.364 / 0.616）有小差异。

### 覆盖上限（99 个 gold 文件）
| 项 | 值 |
|---|---:|
| 被 cam 索引 | 94.9% |
| 一跳可达 | 34.3% |
| 两跳可达 | 57.6% |
| cam top-20 命中 | 32.3% |
| 一跳可达但排在 20 名外 | 2.0% |
| 一跳 / 两跳候选文件数（中位数） | 10.5 / 50 |
| 两跳候选文件数 P90 | 420 |

- **cam 的瓶颈是可达性，不是排序**：一跳内的 gold 几乎都已进 top-20，调方向或边权提不了 R@20。
- **源码 gold 是短板**：源码（n=51）一跳只够到 13.7%，cam@20 13.7%，lexical@20 58.8%；测试（n=48）cam@20 52.1%，与 lexical 56.2% 接近。
- **两跳对融合帮助有限**：两跳新增 23 个 gold（18 个源码），其中 21 个 lexical 或 RepoMap 已在 top-20。它主要能抬名次（15 个在它们的 top-5 里），覆盖几乎不涨，且候选池中位数 50、P90 420，必须有排序。
- **四种方法都漏的 25 个**：只有 2 个在两跳内；5 个是未索引的配置或文档（`Cargo.toml`、`package.json`、`.golangci.yml`、`.md`），标签多为 `shared_config_or_schema`。
- 4 道题锚点文件解析不出符号。

### 事后 RRF（k=60，top-20，无权重、无调参）
| 组合 | R@5 | R@10 | R@20 | MRR | BCY@8k |
|---|---:|---:|---:|---:|---:|
| cam | 0.300 | 0.335 | 0.378 | 0.297 | 0.300 |
| lexical | 0.412 | 0.536 | 0.588 | 0.237 | 0.447 |
| lexical + RepoMap | 0.425 | 0.536 | **0.662** | 0.247 | **0.450** |
| cam + lexical | 0.401 | 0.522 | 0.605 | 0.409 | 0.432 |
| cam + lexical + RepoMap | 0.401 | 0.529 | **0.662** | 0.411 | 0.441 |
| cam + lexical + BM25 | **0.443** | 0.484 | 0.586 | 0.416 | 0.444 |
| cam + lexical + RepoMap + BM25 | 0.441 | 0.534 | 0.654 | **0.441** | 0.443 |

- **cam 在融合里的价值是 MRR**：任何带 cam 的组合 MRR 都在 0.39–0.44，不带 cam 的最高 0.27（论文 `edit2ripple` 上最好的是 pplx 的 0.288）。
- **R@20 靠 lexical + RepoMap**：加不加 cam 都是 0.662；cam 的 top-20 和它们高度重叠，覆盖贡献小。
- **R@5 只略涨**：最好 0.443，仍低于 Qwen3-4B 的 0.481。BCY 没有超过 lexical + RepoMap。

## 仍开放的问题
- cam 自带的 potion-multilingual 向量是否接近 Qwen3-Embedding 的效果，未测。
- RepoMap 本机分数与论文的小差异来源未查。

## 相关路径
- `eval/agent_retrieval/run.py` — 当前图邻居 adapter
- `src/code/index.rs` — 边类型与目标解析
- `src/code/query.rs` — `ref` 一跳查询
