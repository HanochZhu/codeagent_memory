<div align="center">

<img src="assets/logo.svg" alt="cam logo" width="128" height="128">

# cam

[English](README.md) · [中文](README.zh.md) · [日本語](README.ja.md) · [한국어](README.ko.md) · [العربية](README.ar.md) · [🌐 웹사이트](https://hanochzhu.github.io/cam-web/)

### 코딩 에이전트를 위해 특화된 로컬 멀티턴 대화형 메모리 시스템

**코드 그래프 + 메모리 · 심볼 단위 정밀 읽기 · 100% 로컬 · MCP + CLI**

**Rust로 작성된 코어**

[![Website](https://img.shields.io/badge/website-라이브-brightgreen.svg)](https://hanochzhu.github.io/cam-web/)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/Rust-edition%202021-orange.svg)](https://www.rust-lang.org/)
[![Install](https://img.shields.io/badge/install-cargo--git-blue.svg)](#시작하기)

[![Windows](https://img.shields.io/badge/Windows-supported-blue.svg)](#지원하는-플랫폼)
[![macOS](https://img.shields.io/badge/macOS-supported-blue.svg)](#지원하는-플랫폼)
[![Linux](https://img.shields.io/badge/Linux-supported-blue.svg)](#지원하는-플랫폼)

[![Claude Code](https://img.shields.io/badge/Claude_Code-supported-blueviolet.svg)](#지원하는-에이전트)
[![Cursor](https://img.shields.io/badge/Cursor-supported-blueviolet.svg)](#지원하는-에이전트)
[![Codex](https://img.shields.io/badge/Codex-supported-blueviolet.svg)](#지원하는-에이전트)
[![Windsurf](https://img.shields.io/badge/Windsurf-supported-blueviolet.svg)](#지원하는-에이전트)
[![GitHub Copilot](https://img.shields.io/badge/GitHub_Copilot-supported-blueviolet.svg)](#지원하는-에이전트)
[![Continue](https://img.shields.io/badge/Continue-supported-blueviolet.svg)](#지원하는-에이전트)
[![Cline](https://img.shields.io/badge/Cline-supported-blueviolet.svg)](#지원하는-에이전트)
[![JetBrains](https://img.shields.io/badge/JetBrains-supported-blueviolet.svg)](#지원하는-에이전트)

</div>

## 목차

- [왜 cam인가?](#왜-cam인가)
- [시작하기](#시작하기)
- [주요 기능](#주요-기능)
- [동작 원리](#동작-원리)
- [에이전트 워크플로우](#에이전트-워크플로우)
- [CLI 레퍼런스](#cli-레퍼런스)
- [메모리 규칙](#메모리-규칙)
- [설정](#설정)
- [벤치마크](#벤치마크)
- [지원하는 에이전트](#지원하는-에이전트)
- [지원하는 언어](#지원하는-언어)
- [지원하는 플랫폼](#지원하는-플랫폼)
- [라이선스](#라이선스)

가이드: [인터랙티브 웹사이트](https://hanochzhu.github.io/cam-web/) · [메인 에이전트 vs 서브에이전트](docs/agents.md) · [IDE별 MCP 설정](docs/mcp.md) · [中文说明](docs/agents.zh.md)

---

## 왜 cam인가?

AI 코딩 에이전트가 코드베이스를 탐색할 때 보통 `grep`, `glob`, 전체 파일 `Read`를 하나씩 수행하며 구조를 느리게 재탐색하고, 다음 세션이 되면 모든 것을 잊어버립니다.

**`cam`은 단일 바이너리로 에이전트에게 두 가지 로컬 검색 플레인을 제공합니다:**

| 플레인 | 저장 내용 | 에이전트 활용 방식 |
| --- | --- | --- |
| **코드 그래프 (Code Graph)** | tree-sitter 심볼(함수, 타입, 메서드)을 가상 파일시스템으로 구성 + 1-hop callers/callees | `ls`, `find`, `read src/file.rs/symbol`, `ref symbol --dir in\|out` — 전체 파일을 덤프하는 대신 심볼 단위로 정밀하게 읽기 |
| **메모리 트리 (Memory Tree)** | 프로젝트 팩트, 아키텍처 결정, 디버깅 기록, 사용자 습관 | `recall`(벡터 + BM25를 RRF로 융합) 및 `add`(`--parent` 브랜치 계층 구조 및 `--supersedes` 개정 버전 관리) |

> **하나의 바이너리, 두 개의 인터페이스:** **메인 에이전트**는 **MCP**(`cam mcp`)를 통해 도구를 직접 호출하고, MCP 도구를 상속받기 어려운 **서브에이전트**는 셸에서 동일한 **CLI**(`cam --json …`)를 호출합니다. 둘 다 디스크의 `.cam/cam.db`를 공유하며 100% 로컬로 작동하고 API 비용이 전혀 들지 않습니다.

---

## 시작하기

### 옵션 1: 에이전트에 한 줄 프롬프트로 자동 설치 (권장)

이 프롬프트를 코딩 에이전트(**Cursor**, **Claude Code**, **Codex**, **Windsurf**, **GitHub Copilot**, **Cline** 등)에 직접 붙여넣으면 `cam`의 설치, 인덱싱, 설정이 자동으로 완료됩니다:

```text
Install cam from https://github.com/HanochZhu/codeagent_memory: require a working Rust toolchain (rustup), run `cargo install --git https://github.com/HanochZhu/codeagent_memory --locked`, ensure ~/.cargo/bin (or %USERPROFILE%\.cargo\bin on Windows) is on PATH, then verify with `cam --help`. In this repo run `cam index`. For the main agent, register MCP stdio: command `cam`, args `["mcp"]` (Cursor: .cursor/mcp.json; Claude Code: .mcp.json). Subagents call `cam --json` in the shell. Do not add extra docs.
```

<details>
<summary><b>각 IDE / 에이전트에서의 활용 방법</b></summary>

| 환경 | 사용 방법 |
| --- | --- |
| **Cursor / Windsurf / Claude Code / Codex** | Agent / Chat에 붙여넣으면 터미널에서 빌드·설치하고, `.cam/cam.db`를 생성하며 MCP 설정을 작성합니다 |
| **VS Code** (Copilot / Continue / Cline) | 터미널 실행 권한이 활성화된 Agent / Chat 모드에 붙여넣습니다 |
| **JetBrains** (IntelliJ / RustRover / GoLand) | AI Assistant에 붙여넣거나 내장 터미널에서 직접 실행합니다 |

<sub>참고: `cargo install`은 `cam`을 `~/.cargo/bin`(Windows: `%USERPROFILE%\.cargo\bin`)에 설치합니다. 터미널의 `PATH`에서 즉시 인식되지 않으면 터미널이나 IDE를 재시작하세요.</sub>

</details>

---

### 옵션 2: 수동 설치 및 설정

#### 1. CLI 설치

[Rust 툴체인(rustup)](https://rustup.rs/)이 필요합니다:

```bash
cargo install --git https://github.com/HanochZhu/codeagent_memory --locked
cam --help
```

<details>
<summary><b>기타 설치 방법: 로컬 클론 빌드 / Docker</b></summary>

**로컬 클론에서 빌드:**

```bash
git clone https://github.com/HanochZhu/codeagent_memory.git
cd codeagent_memory
cargo install --path . --locked
```

**Docker (Rust 툴체인 불필요):**

```bash
docker pull ghcr.io/hanochzhu/codeagent-memory:0.1.5
docker run -i --rm -v "$PWD:/work" -w /work ghcr.io/hanochzhu/codeagent-memory:0.1.5
```

또는 로컬에서 빌드: `docker build -t cam https://github.com/HanochZhu/codeagent_memory.git`

</details>

#### 2. 프로젝트 인덱싱 및 쿼리

```bash
cd your-project
cam index                                              # 저장소를 파싱하여 .cam/cam.db 생성

# 코드 그래프 조회
cam ls src/                                            # 디렉토리, 파일 또는 파일 내 심볼 목록 조회
cam read src/main.rs/main                              # 특정 심볼 본문만 정밀하게 읽기
cam ref main --dir in                                  # 1-hop 호출자 (in) 또는 피호출자 (out) 추적

# 메모리 트리 조회 및 기록
cam recall "how does hybrid recall fuse BM25 and vectors"
cam add --summary "..." < notes.md
```

<sub>`cam`은 처음 실행할 때 `.cam/`을 자동으로 초기화합니다. 파일 변경 시 실시간 증분 업데이트를 원하면 백그라운드에서 `cam watch`를 실행하세요.</sub>

#### 3. 메인 에이전트 MCP 연결

에디터의 MCP 설정 파일(예: `.cursor/mcp.json` 또는 `.mcp.json`)에 `cam mcp`를 추가합니다:

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

에디터별 설정 경로는 [docs/mcp.md](docs/mcp.md), 메인 에이전트와 서브에이전트 프롬프트 규칙은 [docs/agents.md](docs/agents.md)를 참조하세요.

<sub>첫 `recall` 또는 `add` 실행 시 기본 임베딩 모델(`potion-multilingual-128M`, `~/.cam/models/`에 캐시됨)이 자동으로 다운로드되며, 오프라인 환경에서는 어휘 해시 임베더(hash embedder)로 자동 폴백됩니다.</sub>

---

## 주요 기능

| 기능 | 설명 |
| --- | --- |
| **SQLite 코드 그래프** | tree-sitter가 함수, 타입, 호출 관계를 `.cam/cam.db`로 파싱하여 심볼 단위의 즉각적인 탐색 제공 |
| **가상 경로 지정** | `src/main.rs`는 파일 개요를 가리키고, `src/main.rs/main`은 심볼 본문을 직접 참조 |
| **실시간 증분 동기화** | `cam sync`가 콘텐츠 해시 기반으로 증분 업데이트. `cam watch`가 파일 저장 시 그래프를 자동 갱신 |
| **하이브리드 리콜 (RRF)** | 로컬 밀집 벡터 임베딩(`model2vec`)과 식별자 기반 BM25를 상호 순위 융합(Reciprocal Rank Fusion, $k=60$)으로 결합 |
| **개정 체인 및 메모리 트리** | `--parent`로 계층형 주제 트리를 생성하고, `--supersedes`로 선형 개정을 연결하여 이전 버전을 자동 폴딩 |
| **에빙하우스 망각 곡선** | 보존율 $R = e^{-t / S}$를 통해 랭킹 점수를 미세 조정($\le 10\%$)하며, 히스토리를 삭제하지 않고 `stale` / `needs_update` 플래그 표시 |
| **듀얼 인터페이스 (MCP + CLI)** | 메인 에이전트는 `cam_*` MCP 도구를 사용하고, 서브에이전트는 셸에서 `cam --json …` 실행 |
| **100% 로컬 및 개인정보 보호** | 외부 API 키 불필요. 로컬 SQLite와 디스크 임베딩으로 완전 독립 구동 |
| **9개 언어 지원** | Rust, Python, TypeScript, JavaScript, Go, Java, C, C++, C# |

---

## 동작 원리

![cam 동작 원리](docs/images/how-it-works.svg)

1. **추출 (Extract)** — `tree-sitter`가 워크스페이스를 순회하며 심볼 정의와 호출 관계를 `.cam/cam.db`에 저장합니다.
2. **정밀 읽기 (Surgical Read)** — 에이전트는 가상 경로(`ls` → `find` → `read` → `ref`)를 통해 탐색합니다. 파일 읽기는 심볼 개요를 반환하고, `file/symbol` 읽기는 해당 심볼의 소스 코드 슬라이스만 반환합니다.
3. **하이브리드 리콜 (Hybrid Recall)** — `recall`은 영어 질의를 받아 로컬에서 벡터화하고, 식별자 인식 BM25(`snake_case`, `kebab-case`, `camelCase` 분리 + 영어 어간 추출)를 실행하여 저득점 꼬리를 잘라낸 뒤, 두 결과를 **RRF**($k=60$) 및 보존율 계수 $0.9 + 0.1 \times R$로 융합합니다. 대체된 이전 개정 버전은 자동으로 최신 대체 버전으로 확장됩니다.
4. **다시 쓰기 (Write-Back)** — 과제를 해결한 후 원자적 사실(atomic facts) 단위로 추출하여 각 `add`마다 하나의 사실만 저장합니다(요약은 영어 필수, 본문은 다국어 지원). 연관된 사실들은 `--link` / `links`로 지식 그래프로 연결합니다(`mem show`로 확인 가능하며 그래프 기반 검색은 추후 지원 예정). `--parent`는 구조적 주제에 연결하고, `--supersedes`는 이력을 보존하면서 이전 메모리를 대체합니다.

아키텍처 상세 문서 (중국어): [DESIGN.md](DESIGN.md).

---

## 에이전트 워크플로우

모든 코딩 작업은 일관된 4단계 루프를 따릅니다:

1. **리콜 먼저 (Recall first)** — 코드를 탐색하기 전에 영어로 `cam_recall` / `cam recall`을 호출합니다. 검색된 항목이 `latest`이며 `needs_update`가 아니면 즉시 재사용합니다.
2. **인덱싱 확인 (Ensure indexed)** — 그래프 도구가 `not_indexed`를 반환하면 프로젝트 루트에서 `cam_index` / `cam index`를 한 번 실행합니다.
3. **그래프 탐색 (Walk the graph)** — 메모리에 없을 경우 `find`나 `ls`로 심볼을 찾고, `read <파일>/<심볼>`로 본문을 읽으며, `ref <심볼> --dir in|out`으로 호출 관계를 추적합니다.
4. **다시 기록 (Write back)** — 발견한 새로운 아키텍처 사실이나 해결책을 `cam_add` / `cam add`를 통해 원자적 사실 단위로 영구 저장(한 메모리당 한 사실, 요약은 영어, 본문은 다국어 지원)하고, 관련 사실은 `links` / `--link`로 연결합니다.

| 역할 | 인터페이스 | 주요 사용처 |
| --- | --- | --- |
| **메인 에이전트** (Chat / IDE 세션) | **MCP** (`cam mcp`) | `cam_recall`, `cam_find`, `cam_ls`, `cam_read`, `cam_ref`, `cam_add`, `cam_mem_tree`, `cam_mem_show` 직접 호출 |
| **서브에이전트** (Task / explore / CLI) | **CLI** (`cam --json`) | 셸에서 `cam --json --project <dir> <cmd>` 실행 (서브에이전트는 보통 MCP 도구를 전달받지 못함) |

에이전트 규칙 및 시스템 프롬프트 전문: [docs/agents.md](docs/agents.md).

---

## CLI 레퍼런스

### 코드 그래프 명령

| 명령 | 설명 |
| --- | --- |
| `cam index` | tree-sitter로 프로젝트를 파싱하여 `.cam/cam.db`에 저장 (`.cam/`이 없으면 자동 생성) |
| `cam sync` | 파일 내용 해시를 비교하여 변경된 파일만 증분 갱신 |
| `cam watch [--debounce-ms N]` | 소스 파일을 감시하여 자동 `sync` (`--json` 모드에서는 동기화마다 JSON 1줄 출력) |
| `cam ls [virt_path]` | 디렉토리, 파일 또는 파일 내 심볼 목록 조회 |
| `cam read <virt_path> [--full]` | 파일 심볼 개요, 특정 심볼 본문(`path/symbol`) 또는 `--full`로 파일 전체 읽기 |
| `cam find <query> [--kind K] [--file S] [--scope D] [--limit N]` | 심볼명에 대한 대소문자 무시 부분 문자열 검색 (정확한 일치 우선, 그 다음 접두사 일치) |
| `cam ref <symbol> --dir in\|out [--file S] [--kind K] [--scope D]` | 1-hop 호출자(`in` / `--callers`) 또는 피호출자(`out` / `--callees`). 중복 이름 시 정렬된 `candidates` 반환 |

### 메모리 명령

| 명령 | 설명 |
| --- | --- |
| `cam recall "<query>" [--limit N] [--no-expand] [--include-superseded] [--snippets]` | 하이브리드 RRF 리콜 (영어 질의). `--no-expand`는 최신 버전 자동 확장 비활성화, `--include-superseded`는 이전 개정 보존, `--snippets`는 1위 외 긴 본문 축약 |
| `cam add --summary "..." [--parent ID] [--supersedes ID] [--link ID]... [--body TEXT \| --file PATH]` | 단일 원자적 사실 메모리 저장 (요약은 영어 필수, 본문은 다국어 지원. `--body`, `--file` 또는 `stdin` 지원). `--link`로 지식 그래프 연결, `--parent`로 계층 구조 트리 구성, `--supersedes`로 이전 메모리 대체 |
| `cam mem tree` | 메모리 계층 구조와 개정 체인 출력 |
| `cam mem show <id>` | 단일 메모리 항목의 전체 내용 및 메타데이터 조회 |

### 서버 및 설정 명령

| 명령 | 설명 |
| --- | --- |
| `cam mcp` | 메인 에이전트용 stdio MCP 서버 시작 ([docs/mcp.md](docs/mcp.md) 참조) |
| `cam status` | 식별된 프로젝트 루트, DB 경로, 노드/엣지/메모리 통계 및 현재 설정 표시 |
| `cam config get` | 현재 `~/.cam/config.toml` 설정 출력 |
| `cam config set stale_days <N>` | 메모리를 `stale`로 표시할 경과 일수 임계값 설정 |
| `cam config set embedding_model <model>` | Hugging Face model2vec 저장소, 로컬 모델 디렉토리, `hash` 또는 `default` 설정 |

**전역 플래그 및 규칙:**
- `--project <dir>` (또는 환경변수 `CAM_PROJECT`)으로 상위 디렉토리 `.cam` / `.git` 자동 탐색을 재정의.
- `--json`은 압축된 JSON 출력 (`--pretty`로 들여쓰기). `--json` 모드에서 에러 발생 시 `stdout`으로 `{"error":{"code":…,"message":…}}`를 출력하고 0이 아닌 코드로 종료.
- **동명 심볼 모호성 해결:** `read`나 `ref`가 여러 정의와 일치하면 `status: "ambiguous"`와 후보 노드 ID(예: `src/main.rs:function:main:12`)를 반환하며, 이 ID를 그대로 다시 전달 가능.
- **무시 규칙 및 멀티 리포지토리:** 모든 깊이에서 `.gitignore`와 `.camignore`를 준수. git 서브모듈과 링크된 워크트리는 건너뛰며, 중첩된 독립 클론은 `kind: "project"`로 인덱싱됨 (`--scope`에 전달 가능).

---

## 메모리 규칙

- **원자적 사실 및 지식 그래프 링크:** 단일 항목에 과도한 정보가 들어가지 않도록 메모리 하나당 하나의 원자적 사실만 저장합니다. 여러 사실 간에 연관성이 있는 경우 `--link` / `links`로 연결하여 지식 그래프를 구성합니다 (그래프 기반 검색은 현재 미지원).
- **영어 질의 및 요약:** `recall` 질의와 `add` 요약(summary)은 반드시 영어로 작성해야 합니다 (어간 추출 및 요약 색인의 일관성 유지). 메모리 본문(body)은 한국어를 포함한 비라틴 문자를 완벽히 지원합니다.
- **에빙하우스 보존 및 강화:** 보존율 $R = e^{-t / S}$에 따라 리콜 점수가 $\text{relevance} \times (0.9 + 0.1 \times R)$로 조정됩니다. $R < 0.3$인 항목은 `needs_update`로 표시되며, `stale_days`(기본값 30일)를 초과한 항목은 `stale`로 표시됩니다. 유효한 히트(`needs_update = false`)가 발생할 때마다 접근 시간이 갱신되고 안정성 $S$가 1.7배 증가합니다.
- **비파괴적 이력 보존:** 메모리는 삭제되지 않습니다. `--parent`는 브랜치 트리를 형성하고, `--supersedes`는 선형 개정 체인을 형성합니다. 개정 체인에서는 가장 마지막 노드만 `latest`로 표시되며, 대체된 이전 버전은 기본 검색 결과에서 자동으로 접혀 숨겨집니다.

---

## 설정

전역 설정 파일은 `~/.cam/config.toml`에 위치합니다:

```toml
stale_days = 30
# embedding_model = "minishlab/potion-multilingual-128M" # 기본값 (~/.cam/models/에 캐시됨)
# embedding_model = "minishlab/potion-base-8M"           # 가벼운 Hugging Face model2vec 저장소
# embedding_model = "/path/to/local/model2vec"           # tokenizer.json, config.json, model.safetensors가 있는 로컬 경로
# embedding_model = "hash"                               # 다운로드 없는 어휘 해시 임베더
```

환경변수 `CAM_EMBED_MODEL`을 통해 프로세스 단위로 모델을 재정의하거나, `cam config set embedding_model default`로 기본값으로 복원할 수 있습니다. 모델을 변경하면 다음 `recall` 실행 시 기존 메모리를 자동으로 다시 임베딩한 후 점수를 계산합니다.

---

## 벤치마크

`cam`은 코드 그래프와 메모리 플레인을 **동일한 코퍼스 상에서** 가장 유사한 오픈소스 시스템과 개별적으로 평가합니다:

| 플레인 | 비교 기준 | 벤치마크 코퍼스 | 주요 결과 |
| --- | --- | --- | --- |
| **메모리** | [agentmemory](https://github.com/rohitg00/agentmemory) hybrid + 토큰화 `grep` | [coding-agent-life-v1](eval/coding_life/README.md) | **1.000 R@5 / 0.240 P@5** (하이브리드 상한선 일치, 개정 체인에서 **1.000 newest R@5**) |
| **장기 메모리** | BM25 / 밀집 벡터 검색 | [LongMemEval-S](https://github.com/xiaowu0162/LongMemEval) | `potion-multilingual-128M` 기준 **0.909 R@5 / 0.980 Hit@5 / 0.840 MRR** |
| **코드 그래프** | [codegraph](https://github.com/colbymchenry/codegraph) v0.9.6 | [LongMemCode](eval/longmemcode/README.md) (`clap`) | 지원되는 1-hop 작업에서 **0.769** vs 0.768, P95 ≈ 5.2 ms, 비용 `$0` |
| **워크플로우 검색** | 어휘 / BM25 파일 검색 | [Agent Retrieval Bench V2](https://agent-retrieval-bench.github.io/) `edit2ripple` | **0.297 MRR** (최고의 첫 히트 순위, P95 7.3 ms) |
| **멀티턴 Token** | 전체 컨텍스트 주입 | [DeepSeek 멀티턴](eval/llm_multiturn/README.md) | 메모리 트랙에서 **57% 토큰 절감**, 코드 트랙에서 **98% 토큰 절감** |

<sub>일반적인 대화형 메모리 프레임워크(Mem0, Zep, Letta)는 코딩 에이전트 코퍼스가 아닌 일반 대화 로그를 대상으로 하므로 아래 비교표에서는 제외되었습니다.</sub>

### 시스템 비교

| | **cam** | **agentmemory** | **codegraph** |
| --- | --- | --- | --- |
| **저장 내용** | 코드 그래프 + 메모리 트리 | 세션 / 대화 메모리 | 코드 그래프 |
| **쿼리 기본 요소** | `recall` / `ls` / `find` / `read` / `ref` | `smart-search` / `remember` | `explore` / callers / callees |
| **인터페이스** | **MCP + CLI** | MCP + REST + hooks | MCP + CLI |
| **순위 및 융합** | 벡터 + BM25 + **RRF** + 보존율 | BM25 + 임베딩 + rerank | FTS5 + 그래프 탐색 |
| **비용** | 100% 로컬 (`$0`) | 로컬 서버 | 로컬 |

### 1. 메모리 — `coding-agent-life-v1`

15개 세션, 15개 쿼리, $k=5$ (`agentmemory`의 `score.ts`와 동일한 채점 방식, P@5 이론적 상한은 `0.240`).

| 시스템 | 적중률 (Hit Rate) | R@5 | P@5 | 출처 |
| --- | --- | ---: | ---: | --- |
| `grep` (토큰화 부분 문자열) | 15 / 15 | 0.967 | 0.227 | 본 저장소, 2026-09-23 |
| `cam` min-max 합 (구버전) | 15 / 15 | 0.933 | 0.213 | 과거 기록, hash embed |
| **`cam` RRF** | **15 / 15** | **1.000** | **0.240** | 본 저장소, hash embed |
| `agentmemory` hybrid | 15 / 15 | 1.000 | 0.240 | 공개 버전 v0.9.26 |

![coding-agent-life headline R@5 and P@5 / ceiling](eval/charts/solution-headline.svg)

![coding-agent-life R@5 by question type](eval/charts/solution-recall.svg)

**개정 체인 테스트 (`--lineage`):** 각 턴을 이전 턴을 대체하는 `--supersedes` 개정으로 투입하여 상위 $k$개가 각 정답 체인의 최신 노드를 반환하는지(**newest R@k**), 이전 버전 노드가 잘못하여 `latest`로 표시되지 않는지(**stale `latest`**) 검증.

| k | R@k | P@k | newest R@k | stale `latest` |
| ---: | ---: | ---: | ---: | ---: |
| 5 | 1.000 | 0.240 | **1.000** | **0** |
| 10 | 1.000 | 0.120 | **1.000** | **0** |

### 2. 장기 메모리 — `LongMemEval-S`

정제된 `LongMemEval-S`에서 시드 42로 샘플링된 100개 질문 (응답 거부 없음). LLM 리더 없이 $k=10$에서 RRF로 임베딩과 BM25를 융합.

| 임베더 | R@1 | R@5 | R@10 | Hit@1 | Hit@5 | Hit@10 | MRR | 검색 지연시간 P50 / P95 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `hash` (오프라인 테스트) | 0.382 | 0.803 | 0.922 | 0.580 | 0.910 | 0.970 | 0.718 | 24.1 / 35.5 ms |
| **`potion-multilingual-128M`** | **0.474** | **0.909** | **0.952** | **0.740** | **0.980** | **0.990** | **0.840** | 1.12 / 1.34 s |

### 3. 워크플로우 검색 — Agent Retrieval Bench V2 (`edit2ripple`)

44개 고정 저장소 스냅샷에 걸친 58개 변경 기준 질의 (LLM 없음. BCY = 표준 8K 토큰 파일 패킹 프로토콜 하의 예산 내 컨텍스트 적중률).

| 시스템 | Recall@5 | Recall@10 | Recall@20 | MRR | BCY@8k |
| --- | ---: | ---: | ---: | ---: | ---: |
| **`cam` 그래프** | 0.300 | 0.335 | 0.378 | **0.297** | 0.300 |
| 어휘 검색 | **0.412** | **0.536** | **0.588** | 0.243 | **0.447** |
| BM25 | 0.207 | 0.287 | 0.499 | 0.154 | 0.243 |

`cam` 그래프는 가장 높은 첫 번째 적중 순위(MRR `0.297`, 질의 P95 `7.3 ms`)를 달성하며, 어휘 검색은 여러 파일에 걸친 넓은 범위를 제공합니다 — 이는 그래프와 어휘 검색 순위 융합의 필요성을 보여줍니다.

### 4. 코드 그래프 — LongMemCode (`clap`)

`clap` v4.6.1, SCIP gold 대상 536개 시나리오 (LLM 없음, P95 ≈ 5.2 ms, 비용 `$0`).

| 슬라이스 | n | `cam` | `codegraph` 0.9.6 |
| --- | ---: | ---: | ---: |
| **지원됨 (1-hop)** | 478 | **0.769** | 0.768 |
| `lookup` / `file_symbols` | 293 / 55 | 0.808 / 0.915 | 0.808 / 0.915 |
| `callers` / `callees` | 67 / 18 | **0.397** / 0.811 | 0.355 / **0.867** |
| `implementors` | 40 | **0.119** | 0.094 |
| 전체 원시 / 가중 평균 (전체 536) | 536 | **0.709** / **0.704** | 0.704 / 0.700 |

![LongMemCode clap accuracy bars](eval/charts/code-graph-bars.svg)

![LongMemCode clap accuracy by operation](eval/charts/code-graph.svg)

### 5. 멀티턴 토큰 절감 — DeepSeek

전체 컨텍스트 주입 vs 턴별 `cam` 온디맨드 검색 (`deepseek-flash`, hash embedder, 2026-10-13):

| 트랙 | n | 전체 컨텍스트 정확도 | `cam` 정확도 | 전체 컨텍스트 토큰 | `cam` 토큰 | 토큰 절감률 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| **메모리** (`coding-agent-life`) | 15 | 1.00 | 0.87 | 22,796 | 9,904 | **57%** (1,471 → 616 / 턴) |
| **코드** (본 저장소 인덱싱) | 6 | 0.83 | 0.33 | 379,066 | 6,342 | **98%** (63,076 → 1,000 / 턴) |

![DeepSeek multi-turn prompt token bars](eval/charts/multiturn-bars.svg)

![DeepSeek multi-turn prompt tokens](eval/charts/multiturn-tokens.svg)

<details>
<summary><b>모든 벤치마크 재현 방법</b></summary>

데이터셋 경로는 [eval/datasets.toml](eval/datasets.toml)에 설정되어 있습니다 (`coding-agent-life-v1`의 기본 경로는 `../agentmemory` 또는 `CAM_EVAL_CODING_LIFE`).

```bash
python3 eval/coding_life/run.py --adapter grep
python3 eval/coding_life/run.py                                             # potion; 모델 로드 실패 시 에러
python3 eval/coding_life/run.py --hash-embed                                # hash embedder
python3 eval/longmemeval/run.py --sample 100 --seed 42 --workers 8          # potion
python3 eval/longmemeval/run.py --sample 100 --seed 42 --workers 8 --hash-embed
python3 eval/locomo/run.py --workers 4                                      # potion, 증거 리콜

git clone --depth 1 https://github.com/eyuansu62/agent-retrieval-bench \
  eval/agent_retrieval/vendor/agent-retrieval-bench
python3 -m pip install -e eval/agent_retrieval/vendor/agent-retrieval-bench
arb download-benchmark --version v2_edit2ripple --local-dir eval/agent_retrieval/data --force
python3 eval/agent_retrieval/run.py --workers 8

python3 eval/llm_multiturn/run.py --track both   # potion; DEEPSEEK_API_KEY 필요
python3 eval/longmemcode/run.py --corpus clap
python3 eval/charts/generate.py
```

</details>

---

## 지원하는 에이전트

**메인 에이전트**에는 `cam mcp`(stdio)를 등록하고, **서브에이전트**는 추가 설정 없이 셸에서 `cam --json` CLI를 직접 호출합니다.

| 호스트 | MCP 설정 위치 | 서브에이전트 / 비고 |
| --- | --- | --- |
| **Cursor** | `.cursor/mcp.json`, `~/.cursor/mcp.json` 또는 `.cursor-plugin/plugin.json` | `Task` / `explore` 서브에이전트는 CLI 사용 |
| **Claude Code** | `.mcp.json`, `~/.claude.json` 또는 `claude mcp add cam -- cam mcp` | 도구 제한 서브에이전트는 CLI 사용 |
| **Codex** | `~/.codex/config.toml` → `[mcp_servers.cam]` | `codex exec`는 CLI 사용 |
| **Windsurf** | `~/.codeium/windsurf/mcp_config.json` | Cascade는 MCP 사용 |
| **GitHub Copilot** | `.vscode/mcp.json` 또는 `~/.copilot/mcp-config.json` | Chat은 MCP 사용, CLI 스크립트는 CLI 사용 |
| **Continue / Cline** | `~/.continue/config.yaml` / `~/.cline/mcp.json` | |
| **JetBrains** | AI Assistant → MCP (`cam` / `mcp`) | IDE의 `PATH`가 비어있으면 `cam` 절대 경로 지정 |
| **Gemini CLI / Antigravity / OpenCode / Zed / Droid** | [docs/mcp.md](docs/mcp.md) 참조 | |

복사용 설정 템플릿: [docs/mcp.md](docs/mcp.md) · 시스템 프롬프트: [docs/agents.md](docs/agents.md).

---

## 지원하는 언어

| 언어 | 확장자 | 인덱싱되는 노드 및 관계 |
| --- | --- | --- |
| **Rust** | `.rs` | 함수, 구조체(struct), 열거형(enum), 트레이트(trait), 호출 |
| **Python** | `.py` | 함수, 클래스, 호출 |
| **TypeScript** | `.ts`, `.tsx` | 함수, 메서드, 클래스, 호출 |
| **JavaScript** | `.js`, `.jsx` | 함수, 메서드, 클래스, 호출 |
| **Go** | `.go` | 함수, 메서드, 구조체(struct), 호출 |
| **Java** | `.java` | 함수, 메서드, 클래스, 인터페이스, 호출 |
| **C** | `.c` | 함수, 구조체(struct), 호출 |
| **C++** | `.cc`, `.cpp`, `.cxx`, `.h`, `.hpp` | 함수, 메서드, 클래스, 구조체(struct), 호출 |
| **C#** | `.cs` | 함수, 메서드, 클래스, 인터페이스, 호출 |

---

## 지원하는 플랫폼

`cam`은 **macOS**, **Linux**, **Windows**에서 `cargo install --git https://github.com/HanochZhu/codeagent_memory --locked`를 통해 네이티브 컴파일 및 설치가 가능하며, Docker 이미지(`ghcr.io/hanochzhu/codeagent-memory:0.1.5`)를 통해 어디서나 실행할 수 있습니다. [시작하기](#시작하기)를 참조하세요.

---

## 라이선스

MIT

---

<div align="center">

**AI 코딩 에이전트를 위한 설계 — Claude Code, Cursor, Codex, Windsurf, Copilot, JetBrains**

[버그 보고](https://github.com/HanochZhu/codeagent_memory/issues) · [기능 요청](https://github.com/HanochZhu/codeagent_memory/issues)

</div>
