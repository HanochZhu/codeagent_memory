<div align="center">

<img src="assets/logo.svg" alt="cam logo" width="128" height="128">

# cam

[English](README.md) · [中文](README.zh.md) · [日本語](README.ja.md) · [한국어](README.ko.md) · [العربية](README.ar.md) · [🌐 ウェブサイト](https://hanochzhu.github.io/cam-web/?lang=ja)

### コーディングエージェント向けに特化したローカル・マルチターン会話メモリシステム

**コードグラフ + メモリ · シンボル単位のピンポイント読取 · 100% ローカル · MCP + CLI**

**Rust 製カーネル**

[![Website](https://img.shields.io/badge/website-オンライン-brightgreen.svg)](https://hanochzhu.github.io/cam-web/?lang=ja)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/Rust-edition%202021-orange.svg)](https://www.rust-lang.org/)
[![Install](https://img.shields.io/badge/install-cargo--git-blue.svg)](#クイックスタート)

[![Windows](https://img.shields.io/badge/Windows-supported-blue.svg)](#対応プラットフォーム)
[![macOS](https://img.shields.io/badge/macOS-supported-blue.svg)](#対応プラットフォーム)
[![Linux](https://img.shields.io/badge/Linux-supported-blue.svg)](#対応プラットフォーム)

[![Claude Code](https://img.shields.io/badge/Claude_Code-supported-blueviolet.svg)](#対応エージェント)
[![Cursor](https://img.shields.io/badge/Cursor-supported-blueviolet.svg)](#対応エージェント)
[![Codex](https://img.shields.io/badge/Codex-supported-blueviolet.svg)](#対応エージェント)
[![Windsurf](https://img.shields.io/badge/Windsurf-supported-blueviolet.svg)](#対応エージェント)
[![GitHub Copilot](https://img.shields.io/badge/GitHub_Copilot-supported-blueviolet.svg)](#対応エージェント)
[![Continue](https://img.shields.io/badge/Continue-supported-blueviolet.svg)](#対応エージェント)
[![Cline](https://img.shields.io/badge/Cline-supported-blueviolet.svg)](#対応エージェント)
[![JetBrains](https://img.shields.io/badge/JetBrains-supported-blueviolet.svg)](#対応エージェント)

</div>

## 目次

- [なぜ cam なのか？](#なぜ-cam-なのか)
- [クイックスタート](#クイックスタート)
- [主な機能](#主な機能)
- [動作原理](#動作原理)
- [エージェントのワークフロー](#エージェントのワークフロー)
- [CLI リファレンス](#cli-リファレンス)
- [メモリ規則](#メモリ規則)
- [設定](#設定)
- [ベンチマーク](#ベンチマーク)
- [対応エージェント](#対応エージェント)
- [対応言語](#対応言語)
- [対応プラットフォーム](#対応プラットフォーム)
- [ライセンス](#ライセンス)

ガイド: [インタラクティブ紹介サイト](https://hanochzhu.github.io/cam-web/?lang=ja) · [メインエージェントとサブエージェント](docs/agents.md) · [IDE別 MCP 設定](docs/mcp.md) · [中文说明](docs/agents.zh.md)

---

## なぜ cam なのか？

AI コーディングエージェントがリポジトリを探索する際、通常は `grep`、`glob`、ファイル全体の `Read` を1つずつ実行して構造を再探索し、次のセッションではすべてを忘れてしまいます。

**`cam` は単一のバイナリでエージェントに2つのローカル検索プレーンを提供します：**

| プレーン | 格納するもの | エージェントの利用方法 |
| --- | --- | --- |
| **コードグラフ（Code Graph）** | tree-sitter シンボル（関数、型、メソッド）による仮想ファイルシステム + 1ホップの callers/callees | `ls`、`find`、`read src/file.rs/symbol`、`ref symbol --dir in\|out` — ファイル全体ではなくシンボル単位のピンポイント読み取り |
| **メモリツリー（Memory Tree）** | プロジェクトの事実、設計決定、デバッグ記録、ユーザーの習慣 | `recall`（ベクトル + BM25 を RRF で統合）と `add`（分岐階層用の `--parent` と改訂用の `--supersedes`） |

> **1つのバイナリ、2つのインターフェース：** **メインエージェント**は **MCP**（`cam mcp`）経由で `cam` を呼び出し、MCP ツールを継承しにくい**サブエージェント**は全く同じ **CLI**（`cam --json …`）をシェルから呼び出します。両者はディスク上の `.cam/cam.db` を共有し、100% ローカル、API コストはゼロです。

---

## クイックスタート

### 方法 1: エージェントへの一言指示で自動インストール（推奨）

このプロンプトをコーディングエージェント（**Cursor**、**Claude Code**、**Codex**、**Windsurf**、**GitHub Copilot**、**Cline** など）に直接貼り付けるだけで、`cam` のインストール、インデックス作成、MCP 設定を自動で完了させます：

```text
Install cam from https://github.com/HanochZhu/codeagent_memory: require a working Rust toolchain (rustup), run `cargo install --git https://github.com/HanochZhu/codeagent_memory --locked`, ensure ~/.cargo/bin (or %USERPROFILE%\.cargo\bin on Windows) is on PATH, then verify with `cam --help`. In this repo run `cam index`. For the main agent, register MCP stdio: command `cam`, args `["mcp"]` (Cursor: .cursor/mcp.json; Claude Code: .mcp.json). Subagents call `cam --json` in the shell. Do not add extra docs.
```

<details>
<summary><b>各 IDE / エージェントでの使用方法</b></summary>

| 環境 | 使用方法 |
| --- | --- |
| **Cursor / Windsurf / Claude Code / Codex** | Agent / Chat に貼り付けると、ターミナルでビルド・インストールし、`.cam/cam.db` を生成して MCP 設定を書き込みます |
| **VS Code**（Copilot / Continue / Cline） | ターミナル実行権限を有効にした Agent / Chat モードで貼り付けます |
| **JetBrains**（IntelliJ / RustRover / GoLand） | AI Assistant に貼り付けるか、組み込みターミナルで手動実行します |

<sub>注意: `cargo install` は `cam` を `~/.cargo/bin`（Windows: `%USERPROFILE%\.cargo\bin`）に配置します。`PATH` に即座に反映されない場合は、ターミナルまたは IDE を再起動してください。</sub>

</details>

---

### 方法 2: 手動セットアップ

#### 1. CLI のインストール

[Rust ツールチェーン（rustup）](https://rustup.rs/) が必要です：

```bash
cargo install --git https://github.com/HanochZhu/codeagent_memory --locked
cam --help
```

<details>
<summary><b>その他のインストール方法: ローカルクローンからのビルド / Docker</b></summary>

**ローカルクローンからビルド:**

```bash
git clone https://github.com/HanochZhu/codeagent_memory.git
cd codeagent_memory
cargo install --path . --locked
```

**Docker（Rust ツールチェーン不要）:**

```bash
docker pull ghcr.io/hanochzhu/codeagent-memory:0.1.5
docker run -i --rm -v "$PWD:/work" -w /work ghcr.io/hanochzhu/codeagent-memory:0.1.5
```

またはローカルでビルド: `docker build -t cam https://github.com/HanochZhu/codeagent_memory.git`

</details>

#### 2. プロジェクトのインデックス作成とクエリ

```bash
cd your-project
cam index                                              # リポジトリを解析して .cam/cam.db を生成

# コードグラフのクエリ
cam ls src/                                            # ファイル一覧やファイル内シンボル一覧を表示
cam read src/main.rs/main                              # 特定のシンボル本体のみを読み取る
cam ref main --dir in                                  # 1ホップの callers (in) または callees (out) を調査

# メモリツリーのクエリと書き込み
cam recall "how does hybrid recall fuse BM25 and vectors"
cam add --summary "..." < notes.md
```

<sub>`cam` は初回実行時に `.cam/` を自動初期化します。ファイル保存時にリアルタイムで増分更新を行いたい場合は、バックグラウンドで `cam watch` を実行してください。</sub>

#### 3. メインエージェント用 MCP の接続

エディタの MCP 設定（例: `.cursor/mcp.json` や `.mcp.json`）に `cam mcp` を追加します：

```json
{
  "mcpServers": {
    "cam": {
      "command": "cam",
      "args": ["mcp"]
    }
  }
}
```

エディタ別の設定パスは [docs/mcp.md](docs/mcp.md)、メインエージェントとサブエージェントのプロンプト規則は [docs/agents.md](docs/agents.md) を参照してください。

<sub>初回 `recall` または `add` の実行時に、デフォルトの埋め込みモデル（`potion-multilingual-128M` を `~/.cam/models/` にキャッシュ）が自動ダウンロードされます。オフライン環境では語彙ハッシュ埋め込み（hash embedder）に自動フォールバックします。</sub>

---

## 主な機能

| 機能 | 詳細 |
| --- | --- |
| **SQLite コードグラフ** | tree-sitter が関数、型、呼び出しエッジを `.cam/cam.db` に解析し、シンボル単位の瞬時ナビゲーションを実現 |
| **仮想パス指定** | `src/main.rs` はファイルのアウトラインを参照し、`src/main.rs/main` はシンボル本体を直接参照 |
| **リアルタイム増分同期** | `cam sync` がコンテンツハッシュで増分更新。`cam watch` がファイル保存時にグラフを自動更新 |
| **ハイブリッド想起 (RRF)** | ローカルの密ベクトル埋め込み（`model2vec`）と識別子対応 BM25 を相互ランク融合（Reciprocal Rank Fusion, $k=60$）で統合 |
| **改訂チェーンとメモリツリー** | `--parent` で分岐トピック階層を構築。`--supersedes` で線形改訂をリンクし古いエントリを自動折りたたみ |
| **エビングハウスの忘却曲線** | 保持率 $R = e^{-t / S}$ でランクスコアを緩やかに調整（$\le 10\%$）。履歴を削除せず `stale` / `needs_update` のフラグを付与 |
| **デュアルインターフェース (MCP + CLI)** | メインエージェントは `cam_*` MCP ツールを使用、サブエージェントはシェルで `cam --json …` を実行 |
| **100% ローカル＆プライベート** | 外部 API キー不要。ローカルの SQLite とディスク上の埋め込みのみで完全動作 |
| **9言語に対応** | Rust、Python、TypeScript、JavaScript、Go、Java、C、C++、C# |

---

## 動作原理

![cam の動作原理](docs/images/how-it-works.svg)

1. **抽出 (Extract)** — `tree-sitter` がワークスペースを走査し、シンボル定義と呼び出しエッジを `.cam/cam.db` に格納します。
2. **ピンポイント読取 (Surgical Read)** — エージェントは仮想パス（`ls` → `find` → `read` → `ref`）で移動します。ファイルを読み取るとシンボルのアウトラインが返され、`file/symbol` を読み取るとそのシンボルのソーススライスのみが返されます。
3. **ハイブリッド想起 (Hybrid Recall)** — `recall` は英語のクエリを受け取り、ローカルでベクトル化し、識別子対応 BM25（`snake_case`、`kebab-case`、`camelCase` の分割 + 英語ステミング）を実行、低スコアの裾野を刈り取り、両方のリストを **RRF**（$k=60$）と保持率スケーリング $0.9 + 0.1 \times R$ で融合します。置換された古い改訂は自動的に最新版に展開されます。
4. **書き戻し (Write-Back)** — 課題解決後、原子的な事実（atomic facts）として抽出し、1回の `add` につき1つの事実のみを保存します（要約は英語必須、本文は非ラテン文字に対応）。関連する事実は `--link` / `links` でナレッジグラフとして接続します（`mem show` で確認可能。現時点ではグラフ想起は未実装）。`--parent` で構造トピックにアタッチし、`--supersedes` で履歴を残したまま古いメモリを置き換えます。

アーキテクチャ詳細（中国語）: [DESIGN.md](DESIGN.md)。

---

## エージェントのワークフロー

すべてのコーディングタスクは、共通の4ステップのループに従います：

1. **まず想起 (Recall first)** — コードを探索する前に、英語で `cam_recall` / `cam recall` を実行します。ヒットしたエントリが `latest` かつ `needs_update` でなければそのまま再利用します。
2. **インデックスを確認 (Ensure indexed)** — グラフツールが `not_indexed` を返した場合は、プロジェクトに対して `cam_index` / `cam index` を1回実行します。
3. **グラフを辿る (Walk the graph)** — ヒットしなかった場合、`find` または `ls` でシンボルを特定し、`read <ファイル>/<シンボル>` でシンボル本体を読み、`ref <シンボル> --dir in|out` で呼び出し元/先を追跡します。
4. **書き戻す (Write back)** — 発見した新しい設計事実や修正を `cam_add` / `cam add` で原子的事実として永続化（1メモリ1事実、要約は英語、本文は多言語対応）し、関連事実は `links` / `--link` で接続します。

| 役割 | インターフェース | 主な用途 |
| --- | --- | --- |
| **メインエージェント**（Chat / IDE セッション） | **MCP**（`cam mcp`） | `cam_recall`、`cam_find`、`cam_ls`、`cam_read`、`cam_ref`、`cam_add`、`cam_mem_tree`、`cam_mem_show` を直接呼び出し |
| **サブエージェント**（Task / explore / CLI） | **CLI**（`cam --json`） | シェルで `cam --json --project <dir> <cmd>` を実行（サブエージェントは通常 MCP ツールを継承しません） |

エージェント規則とシステムプロンプト全文: [docs/agents.md](docs/agents.md)。

---

## CLI リファレンス

### コードグラフコマンド

| コマンド | 説明 |
| --- | --- |
| `cam index` | tree-sitter でプロジェクトを解析し `.cam/cam.db` に書き込む（`.cam/` が無ければ自動作成） |
| `cam sync` | コンテンツハッシュに基づき変更のあったファイルを増分更新 |
| `cam watch [--debounce-ms N]` | ソースファイルを監視して自動 `sync`（`--json` で同期ごとに JSON を1行出力） |
| `cam ls [virt_path]` | ディレクトリ、ファイル、またはファイル内のシンボル一覧を表示 |
| `cam read <virt_path> [--full]` | ファイルのシンボルアウトライン、指定シンボル本体（`path/symbol`）、または `--full` でファイル全体を読み取る |
| `cam find <query> [--kind K] [--file S] [--scope D] [--limit N]` | シンボル名に対する大文字小文字を区別しない部分一致検索（完全一致が最優先、次いで前方一致） |
| `cam ref <symbol> --dir in\|out [--file S] [--kind K] [--scope D]` | 1ホップの呼び出し元（`in` / `--callers`）または呼び出し先（`out` / `--callees`）。曖昧な名前の場合はランク付けされた `candidates` を返却 |

### メモリコマンド

| コマンド | 説明 |
| --- | --- |
| `cam recall "<query>" [--limit N] [--no-expand] [--include-superseded] [--snippets]` | ハイブリッド RRF 想起（英語クエリ）。`--no-expand` は最新版への自動展開を無効化、`--include-superseded` は旧改訂を保持、`--snippets` は1位以外の長い本文を省略 |
| `cam add --summary "..." [--parent ID] [--supersedes ID] [--link ID]... [--body TEXT \| --file PATH]` | 1つの原子的事実をメモリとして保存（要約は英語必須、本文は非ラテン文字対応。`--body`、`--file`、または `stdin` 経由）。`--link` でナレッジグラフ上の関連事実を接続、`--parent` で階層トピックに配置、`--supersedes` で古いメモリを置き換え |
| `cam mem tree` | メモリツリーの階層構造と改訂チェーンを表示 |
| `cam mem show <id>` | 単一のメモリエントリの全文とメタデータを確認 |

### サーバーおよび設定コマンド

| コマンド | 説明 |
| --- | --- |
| `cam mcp` | メインエージェント用 stdio MCP サーバーを起動（[docs/mcp.md](docs/mcp.md) 参照） |
| `cam status` | 解決されたプロジェクトルート、DB パス、ノード/エッジ/メモリの統計、現在の設定を表示 |
| `cam config get` | 現在の `~/.cam/config.toml` 設定を表示 |
| `cam config set stale_days <N>` | メモリを `stale` と判定する経過日数しきい値を設定 |
| `cam config set embedding_model <model>` | Hugging Face model2vec リポジトリ、ローカルモデルディレクトリ、`hash`、または `default` を設定 |

**グローバルフラグと規約:**
- `--project <dir>`（または環境変数 `CAM_PROJECT`）で、上位ディレクトリへの `.cam` / `.git` 自動探索を上書き。
- `--json` はコンパクトな JSON を出力（`--pretty` でインデント付き）。`--json` でのエラー時は `stdout` に `{"error":{"code":…,"message":…}}` を出力し非ゼロで終了。
- **同名シンボルの解消:** `read` や `ref` で複数の同名定義に一致した場合、`status: "ambiguous"` と候補ノード ID（例: `src/main.rs:function:main:12`）を返し、その ID をそのまま `read` や `ref` に再入力可能。
- **無視ルールとマルチリポジトリ:** 任意の階層で `.gitignore` および `.camignore` に従います。git submodule や linked worktree はスキップされ、ネストされた独立クローンは `kind: "project"` としてインデックスされます（`--scope` に指定可能）。

---

## メモリ規則

- **原子的事実とナレッジグラフリンク:** 1つのメモリに情報を詰め込みすぎないよう、1エントリにつき1つの原子的事実のみを保存します。複数の事実に関連がある場合は `--link` / `links` でリンクしてナレッジグラフを構築します（グラフ検索は現在未実装）。
- **英語のクエリと要約:** `recall` のクエリおよびメモリの要約（summary）は英語で記述する必要があります（語幹処理や要約インデックスの一貫性を保証するため）。メモリ本文（body）は日本語などの非ラテン文字に対応しています。
- **忘却曲線と強化:** エビングハウス保持率 $R = e^{-t / S}$ により、スコアは $\text{relevance} \times (0.9 + 0.1 \times R)$ で調整されます。$R < 0.3$ のエントリは `needs_update`、`stale_days`（デフォルト30日）を超えたエントリは `stale` と判定されます。有効なヒット（`needs_update = false`）があるたびにアクセス日時が更新され、安定度 $S$ が 1.7 倍に強化されます。
- **非破壊的な履歴管理:** メモリは削除されません。`--parent` は分岐ツリーを形成し、`--supersedes` は線形改訂チェーンを形成します。改訂チェーンでは末尾のみが `latest` となり、古い改訂はデフォルトの検索結果から自動的に折りたたまれます。

---

## 設定

グローバル設定は `~/.cam/config.toml` に保存されます：

```toml
stale_days = 30
# embedding_model = "minishlab/potion-multilingual-128M" # デフォルト（~/.cam/models/ にキャッシュ）
# embedding_model = "minishlab/potion-base-8M"           # より軽量な Hugging Face model2vec リポジトリ
# embedding_model = "/path/to/local/model2vec"           # tokenizer.json, config.json, model.safetensors を含むローカルディレクトリ
# embedding_model = "hash"                               # モデルダウンロード不要の語彙ハッシュ埋め込み
```

環境変数 `CAM_EMBED_MODEL` を使って単一プロセスでのモデルを上書きすることや、`cam config set embedding_model default` でデフォルトにリセットすることが可能です。モデルを変更した場合、次回の `recall` 実行時に既存メモリが自動で再埋め込みされてからスコアリングされます。

---

## ベンチマーク

`cam` は、コードグラフとメモリプレーンを**同一のコーパス上**で最も近いオープンソースシステムと個別に評価しています：

| プレーン | 比較ベースライン | ベンチマークコーパス | 主な結果 |
| --- | --- | --- | --- |
| **メモリ** | [agentmemory](https://github.com/rohitg00/agentmemory) hybrid + トークン化 `grep` | [coding-agent-life-v1](eval/coding_life/README.md) | **1.000 R@5 / 0.240 P@5**（ハイブリッド上限に一致、改訂チェーンで **1.000 newest R@5**） |
| **長期記憶** | BM25 / 密ベクトル検索 | [LongMemEval-S](https://github.com/xiaowu0162/LongMemEval) | `potion-multilingual-128M` で **0.909 R@5 / 0.980 Hit@5 / 0.840 MRR** |
| **コードグラフ** | [codegraph](https://github.com/colbymchenry/codegraph) v0.9.6 | [LongMemCode](eval/longmemcode/README.md) (`clap`) | サポート対象の1ホップタスクで **0.769** vs 0.768、P95 ≈ 5.2 ms、コスト `$0` |
| **ワークフロー検索** | 語彙 / BM25 ファイル検索 | [Agent Retrieval Bench V2](https://agent-retrieval-bench.github.io/) `edit2ripple` | **0.297 MRR**（最良のファーストヒットランク、P95 7.3 ms） |
| **マルチターン Token** | 全文コンテキストダンプ | [DeepSeek マルチターン](eval/llm_multiturn/README.md) | メモリトラックで **57% のトークン削減**、コードトラックで **98% のトークン削減** |

<sub>一般的なチャットメモリフレームワーク（Mem0、Zep、Letta）はコーディングエージェント向けコーパスではなく会話ログを対象としているため、以下の比較表からは除外しています。</sub>

### システム比較

| | **cam** | **agentmemory** | **codegraph** |
| --- | --- | --- | --- |
| **格納内容** | コードグラフ + メモリツリー | セッション / チャットメモリ | コードグラフ |
| **クエリプリミティブ** | `recall` / `ls` / `find` / `read` / `ref` | `smart-search` / `remember` | `explore` / callers / callees |
| **インターフェース** | **MCP + CLI** | MCP + REST + hooks | MCP + CLI |
| **ランキング / 統合** | ベクトル + BM25 + **RRF** + 保持率 | BM25 + 埋め込み + rerank | FTS5 + グラフ探索 |
| **コスト** | 100% ローカル（`$0`） | ローカルサーバー | ローカル |

### 1. メモリ — `coding-agent-life-v1`

15セッション、15クエリ、$k=5$（`agentmemory` の `score.ts` と同一の採点方式。P@5 の理論上限は `0.240`）。

| システム | ヒット率 | R@5 | P@5 | 出典 |
| --- | --- | ---: | ---: | --- |
| `grep`（トークン化部分文字列） | 15 / 15 | 0.967 | 0.227 | 本リポジトリ、2026-09-23 |
| `cam` min-max 和（旧版） | 15 / 15 | 0.933 | 0.213 | 過去ログ、hash embed |
| **`cam` RRF** | **15 / 15** | **1.000** | **0.240** | 本リポジトリ、hash embed |
| `agentmemory` hybrid | 15 / 15 | 1.000 | 0.240 | 公開版 v0.9.26 |

![coding-agent-life headline R@5 and P@5 / ceiling](eval/charts/solution-headline.svg)

![coding-agent-life R@5 by question type](eval/charts/solution-recall.svg)

**改訂チェーンテスト（`--lineage`）：** 各ターンを `--supersedes` 改訂として順次投入し、上位 $k$ 件が各正解チェーンの最新ノードを返すか（**newest R@k**）、および古いノードが誤って `latest` を主張していないか（**stale `latest`**）を検証。

| k | R@k | P@k | newest R@k | stale `latest` |
| ---: | ---: | ---: | ---: | ---: |
| 5 | 1.000 | 0.240 | **1.000** | **0** |
| 10 | 1.000 | 0.120 | **1.000** | **0** |

### 2. 長期記憶 — `LongMemEval-S`

クリーンアップされた `LongMemEval-S` からシード 42 でサンプリングされた 100 問（拒絶応答なし）。LLM リーダーなしで $k=10$ の RRF により埋め込みと BM25 を融合。

| 埋め込み器 | R@1 | R@5 | R@10 | Hit@1 | Hit@5 | Hit@10 | MRR | 検索遅延 P50 / P95 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `hash`（オフラインテスト） | 0.382 | 0.803 | 0.922 | 0.580 | 0.910 | 0.970 | 0.718 | 24.1 / 35.5 ms |
| **`potion-multilingual-128M`** | **0.474** | **0.909** | **0.952** | **0.740** | **0.980** | **0.990** | **0.840** | 1.12 / 1.34 s |

### 3. ワークフロー検索 — Agent Retrieval Bench V2 (`edit2ripple`)

44 の固定リポジトリスナップショットにわたる 58 の変更アンカークエリ（LLM なし。BCY = 標準的な 8K トークンファイルパッキングプロトコルにおける予算内コンテキスト歩留まり）。

| システム | Recall@5 | Recall@10 | Recall@20 | MRR | BCY@8k |
| --- | ---: | ---: | ---: | ---: | ---: |
| **`cam` グラフ** | 0.300 | 0.335 | 0.378 | **0.297** | 0.300 |
| 語彙検索 | **0.412** | **0.536** | **0.588** | 0.243 | **0.447** |
| BM25 | 0.207 | 0.287 | 0.499 | 0.154 | 0.243 |

`cam` グラフは最高のファーストヒットランクを達成し（MRR `0.297`、クエリ P95 `7.3 ms`）、語彙検索は複数ファイルへのより広い網羅性を提供します — これがグラフ検索と語彙検索のランク融合を動機付けています。

### 4. コードグラフ — LongMemCode (`clap`)

`clap` v4.6.1、SCIP gold に対する 536 のシナリオ（LLM なし、P95 ≈ 5.2 ms、コスト `$0`）。

| スライス | n | `cam` | `codegraph` 0.9.6 |
| --- | ---: | ---: | ---: |
| **サポート対象（1ホップ）** | 478 | **0.769** | 0.768 |
| `lookup` / `file_symbols` | 293 / 55 | 0.808 / 0.915 | 0.808 / 0.915 |
| `callers` / `callees` | 67 / 18 | **0.397** / 0.811 | 0.355 / **0.867** |
| `implementors` | 40 | **0.119** | 0.094 |
| 生値 / 重み付け平均（全536件） | 536 | **0.709** / **0.704** | 0.704 / 0.700 |

![LongMemCode clap accuracy bars](eval/charts/code-graph-bars.svg)

![LongMemCode clap accuracy by operation](eval/charts/code-graph.svg)

### 5. マルチターントークン削減 — DeepSeek

全コンテキストダンプとターンごとの `cam` 検索の比較（`deepseek-flash`、hash embedder、2026-10-13）：

| トラック | n | フルコンテキスト正解率 | `cam` 正解率 | フルコンテキストトークン | `cam` トークン | トークン削減率 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| **メモリ**（`coding-agent-life`） | 15 | 1.00 | 0.87 | 22,796 | 9,904 | **57%**（1,471 → 616 / ターン） |
| **コード**（本リポジトリをインデックス） | 6 | 0.83 | 0.33 | 379,066 | 6,342 | **98%**（63,076 → 1,000 / ターン） |

![DeepSeek multi-turn prompt token bars](eval/charts/multiturn-bars.svg)

![DeepSeek multi-turn prompt tokens](eval/charts/multiturn-tokens.svg)

<details>
<summary><b>すべてのベンチマークを再現する手順</b></summary>

データセットのパスは [eval/datasets.toml](eval/datasets.toml) で設定されています（`coding-agent-life-v1` はデフォルトで `../agentmemory` または `CAM_EVAL_CODING_LIFE` を参照します）。

```bash
python3 eval/coding_life/run.py --adapter grep
python3 eval/coding_life/run.py                                             # potion; モデル読み込み失敗時はエラー
python3 eval/coding_life/run.py --hash-embed                                # hash embedder
python3 eval/longmemeval/run.py --sample 100 --seed 42 --workers 8          # potion
python3 eval/longmemeval/run.py --sample 100 --seed 42 --workers 8 --hash-embed
python3 eval/locomo/run.py --workers 4                                      # potion, エビデンス想起

git clone --depth 1 https://github.com/eyuansu62/agent-retrieval-bench \
  eval/agent_retrieval/vendor/agent-retrieval-bench
python3 -m pip install -e eval/agent_retrieval/vendor/agent-retrieval-bench
arb download-benchmark --version v2_edit2ripple --local-dir eval/agent_retrieval/data --force
python3 eval/agent_retrieval/run.py --workers 8

python3 eval/llm_multiturn/run.py --track both   # potion; DEEPSEEK_API_KEY が必要
python3 eval/longmemcode/run.py --corpus clap
python3 eval/charts/generate.py
```

</details>

---

## 対応エージェント

**メインエージェント**には `cam mcp`（stdio）を登録し、**サブエージェント**は追加設定なしでシェルから直接 `cam --json` CLI を使用します。

| ホスト | MCP 設定場所 | サブエージェント / 備考 |
| --- | --- | --- |
| **Cursor** | `.cursor/mcp.json`、`~/.cursor/mcp.json`、または `.cursor-plugin/plugin.json` | `Task` / `explore` サブエージェントは CLI を使用 |
| **Claude Code** | `.mcp.json`、`~/.claude.json`、または `claude mcp add cam -- cam mcp` | 制限付きツールのサブエージェントは CLI を使用 |
| **Codex** | `~/.codex/config.toml` → `[mcp_servers.cam]` | `codex exec` は CLI を使用 |
| **Windsurf** | `~/.codeium/windsurf/mcp_config.json` | Cascade は MCP を使用 |
| **GitHub Copilot** | `.vscode/mcp.json` または `~/.copilot/mcp-config.json` | Chat は MCP を使用、CLI スクリプトは CLI を使用 |
| **Continue / Cline** | `~/.continue/config.yaml` / `~/.cline/mcp.json` | |
| **JetBrains** | AI Assistant → MCP（`cam` / `mcp`） | IDE の `PATH` が空の場合は `cam` の絶対パスを指定 |
| **Gemini CLI / Antigravity / OpenCode / Zed / Droid** | [docs/mcp.md](docs/mcp.md) 参照 | |

コピペ用設定テンプレート: [docs/mcp.md](docs/mcp.md) · システムプロンプト: [docs/agents.md](docs/agents.md)。

---

## 対応言語

| 言語 | 拡張子 | インデックスされるノードとエッジ |
| --- | --- | --- |
| **Rust** | `.rs` | 関数、struct、enum、trait、呼び出し |
| **Python** | `.py` | 関数、クラス、呼び出し |
| **TypeScript** | `.ts`, `.tsx` | 関数、メソッド、クラス、呼び出し |
| **JavaScript** | `.js`, `.jsx` | 関数、メソッド、クラス、呼び出し |
| **Go** | `.go` | 関数、メソッド、struct、呼び出し |
| **Java** | `.java` | 関数、メソッド、クラス、インターフェース、呼び出し |
| **C** | `.c` | 関数、struct、呼び出し |
| **C++** | `.cc`, `.cpp`, `.cxx`, `.h`, `.hpp` | 関数、メソッド、クラス、struct、呼び出し |
| **C#** | `.cs` | 関数、メソッド、クラス、インターフェース、呼び出し |

---

## 対応プラットフォーム

`cam` は **macOS**、**Linux**、**Windows** 上で `cargo install --git https://github.com/HanochZhu/codeagent_memory --locked` によりネイティブにコンパイル・インストール可能で、Docker イメージ（`ghcr.io/hanochzhu/codeagent-memory:0.1.5`）を介してどこでも実行できます。[クイックスタート](#クイックスタート) を参照してください。

---

## ライセンス

MIT

---

<div align="center">

**AI コーディングエージェントのための設計 — Claude Code、Cursor、Codex、Windsurf、Copilot、JetBrains**

[バグ報告](https://github.com/HanochZhu/codeagent_memory/issues) · [機能リクエスト](https://github.com/HanochZhu/codeagent_memory/issues)

</div>
