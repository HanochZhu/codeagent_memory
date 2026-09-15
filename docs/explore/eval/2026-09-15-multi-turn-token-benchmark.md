# 多轮对话效果与 token 消耗评测

- 日期：2026-09-15
- 问题：有没有能评测解决方案记忆和代码图、在多轮对话里的效果和 token 消耗的 benchmark；有的话用 `.env` 的 DeepSeek 跑
- 状态：已结论

## 目标

确认仓库内外可用的评测，并实际跑：解法召回、代码图召回、以及带 LLM 的多轮问答 + token。

## 探索路径

1. `eval/coding_life`、`eval/longmemcode`、`DESIGN.md` → 已有两套检索评测，都不进 LLM
2. 兄弟仓 `agentmemory/eval` → coding-agent-life-v1（15 session / 15 query，含 multi-session）和 LongMemEval（会话记忆，非代码）
3. LongMemCode 官方轴是 accuracy / latency / compression / $/1k，本仓 adapter 已报 `$0`，但不跑 LLM
4. `.env` 的 DeepSeek（`deepseek-v4-flash` → `deepseek-flash`）本机 Python ssl 失败，改走 curl；须 `thinking.type=disabled` 否则 CoT 吃光 `max_tokens`、content 为空
5. 新增 `eval/llm_multiturn/run.py`：同一段多轮对话对比 full dump vs cam 检索

## 关键发现

- **解法检索**：coding-agent-life-v1。hash embedder：hit_rate 1.000，R@5 0.933，P@5 0.213（天花板 0.240）。miss 在 temporal / multi-session-causal 的第二枚 gold
- **代码图检索**：LongMemCode clap/fastapi，确定性 scorer，无 LLM。本树 README 已有 clap weighted 0.704、`$ / 1k = 0`
- **多轮 + token**：没有现成「cam × DeepSeek」harness。新建的 runner 把会话当一轮轮追问，每轮只把当前检索注入，历史只留问答
- **解法 × LLM**：15 轮，full / cam 都 1.00 正确；token 23673 → 13255（省 44%）
- **代码 × LLM**：6 轮，full 1.00 / cam 0.50；token 169838 → 6432（省 96%）。miss：`fuse_scores` callers 未进上下文、retention 常量不在函数体、DESIGN 召回没命中「不删除 + cam add」

## 结论

能测「解法」的是 coding-agent-life-v1；能测「代码图」的是 LongMemCode；能同时测多轮效果和 token 的，要用 `eval/llm_multiturn/run.py` 接 DeepSeek。cam 在解法线上不掉点并省约四成 token；代码线上省 token 很明显，但 callers / 模块常量 / CLI 规则检索不够时会掉正确率。

## 仍开放的问题

- coding-life 未跑 potion-multilingual-128M（`~/.cam/models` 空）
- LongMemCode clap 本机未复跑（无 corpora 克隆）
- 代码 cam 检索仍是规则抽取 ident + recall/read/ref，不是带工具的 agent loop

## 相关路径

- `eval/coding_life/run.py` — 解法召回 R@5 / hit rate
- `eval/longmemcode/run.py` — 代码图确定性评分
- `eval/llm_multiturn/run.py` — DeepSeek 多轮问答 + token
- `eval/llm_multiturn/results/llm-multiturn-20260915-235953.json` — 本次 LLM 结果
- `.env` — `DEEPSEEK_API_KEY` / `MODEL=deepseek-v4-flash`
