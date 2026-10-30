<div align="center">

<img src="assets/logo.svg" alt="cam logo" width="128" height="128">

# cam

[English](README.md) · [中文](README.zh.md) · [日本語](README.ja.md) · [한국어](README.ko.md) · [العربية](README.ar.md) · [🌐 الموقع](https://hanochzhu.github.io/cam-web/?lang=ar)

### نظام ذاكرة حوارية محلي متعدد الجولات مخصص لوكلاء البرمجة الذكية (Coding Agents)

**رسم بياني للكود + ذاكرة · قراءة دقيقة حسب الرمز · محلي 100% · MCP + CLI**

**النواة مكتوبة بلغة Rust**

[![Website](https://img.shields.io/badge/website-مباشر-brightgreen.svg)](https://hanochzhu.github.io/cam-web/?lang=ar)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/Rust-edition%202021-orange.svg)](https://www.rust-lang.org/)
[![Install](https://img.shields.io/badge/install-cargo--git-blue.svg)](#البدء)

[![Windows](https://img.shields.io/badge/Windows-supported-blue.svg)](#المنصات-المدعومة)
[![macOS](https://img.shields.io/badge/macOS-supported-blue.svg)](#المنصات-المدعومة)
[![Linux](https://img.shields.io/badge/Linux-supported-blue.svg)](#المنصات-المدعومة)

[![Claude Code](https://img.shields.io/badge/Claude_Code-supported-blueviolet.svg)](#الوكلاء-المدعومون)
[![Cursor](https://img.shields.io/badge/Cursor-supported-blueviolet.svg)](#الوكلاء-المدعومون)
[![Codex](https://img.shields.io/badge/Codex-supported-blueviolet.svg)](#الوكلاء-المدعومون)
[![Windsurf](https://img.shields.io/badge/Windsurf-supported-blueviolet.svg)](#الوكلاء-المدعومون)
[![GitHub Copilot](https://img.shields.io/badge/GitHub_Copilot-supported-blueviolet.svg)](#الوكلاء-المدعومون)
[![Continue](https://img.shields.io/badge/Continue-supported-blueviolet.svg)](#الوكلاء-المدعومون)
[![Cline](https://img.shields.io/badge/Cline-supported-blueviolet.svg)](#الوكلاء-المدعومون)
[![JetBrains](https://img.shields.io/badge/JetBrains-supported-blueviolet.svg)](#الوكلاء-المدعومون)

</div>

## المحتويات

- [لماذا cam؟](#لماذا-cam)
- [البدء](#البدء)
- [الميزات الرئيسية](#الميزات-الرئيسية)
- [كيف يعمل](#كيف-يعمل)
- [سير عمل الوكيل (Agent Workflow)](#سير-عمل-الوكيل-agent-workflow)
- [مرجع أوامر CLI](#مرجع-أوامر-cli)
- [قواعد الذاكرة](#قواعد-الذاكرة)
- [الإعدادات والتكوين](#الإعدادات-والتكوين)
- [اختبارات الأداء (Benchmarks)](#اختبارات-الأداء-benchmarks)
- [الوكلاء المدعومون](#الوكلاء-المدعومون)
- [لغات البرمجة المدعومة](#لغات-البرمجة-المدعومة)
- [المنصات المدعومة](#المنصات-المدعومة)
- [الترخيص](#الترخيص)

الأدلة الإرشادية: [الموقع التفاعلي](https://hanochzhu.github.io/cam-web/?lang=ar) · [الوكيل الرئيسي مقابل الوكلاء الفرعيين](docs/agents.md) · [تكوينات MCP حسب بيئة التطوير](docs/mcp.md) · [中文说明](docs/agents.zh.md)

---

## لماذا cam؟

عندما يستكشف وكيل البرمجة الذكي مستودعاً برمجياً، فإنه عادةً ما يعيد اكتشاف بنية المشروع بالطريقة البطيئة — تشغيل `grep` و `glob` وقراءة الملف كاملاً `Read` ملفاً تلو الآخر — ثم ينسى كل شيء في الجلسة التالية.

**يمنح `cam` الوكيل مستويين محليين للاسترجاع في ملف تنفيذي واحد:**

| المستوى | ما الذي يخزنه | كيف يستخدمه الوكيل |
| --- | --- | --- |
| **الرسم البياني للكود (Code Graph)** | رموز tree-sitter (الدوال، الأنواع، الطرق) كنظام ملفات افتراضي + المتصلين/المستدعين من خطوة واحدة (1-hop callers/callees) | `ls`، `find`، `read src/file.rs/symbol`، `ref symbol --dir in\|out` — قراءة جراحية دقيقة للرموز بدلاً من تفريغ الملف كاملاً |
| **شجرة الذاكرة (Memory Tree)** | حقائق المشروع، القرارات المعمارية، ملخصات استكشاف الأخطاء، وعادات المستخدم | `recall` (دمج التضمين الشعاعي + BM25 عبر RRF) و `add` (مع التفريع الهيكلي `--parent` ومراجعة الإصدارات `--supersedes`) |

> **ملف تنفيذي واحد، واجهتان:** يتصل **الوكيل الرئيسي (Main Agent)** عبر **MCP** (`cam mcp`)؛ بينما تتصل **الوكلاء الفرعيون (Subagents)** (الذين نادراً ما يرثون أدوات MCP) عبر نفس واجهة السطر البرمجي **CLI** (`cam --json …`). كلاهما يتشاركان قاعدة البيانات `.cam/cam.db` محلياً بنسبة 100% وبدون أي تكلفة للواجهات البرمجية (Zero API cost).

---

## البدء

### الخيار 1: تثبيت تلقائي بجملة واحدة عبر الوكيل (موصى به)

الصق هذا الموجه (Prompt) مباشرة في وكيل البرمجة الذكي لديك (**Cursor** أو **Claude Code** أو **Codex** أو **Windsurf** أو **GitHub Copilot** أو **Cline** وغيرها) ليتولى التثبيت والفهرسة وضبط الإعدادات تلقائياً:

```text
Install cam from https://github.com/HanochZhu/codeagent_memory: require a working Rust toolchain (rustup), run `cargo install --git https://github.com/HanochZhu/codeagent_memory --locked`, ensure ~/.cargo/bin (or %USERPROFILE%\.cargo\bin on Windows) is on PATH, then verify with `cam --help`. In this repo run `cam index`. For the main agent, register MCP stdio: command `cam`, args `["mcp"]` (Cursor: .cursor/mcp.json; Claude Code: .mcp.json). Subagents call `cam --json` in the shell. Do not add extra docs.
```

<details>
<summary><b>كيفية التعامل مع هذا الموجه في كل بيئة / وكيل</b></summary>

| البيئة | الإجراء المطلوب |
| --- | --- |
| **Cursor / Windsurf / Claude Code / Codex** | الصقه في Agent / Chat؛ وسيقوم الوكيل بتشغيل التثبيت في الطرفية، وبناء `.cam/cam.db`، وكتابة إعدادات MCP |
| **VS Code** (Copilot / Continue / Cline) | الصقه في وضع Agent / Chat مع تفعيل صلاحيات تشغيل الطرفية |
| **JetBrains** (IntelliJ / RustRover / GoLand) | الصقه في AI Assistant، أو شغّل أمر التثبيت يدوياً في الطرفية المدمجة |

<sub>ملاحظة: يقوم `cargo install` بتثبيت `cam` في `~/.cargo/bin` (في Windows: `%USERPROFILE%\.cargo\bin`). يرجى إعادة تشغيل الطرفية أو بيئة التطوير إذا لم يتم التعرف على المسار في `PATH` فوراً.</sub>

</details>

---

### الخيار 2: الإعداد اليدوي

#### 1. تثبيت واجهة السطر البرمجي (CLI)

يتطلب وجود [بيئة عمل Rust (rustup)](https://rustup.rs/):

```bash
cargo install --git https://github.com/HanochZhu/codeagent_memory --locked
cam --help
```

<details>
<summary><b>طرق بديلة: البناء من مستودع محلي أو التشغيل عبر Docker</b></summary>

**البناء من استنساخ محلي:**

```bash
git clone https://github.com/HanochZhu/codeagent_memory.git
cd codeagent_memory
cargo install --path . --locked
```

**Docker (لا يتطلب تثبيت Rust على الجهاز):**

```bash
docker pull ghcr.io/hanochzhu/codeagent-memory:0.1.6
docker run -i --rm -v "$PWD:/work" -w /work ghcr.io/hanochzhu/codeagent-memory:0.1.6
```

أو البناء محلياً: `docker build -t cam https://github.com/HanochZhu/codeagent_memory.git`

</details>

#### 2. فهرسة المشروع والاستعلام

```bash
cd your-project
cam index                                              # تحليل المستودع وبناء .cam/cam.db

# الاستعلام عن رسم الكود البياني
cam ls src/                                            # عرض الملفات أو الرموز داخل ملف
cam read src/main.rs/main                              # قراءة متن رمز محدد فقط
cam ref main --dir in                                  # المتصلون من خطوة واحدة (in) أو المستدعون (out)

# الاستعلام والكتابة في شجرة الذاكرة
cam recall "how does hybrid recall fuse BM25 and vectors"
cam add --summary "..." < notes.md
```

<sub>يقوم `cam` بتهيئة مجلد `.cam/` تلقائياً عند أول تشغيل. يمكنك تشغيل `cam watch` في الخلفية لتحديث الرسم البياني بشكل تزايدي وتلقائي عند حفظ الملفات.</sub>

#### 3. ربط بروتوكول MCP للوكيل الرئيسي

أضف `cam mcp` إلى ملف تكوين MCP الخاص ببيئة التطوير (مثل `.cursor/mcp.json` أو `.mcp.json`):

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

راجع [docs/mcp.md](docs/mcp.md) لمسارات التكوين لمختلف بيئات التطوير، و[docs/agents.md](docs/agents.md) لقواعد توجيه الوكيل الرئيسي والوكلاء الفرعيين.

<sub>يقوم أول استدعاء لأمر `recall` أو `add` بتنزيل نموذج التضمين الافتراضي (`potion-multilingual-128M` وتخزينه في `~/.cam/models/`)، مع التراجع التلقائي إلى نموذج التجزئة المعجمي (hash embedder) في حال عدم توفر اتصال بالإنترنت.</sub>

---

## الميزات الرئيسية

| الميزة | التفاصيل |
| --- | --- |
| **رسم بياني للكود عبر SQLite** | يحلل tree-sitter الدوال والأنواع وعلاقات الاستدعاء داخل `.cam/cam.db` للتنقل الفوري على مستوى الرمز |
| **عنونة المسارات الافتراضية** | يشير `src/main.rs` إلى هيكل الملف العام؛ ويشير `src/main.rs/main` إلى متن الرمز البرمجي مباشرة |
| **مزامنة تزايدية حية** | يعتمد `cam sync` على تجزئة محتوى الملفات؛ ويحافظ `cam watch` على تحديث الرسم البياني عند الحفظ |
| **استرجاع هجين (RRF)** | يدمج التضمينات الشعاعية المحلية الكثيفة (`model2vec`) مع BM25 المدرك للمعلمات عبر دمج الرتب التبادلي (Reciprocal Rank Fusion, $k=60$) |
| **سلاسل المراجعات وشجرة الذاكرة** | يبني `--parent` تسلسلات هرمية متفرعة؛ ويربط `--supersedes` المراجعات الخطية ويطوي الإصدارات القديمة تلقائياً |
| **منحنى إبنجهاوس للاحتفاظ** | يضبط الاحتفاظ $R = e^{-t / S}$ درجات الترتيب بلطف ($\le 10\%$) ويميز الذكريات بـ `stale` / `needs_update` دون حذف السجل |
| **واجهة مزدوجة (MCP + CLI)** | يستخدم الوكيل الرئيسي أدوات `cam_*` عبر MCP؛ بينما يشغل الوكلاء الفرعيون `cam --json …` في الطرفية |
| **محلي وخاص 100%** | لا حاجة لأي مفاتيح واجهات برمجية خارجية (API keys)؛ يعمل كلياً على SQLite والتضمينات المحلية |
| **يدعم 9 لغات برمجية** | Rust و Python و TypeScript و JavaScript و Go و Java و C و C++ و C# |

---

## كيف يعمل

![كيف يعمل cam](docs/images/how-it-works.svg)

1. **الاستخراج (Extract)** — يمر `tree-sitter` على مساحة العمل ويخزن تعريفات الرموز وحواف الاستدعاء داخل `.cam/cam.db`.
2. **القراءة الجراحية (Surgical Read)** — يتنقل الوكلاء عبر المسارات الافتراضية (`ls` → `find` → `read` → `ref`). تعيد قراءة الملف هيكل الرموز، بينما تعيد قراءة `file/symbol` مقطع الكود المصدري الخاص بذلك الرمز فقط.
3. **الاسترجاع الهجين (Hybrid Recall)** — يستقبل `recall` استعلاماً باللغة الإنجليزية، ويولّد التضمين محلياً، ويشغل BM25 المدرك لأسماء المعرفات (مع تفكيك `snake_case` و `kebab-case` و `camelCase` وتجذير الكلمات بالإنجليزية)، ويقتطع الذيول منخفضة الثقة، ويدمج القائمتين عبر **RRF** ($k=60$) مع معامل الاحتفاظ $0.9 + 0.1 \times R$. يتم توسيع المراجعات المستبدلة تلقائياً إلى أحدث بديل لها.
4. **إعادة الكتابة (Write-Back)** — بعد حل أي مهمة برمجية، يتم استخراج حقائق ذرية (atomic facts) بحيث يخزن كل أمر `add` حقيقة واحدة فقط (الملخص بالإنجليزية، والمتن يدعم جميع اللغات غير اللاتينية). ترتبط الحقائق ذات الصلة عبر `--link` / `links` لتكوين رسم بياني معرفي (يمكن فحصه عبر `mem show`؛ علماً أن الاسترجاع عبر روابط الرسم البياني لم يُفعل بعد). يربط `--parent` الذاكرة بموضوع هيكلي، بينما يستبدل `--supersedes` الذاكرة القديمة مع الحفاظ على تسلسل النسب.

تفاصيل المعمارية والتصميم (باللغة الصينية): [DESIGN.md](DESIGN.md).

---

## سير عمل الوكيل (Agent Workflow)

تتبع كل مهمة برمجية حلقة عمل موحدة من 4 خطوات:

1. **الاسترجاع أولاً (Recall first)** — استعلم عبر `cam_recall` / `cam recall` باللغة الإنجليزية قبل استخدام grep. أعد استخدام النتيجة إذا كانت `latest` وليست `needs_update`.
2. **التأكد من الفهرسة (Ensure indexed)** — إذا أبلغت أدوات الرسم البياني عن `not_indexed`، شغّل `cam_index` / `cam index` مرة واحدة للمشروع.
3. **تتبع الرسم البياني (Walk the graph)** — في حال عدم وجود ذاكرة سابقة، حدد الرموز عبر `find` أو `ls`، واقرأ متن الرمز عبر `read <file>/<symbol>`، وتتبع مسار الاستدعاءات عبر `ref <symbol> --dir in|out`.
4. **كتابة النتائج (Write back)** — احفظ الحقائق المعمارية أو الحلول المكتشفة حديثاً عبر `cam_add` / `cam add` كحقائق ذرية (حقيقة واحدة لكل ذاكرة، الملخص بالإنجليزية، والمتن متعدد اللغات)، واربط الحقائق المترابطة عبر `links` / `--link`.

| الدور | الواجهة | الاستخدام النموذجي |
| --- | --- | --- |
| **الوكيل الرئيسي** (جلسة المحادثة / IDE) | **MCP** (`cam mcp`) | يستدعي مباشرة `cam_recall`، `cam_find`، `cam_ls`، `cam_read`، `cam_ref`، `cam_add`، `cam_mem_tree`، `cam_mem_show` |
| **الوكيل الفرعي** (Task / explore / CLI) | **CLI** (`cam --json`) | يشغل `cam --json --project <dir> <cmd>` في الطرفية (نادراً ما ترث الوكلاء الفرعيون أدوات MCP) |

قواعد الوكيل ونصوص التوجيه الكاملة: [docs/agents.md](docs/agents.md).

---

## مرجع أوامر CLI

### أوامر الرسم البياني للكود

| الأمر | الوصف |
| --- | --- |
| `cam index` | تحليل المشروع باستخدام tree-sitter وحفظه في `.cam/cam.db` (ينشئ مجلد `.cam/` إذا لم يوجد) |
| `cam sync` | تحديث تزايدي للرسم البياني للملفات المعدلة بناءً على تجزئة المحتوى |
| `cam watch [--debounce-ms N]` | مراقبة الملفات المصدرية والمزامنة التلقائية (يخرج سطر JSON لكل مزامنة مع `--json`) |
| `cam ls [virt_path]` | سرد المجلدات، أو الملفات، أو الرموز داخل ملف |
| `cam read <virt_path> [--full]` | قراءة هيكل رموز الملف، أو متن رمز محدد (`path/symbol`)، أو الملف كاملاً باستخدام `--full` |
| `cam find <query> [--kind K] [--file S] [--scope D] [--limit N]` | بحث عن تطابق جزئي غير حساس لحالة الأحرف في أسماء الرموز (التطابق التام أولاً، ثم البادئات) |
| `cam ref <symbol> --dir in\|out [--file S] [--kind K] [--scope D]` | المتصلون من خطوة واحدة (`in` / `--callers`) أو المستدعون (`out` / `--callees`). يعيد `candidates` مرتبة عند غموض الاسم |

### أوامر الذاكرة

| الأمر | الوصف |
| --- | --- |
| `cam recall "<query>" [--limit N] [--no-expand] [--include-superseded] [--snippets]` | استرجاع هجين بنظام RRF (استعلام بالإنجليزية). يعطل `--no-expand` التوسع لأحدث مراجعة؛ ويحتفظ `--include-superseded` بالمراجعات القديمة؛ ويقص `--snippets` النصوص الطويلة بعد المرتبة الأولى |
| `cam add --summary "..." [--parent ID] [--supersedes ID] [--link ID]... [--body TEXT \| --file PATH]` | تخزين حقيقة ذرية واحدة في الذاكرة (الملخص بالإنجليزية إلزامي؛ المتن يدعم اللغات غير اللاتينية؛ عبر `--body` أو `--file` أو `stdin`). يربط `--link` الحقائق ذات الصلة؛ ويحدد `--parent` التفرع؛ ويستبدل `--supersedes` مراجعة أقدم |
| `cam mem tree` | عرض الهيكل الهرمي للذاكرة وسلاسل المراجعات |
| `cam mem show <id>` | استعراض محتوى وبيانات إدخال ذاكرة محدد بالكامل |

### أوامر الخادم والتكوين

| الأمر | الوصف |
| --- | --- |
| `cam mcp` | تشغيل خادم stdio MCP للوكيل الرئيسي (انظر [docs/mcp.md](docs/mcp.md)) |
| `cam status` | إظهار جذر المشروع ومسار قاعدة البيانات وإحصاءات العقد/الحواف/الذاكرة والتكوين الفعال |
| `cam config get` | طباعة إعدادات `~/.cam/config.toml` الحالية |
| `cam config set stale_days <N>` | ضبط حد الأيام لتمييز الذاكرة على أنها قديمة (`stale`) |
| `cam config set embedding_model <model>` | تحديد مستودع model2vec من Hugging Face، أو مجلد محلي، أو `hash`، أو `default` |

**الرايات العامة والاتفاقيات:**
- `--project <dir>` (أو متغير البيئة `CAM_PROJECT`) يتجاوز الاكتشاف التلقائي لجذر المشروع `.cam` / `.git`.
- `--json` يخرج بصيغة JSON مدمجة (`--pretty` لـ JSON منسق). تطبع الأخطاء مع `--json` صيغة `{"error":{"code":…,"message":…}}` على `stdout` وتخرج برمز خطأ.
- **الرموز المتشابهة بالاسم:** عندما يتطابق `read` أو `ref` مع تعريفات متعددة، يعيد `status: "ambiguous"` مع معرفات العقد المرشحة (مثل `src/main.rs:function:main:12`) التي يمكن تمريرها مباشرة لأمر `read` أو `ref`.
- **قواعد التجاهل والمشاريع المتعددة:** يلتزم بملفات `.gitignore` و `.camignore` على أي عمق. أي مجلد فرعي يملك `.git` خاصاً به (نسخة مستقلة أو submodule أو linked worktree) يُفهرس في `.cam/cam.db` داخل ذلك المجلد. تحتفظ قاعدة الأب بفهرس هذه الرسوم وتوزع الاستعلامات عليها، لذا فتح المجلد الفرعي يعيد استخدام الرسم نفسه. يعرضها `cam ls` كـ `kind: "project"` (يمكن تمريرها إلى `--scope`).

---

## قواعد الذاكرة

- **الحقائق الذرية وروابط الرسم البياني المعرفي:** قم بتخزين حقيقة ذرية واحدة فقط في كل إدخال ذاكرة لتجنب تكديس المعلومات؛ وعند وجود صلة بين حقائق متعددة، اربطها باستخدام `--link` / `links` لبناء رسم بياني معرفي (استرجاع الرسم البياني غير مفعل حالياً).
- **الاستعلامات والملخصات باللغة الإنجليزية:** يجب كتابة استعلامات `recall` وملخصات الذاكرة باللغة الإنجليزية لضمان اتساق التجذير وفهارس البحث. يدعم متن الذاكرة (body) جميع الحروف غير اللاتينية بشكل كامل (مثل العربية واليابانية والكورية وغيرها).
- **الاحتفاظ والتعزيز:** يضبط احتفاظ إبنجهاوس $R = e^{-t / S}$ نتيجة الاسترجاع عبر المعادلة $\text{relevance} \times (0.9 + 0.1 \times R)$. يتم وسم الإدخالات التي تحقق $R < 0.3$ بـ `needs_update`، وتلك التي تتجاوز `stale_days` (افتراضياً 30 يوماً) بـ `stale`. كل استرجاع صحيح (`needs_update = false`) يجدد وقت الوصول ويضاعف الاستقرار $S$ بمقدار `1.7`.
- **سجل المراجعات غير المدمر:** لا يتم حذف الذكريات أبداً. يشكل `--parent` أشجار تفرع، ويشكل `--supersedes` سلاسل مراجعة خطية حيث يتم تمييز آخر عقدة فقط كـ `latest`، ويتم طي المراجعات القديمة تلقائياً من نتائج البحث الافتراضية.

---

## الإعدادات والتكوين

توجد الإعدادات العامة في `~/.cam/config.toml`:

```toml
stale_days = 30
# embedding_model = "minishlab/potion-multilingual-128M" # افتراضي (مخزن في ~/.cam/models/)
# embedding_model = "minishlab/potion-base-8M"           # نموذج أخف من Hugging Face model2vec
# embedding_model = "/path/to/local/model2vec"           # مجلد محلي يحوي tokenizer.json و config.json و model.safetensors
# embedding_model = "hash"                               # نموذج تجزئة معجمي لا يتطلب أي تنزيل
```

استخدم متغير البيئة `CAM_EMBED_MODEL` لتجاوز النموذج لعملية محددة، أو `cam config set embedding_model default` لإعادة الضبط. عند تغيير النموذج، يعيد أمر `recall` التالي توليد التضمينات للذكريات الموجودة تلقائياً قبل حساب النتائج.

---

## اختبارات الأداء (Benchmarks)

يختبر `cam` كلاً من مستويي الرسم البياني للكود وشجرة الذاكرة بشكل مستقل مقابل أقرب الأنظمة المفتوحة المصدر **على نفس مجموعات البيانات**:

| المستوى | خط الأساس / المقارن | مجموعة بيانات الاختبار | النتيجة البارزة |
| --- | --- | --- | --- |
| **الذاكرة** | [agentmemory](https://github.com/rohitg00/agentmemory) hybrid + `grep` | [coding-agent-life-v1](eval/coding_life/README.md) | **1.000 R@5 / 0.240 P@5** (يطابق سقف النظام الهجين، و **1.000 newest R@5** لسلاسل المراجعات) |
| **الذاكرة طويلة المدى** | استرجاع BM25 / الشعاعي الكثيف | [LongMemEval-S](https://github.com/xiaowu0162/LongMemEval) | **0.909 R@5 / 0.980 Hit@5 / 0.840 MRR** مع `potion-multilingual-128M` |
| **الرسم البياني للكود** | [codegraph](https://github.com/colbymchenry/codegraph) v0.9.6 | [LongMemCode](eval/longmemcode/README.md) (`clap`) | **0.769** مقابل 0.768 في مهام 1-hop المدعومة، مع زمن استجابة P95 ≈ 5.2 ms وتكلفة `$0` |
| **استرجاع سير العمل** | استرجاع الملفات معجمياً / BM25 | [Agent Retrieval Bench V2](https://agent-retrieval-bench.github.io/) `edit2ripple` | **0.297 MRR** (أفضل رتبة لأول نتيجة، زمن استجابة P95 يبلغ 7.3 ms) |
| **استهلاك الرموز (Tokens)** | تفريغ السياق بالكامل | [DeepSeek multi-turn](eval/llm_multiturn/README.md) | **توفير 57% من الرموز** في مسار الذاكرة، و **98% من الرموز** في مسار الكود |

<sub>أطر عمل ذاكرة المحادثات العامة (Mem0 و Zep و Letta) تستهدف سجلات المحادثات العامة وليس مهام وكلاء البرمجة، لذا تم استبعادها من الجداول أدناه.</sub>

### مقارنة الأنظمة

| | **cam** | **agentmemory** | **codegraph** |
| --- | --- | --- | --- |
| **ما الذي يخزنه** | رسم بياني للكود + شجرة ذاكرة | ذكريات الجلسة / المحادثة | رسم بياني للكود |
| **أوليات الاستعلام** | `recall` / `ls` / `find` / `read` / `ref` | `smart-search` / `remember` | `explore` / callers / callees |
| **واجهة الاتصال** | **MCP + CLI** | MCP + REST + hooks | MCP + CLI |
| **الترتيب / الدمج** | شعاعي + BM25 + **RRF** + الاحتفاظ | BM25 + تضمين + إعادة ترتيب | FTS5 + تتبع الرسم البياني |
| **التكلفة** | محلي 100% (`$0`) | خادم محلي | محلي |

### 1. الذاكرة — `coding-agent-life-v1`

15 جلسة، 15 استعلاماً، $k=5$ (نفس مقياس تقييم `score.ts` في `agentmemory`؛ الحد الأقصى النظري لـ P@5 هو `0.240`).

| النظام | معدل الإصابة | R@5 | P@5 | المصدر |
| --- | --- | ---: | ---: | --- |
| `grep` (بحث جزئي مجزأ) | 15 / 15 | 0.967 | 0.227 | هذا المستودع، 2026-09-23 |
| `cam` مجموع min-max (القديم) | 15 / 15 | 0.933 | 0.213 | سجل سابق، hash embed |
| **`cam` RRF** | **15 / 15** | **1.000** | **0.240** | هذا المستودع، hash embed |
| `agentmemory` hybrid | 15 / 15 | 1.000 | 0.240 | منشور v0.9.26 |

![coding-agent-life headline R@5 and P@5 / ceiling](eval/charts/solution-headline.svg)

![coding-agent-life R@5 by question type](eval/charts/solution-recall.svg)

**سلاسل المراجعات (`--lineage`):** اختبار إدخال كل جولة كمراجعة `--supersedes` بديلة للسابقة للتحقق مما إذا كانت النتائج تعيد أحدث عقدة في كل سلسلة ذهبية (**newest R@k**) وما إذا كانت أي عقدة مستبدلة تدعي خطأ أنها `latest` (**stale `latest`**).

| k | R@k | P@k | newest R@k | stale `latest` |
| ---: | ---: | ---: | ---: | ---: |
| 5 | 1.000 | 0.240 | **1.000** | **0** |
| 10 | 1.000 | 0.120 | **1.000** | **0** |

### 2. الذاكرة طويلة المدى — `LongMemEval-S`

100 سؤال من `LongMemEval-S` المنقحة باستخدام البذرة 42. يدمج الاسترجاع التضمينات مع BM25 عبر RRF عند $k=10$ دون استخدام قارئ LLM.

| نموذج التضمين | R@1 | R@5 | R@10 | Hit@1 | Hit@5 | Hit@10 | MRR | زمن الاستجابة P50 / P95 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `hash` (اختبار دون إنترنت) | 0.382 | 0.803 | 0.922 | 0.580 | 0.910 | 0.970 | 0.718 | 24.1 / 35.5 ms |
| **`potion-multilingual-128M`** | **0.474** | **0.909** | **0.952** | **0.740** | **0.980** | **0.990** | **0.840** | 1.12 / 1.34 s |

### 3. استرجاع سير العمل — Agent Retrieval Bench V2 (`edit2ripple`)

58 استعلاماً للتغييرات عبر 44 لقطة مستودع مجمدة (بدون LLM؛ BCY = عائد السياق المحسوب ضمن بروتوكول حزم الملفات القياسي 8K رمز).

| النظام | Recall@5 | Recall@10 | Recall@20 | MRR | BCY@8k |
| --- | ---: | ---: | ---: | ---: | ---: |
| **رسم `cam` البياني** | 0.300 | 0.335 | 0.378 | **0.297** | 0.300 |
| البحث المعجمي | **0.412** | **0.536** | **0.588** | 0.243 | **0.447** |
| BM25 | 0.207 | 0.287 | 0.499 | 0.154 | 0.243 |

يحقق رسم `cam` البياني أعلى رتبة لأول نتيجة (MRR `0.297`، زمن استجابة P95 يبلغ `7.3 ms`)، بينما يوفر البحث المعجمي تغطية أوسع لملفات متعددة — مما يدعم دمج الرتب بين الرسم البياني والبحث المعجمي.

### 4. الرسم البياني للكود — LongMemCode (`clap`)

`clap` v4.6.1، 536 سيناريو مقارنة مع معيار SCIP الذهبي (بدون LLM، زمن استجابة P95 ≈ 5.2 ms، تكلفة `$0`).

| المقطع | n | `cam` | `codegraph` 0.9.6 |
| --- | ---: | ---: | ---: |
| **المدعوم (1-hop)** | 478 | **0.769** | 0.768 |
| `lookup` / `file_symbols` | 293 / 55 | 0.808 / 0.915 | 0.808 / 0.915 |
| `callers` / `callees` | 67 / 18 | **0.397** / 0.811 | 0.355 / **0.867** |
| `implementors` | 40 | **0.119** | 0.094 |
| الإجمالي الخام / الموزون (كل الـ 536) | 536 | **0.709** / **0.704** | 0.704 / 0.700 |

![LongMemCode clap accuracy bars](eval/charts/code-graph-bars.svg)

![LongMemCode clap accuracy by operation](eval/charts/code-graph.svg)

### 5. توفير الرموز (Tokens) في المحادثات متعددة الجولات — DeepSeek

تفريغ السياق بالكامل مقابل الاسترجاع عند الطلب عبر `cam` لكل جولة (`deepseek-flash`، نموذج تجزئة، 2026-10-13):

| المسار | n | الدقة مع السياق الكامل | دقة `cam` | رموز السياق الكامل | رموز `cam` | نسبة التوفير |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| **الذاكرة** (`coding-agent-life`) | 15 | 1.00 | 0.87 | 22,796 | 9,904 | **57%** (من 1,471 إلى 616 / جولة) |
| **الكود** (فهرسة هذا المستودع) | 6 | 0.83 | 0.33 | 379,066 | 6,342 | **98%** (من 63,076 إلى 1,000 / جولة) |

![DeepSeek multi-turn prompt token bars](eval/charts/multiturn-bars.svg)

![DeepSeek multi-turn prompt tokens](eval/charts/multiturn-tokens.svg)

### 6. كتابة الذاكرة

الملاحظات في مستودع مستقل: [cam-write-memory-eval](https://github.com/HanochZhu/cam-write-memory-eval). هذا المستودع يحتفظ بسكربت التشغيل. أول تشغيل لـ `eval/write_memory/run.py` ينزّل `cases.json` إلى `eval/write_memory/data/`. هذا نفس فصل LongMemEval وLoCoMo.

<details>
<summary><b>إعادة إنتاج جميع الاختبارات</b></summary>

مسارات البيانات محددة في [eval/datasets.toml](eval/datasets.toml) (يشير `coding-agent-life-v1` افتراضياً إلى `../agentmemory` أو `CAM_EVAL_CODING_LIFE`).

```bash
python3 eval/coding_life/run.py --adapter grep
python3 eval/coding_life/run.py                                             # potion; يفشل في حال عدم تحميل النموذج
python3 eval/coding_life/run.py --hash-embed                                # hash embedder
python3 eval/longmemeval/run.py --sample 100 --seed 42 --workers 8          # potion
python3 eval/longmemeval/run.py --sample 100 --seed 42 --workers 8 --hash-embed
python3 eval/locomo/run.py --workers 4                                      # potion, استرجاع الأدلة

git clone --depth 1 https://github.com/eyuansu62/agent-retrieval-bench \
  eval/agent_retrieval/vendor/agent-retrieval-bench
python3 -m pip install -e eval/agent_retrieval/vendor/agent-retrieval-bench
arb download-benchmark --version v2_edit2ripple --local-dir eval/agent_retrieval/data --force
python3 eval/agent_retrieval/run.py --workers 8

python3 eval/llm_multiturn/run.py --track both   # potion; يتطلب DEEPSEEK_API_KEY
python3 eval/write_memory/test_score.py          # ينزّل مجموعة الحالات أول مرة، دون استدعاء نموذج
python3 eval/write_memory/run.py                 # ينزّل مجموعة الحالات أول مرة؛ يتطلب DEEPSEEK_API_KEY
python3 eval/longmemcode/run.py --corpus clap
python3 eval/charts/generate.py
```

</details>

---

## الوكلاء المدعومون

سجّل `cam mcp` (stdio) لـ **الوكيل الرئيسي**؛ بينما يستخدم **الوكلاء الفرعيون** واجهة `cam --json` السطرية مباشرة دون أي إعداد إضافي.

| بيئة التطوير | موقع تكوين MCP | الوكيل الفرعي / ملاحظات |
| --- | --- | --- |
| **Cursor** | `.cursor/mcp.json` أو `~/.cursor/mcp.json` أو `.cursor-plugin/plugin.json` | وكلاء `Task` / `explore` يستخدمون CLI |
| **Claude Code** | `.mcp.json` أو `~/.claude.json` أو `claude mcp add cam -- cam mcp` | الوكلاء ذوو الأدوات المحدودة يستخدمون CLI |
| **Codex** | `~/.codex/config.toml` ← `[mcp_servers.cam]` | `codex exec` يستخدم CLI |
| **Windsurf** | `~/.codeium/windsurf/mcp_config.json` | Cascade يستخدم MCP |
| **GitHub Copilot** | `.vscode/mcp.json` أو `~/.copilot/mcp-config.json` | Chat يستخدم MCP؛ السكربتات تستخدم CLI |
| **Continue / Cline** | `~/.continue/config.yaml` / `~/.cline/mcp.json` | |
| **JetBrains** | AI Assistant ← MCP (`cam` / `mcp`) | استخدم المسار المطلق لـ `cam` إذا كان `PATH` فارغاً في IDE |
| **Gemini CLI / Antigravity / OpenCode / Zed / Droid** | انظر [docs/mcp.md](docs/mcp.md) | |

نماذج التكوين الجاهزة: [docs/mcp.md](docs/mcp.md) · نصوص توجيه الوكلاء: [docs/agents.md](docs/agents.md).

---

## لغات البرمجة المدعومة

| اللغة | الامتدادات | العقد والحواف المفهرسة |
| --- | --- | --- |
| **Rust** | `.rs` | الدوال، الهياكل (structs)، التعدادات (enums)، السمات (traits)، الاستدعاءات |
| **Python** | `.py` | الدوال، الأصناف (classes)، الاستدعاءات |
| **TypeScript** | `.ts`, `.tsx` | الدوال، الطرق (methods)، الأصناف، الاستدعاءات |
| **JavaScript** | `.js`, `.jsx` | الدوال، الطرق، الأصناف، الاستدعاءات |
| **Go** | `.go` | الدوال، الطرق، الهياكل، الاستدعاءات |
| **Java** | `.java` | الدوال، الطرق، الأصناف، الواجهات (interfaces)، الاستدعاءات |
| **C** | `.c` | الدوال، الهياكل، الاستدعاءات |
| **C++** | `.cc`, `.cpp`, `.cxx`, `.h`, `.hpp` | الدوال، الطرق، الأصناف، الهياكل، الاستدعاءات |
| **C#** | `.cs` | الدوال، الطرق، الأصناف، الواجهات، الاستدعاءات |

---

## المنصات المدعومة

يتم تجميع وتثبيت `cam` بشكل أصلي على **macOS** و **Linux** و **Windows** عبر الأمر `cargo install --git https://github.com/HanochZhu/codeagent_memory --locked`، أو تشغيله في أي مكان عبر حاوية Docker الرسمية (`ghcr.io/hanochzhu/codeagent-memory:0.1.6`). انظر قسم [البدء](#البدء).

---

## الترخيص

MIT

---

<div align="center">

**صُمم خصيصاً لوكلاء البرمجة الذكية — Claude Code و Cursor و Codex و Windsurf و Copilot و JetBrains**

[الإبلاغ عن خطأ](https://github.com/HanochZhu/codeagent_memory/issues) · [طلب ميزة جديدة](https://github.com/HanochZhu/codeagent_memory/issues)

</div>
