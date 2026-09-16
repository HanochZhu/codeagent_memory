#!/usr/bin/env python3
"""Draw README bar + line charts from completed eval JSON files."""

from __future__ import annotations

import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
EVAL = HERE.parent

BLUE = "#2563eb"
BLUE_LIGHT = "#93c5fd"
SLATE = "#64748b"
GREEN = "#16a34a"
ORANGE = "#ea580c"
GRAY = "#94a3b8"
INK = "#0f172a"
GRID = "#e2e8f0"
BG = "#ffffff"

TYPE_ORDER = [
    ("single-session-bug", "bug"),
    ("single-session-infra", "infra"),
    ("single-session-refactor", "refactor"),
    ("single-session-feature", "feature"),
    ("single-session-test", "test"),
    ("single-session-perf", "perf"),
    ("single-session-api", "api"),
    ("single-session-db", "db"),
    ("single-session-release", "release"),
    ("multi-session-causal", "causal"),
    ("preference", "pref"),
    ("multi-session-review", "review"),
    ("temporal", "temporal"),
]

OPS = ["lookup", "file_symbols", "contained_by", "callers", "callees", "implementors", "weighted"]

# Published agentmemory v0.9.26 scorecard — not re-run on this machine.
HYBRID_BY_TYPE = {key: 1.0 for key, _ in TYPE_ORDER}
HYBRID_R = 1.0
HYBRID_P = 0.240


def svg_escape(text: str) -> str:
    return text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def latest_life() -> dict[str, dict]:
    found: dict[str, tuple[float, dict]] = {}
    for path in (EVAL / "coding_life" / "results").glob("cam-life-*.json"):
        data = json.loads(path.read_text())
        summary = data["summary"]
        adapter = summary.get("adapter") or "cam"
        fusion = summary.get("fusion")
        key = "grep" if adapter == "grep" else f"cam-{fusion or 'sum'}"
        mtime = path.stat().st_mtime
        if key not in found or mtime > found[key][0]:
            found[key] = (mtime, data)
    return {k: v[1] for k, v in found.items()}


def r_by_type(data: dict) -> list[float]:
    by = data["summary"]["by_type"]
    return [by[full]["R@k"] for full, _ in TYPE_ORDER]


def load_op(path: Path, name: str) -> list[float]:
    summary = json.loads(path.read_text())["summary"]
    ops = summary["by_op"]
    out = [ops[op]["avg"] for op in OPS[:-1]]
    out.append(summary["weighted_accuracy"])
    return out


def line_chart(
    path: Path,
    *,
    title: str,
    labels: list[str],
    series: list[tuple[str, str, list[float], str]],
    y_max: float,
    y_ticks: list[float],
    width: int = 760,
    height: int = 340,
    y_fmt: str = "{:.2f}",
) -> None:
    left, right, top, bottom = 64, 16, 36, 78
    inner_w = width - left - right
    inner_h = height - top - bottom
    n = len(labels)
    xs = [left + inner_w * i / max(n - 1, 1) for i in range(n)]

    def y_of(v: float) -> float:
        return top + inner_h * (1 - v / y_max)

    parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" '
        f'viewBox="0 0 {width} {height}" role="img">',
        f"<title>{svg_escape(title)}</title>",
        f'<rect width="{width}" height="{height}" fill="{BG}"/>',
        f'<text x="{left}" y="22" fill="{INK}" font-size="14" font-family="ui-sans-serif,system-ui,sans-serif" font-weight="600">{svg_escape(title)}</text>',
    ]
    for tick in y_ticks:
        y = y_of(tick)
        parts.append(
            f'<line x1="{left}" y1="{y:.1f}" x2="{width - right}" y2="{y:.1f}" stroke="{GRID}" stroke-width="1"/>'
        )
        parts.append(
            f'<text x="{left - 8}" y="{y + 4:.1f}" text-anchor="end" fill="{SLATE}" font-size="11" font-family="ui-sans-serif,system-ui,sans-serif">{y_fmt.format(tick)}</text>'
        )
    for x, label in zip(xs, labels):
        parts.append(
            f'<text x="{x:.1f}" y="{height - 48}" text-anchor="end" transform="rotate(-32 {x:.1f} {height - 48})" fill="{SLATE}" font-size="11" font-family="ui-sans-serif,system-ui,sans-serif">{svg_escape(label)}</text>'
        )
    for name, color, values, dash in series:
        pts = " ".join(f"{x:.1f},{y_of(v):.1f}" for x, v in zip(xs, values))
        dash_attr = f' stroke-dasharray="{dash}"' if dash else ""
        parts.append(
            f'<polyline fill="none" stroke="{color}" stroke-width="2.4" stroke-linejoin="round" stroke-linecap="round"{dash_attr} points="{pts}"/>'
        )
        for x, v in zip(xs, values):
            parts.append(f'<circle cx="{x:.1f}" cy="{y_of(v):.1f}" r="3.4" fill="{color}"/>')
    legend_x = left
    legend_y = height - 14
    for name, color, _, _ in series:
        parts.append(
            f'<rect x="{legend_x}" y="{legend_y - 9}" width="10" height="10" rx="2" fill="{color}"/>'
        )
        parts.append(
            f'<text x="{legend_x + 14}" y="{legend_y}" fill="{INK}" font-size="11" font-family="ui-sans-serif,system-ui,sans-serif">{svg_escape(name)}</text>'
        )
        legend_x += 18 + 7 * len(name) + 18
    parts.append("</svg>")
    path.write_text("\n".join(parts) + "\n")


def bar_chart(
    path: Path,
    *,
    title: str,
    labels: list[str],
    series: list[tuple[str, str, list[float]]],
    y_max: float,
    y_ticks: list[float],
    width: int = 760,
    height: int = 340,
    y_fmt: str = "{:.2f}",
    value_fmt: str = "{:.3f}",
) -> None:
    left, right, top, bottom = 64, 16, 36, 72
    inner_w = width - left - right
    inner_h = height - top - bottom
    groups = len(labels)
    n_series = len(series)
    gap = 18
    group_w = (inner_w - gap * (groups - 1)) / groups
    bar_w = group_w / max(n_series, 1) * 0.82
    bar_gap = group_w / max(n_series, 1) * 0.18

    def y_of(v: float) -> float:
        return top + inner_h * (1 - min(v, y_max) / y_max)

    parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" '
        f'viewBox="0 0 {width} {height}" role="img">',
        f"<title>{svg_escape(title)}</title>",
        f'<rect width="{width}" height="{height}" fill="{BG}"/>',
        f'<text x="{left}" y="22" fill="{INK}" font-size="14" font-family="ui-sans-serif,system-ui,sans-serif" font-weight="600">{svg_escape(title)}</text>',
    ]
    for tick in y_ticks:
        y = y_of(tick)
        parts.append(
            f'<line x1="{left}" y1="{y:.1f}" x2="{width - right}" y2="{y:.1f}" stroke="{GRID}" stroke-width="1"/>'
        )
        parts.append(
            f'<text x="{left - 8}" y="{y + 4:.1f}" text-anchor="end" fill="{SLATE}" font-size="11" font-family="ui-sans-serif,system-ui,sans-serif">{y_fmt.format(tick)}</text>'
        )
    axis_y = y_of(0)
    for g, label in enumerate(labels):
        gx = left + g * (group_w + gap)
        cx = gx + group_w / 2
        parts.append(
            f'<text x="{cx:.1f}" y="{height - 42}" text-anchor="middle" fill="{SLATE}" font-size="11" font-family="ui-sans-serif,system-ui,sans-serif">{svg_escape(label)}</text>'
        )
        for i, (name, color, values) in enumerate(series):
            v = values[g]
            x = gx + i * (bar_w + bar_gap)
            y = y_of(v)
            h = axis_y - y
            parts.append(
                f'<rect x="{x:.1f}" y="{y:.1f}" width="{bar_w:.1f}" height="{max(h, 0):.1f}" rx="2" fill="{color}"/>'
            )
            parts.append(
                f'<text x="{x + bar_w / 2:.1f}" y="{y - 4:.1f}" text-anchor="middle" fill="{INK}" font-size="10" font-family="ui-sans-serif,system-ui,sans-serif">{value_fmt.format(v)}</text>'
            )
    legend_x = left
    legend_y = height - 14
    for name, color, _ in series:
        parts.append(
            f'<rect x="{legend_x}" y="{legend_y - 9}" width="10" height="10" rx="2" fill="{color}"/>'
        )
        parts.append(
            f'<text x="{legend_x + 14}" y="{legend_y}" fill="{INK}" font-size="11" font-family="ui-sans-serif,system-ui,sans-serif">{svg_escape(name)}</text>'
        )
        legend_x += 18 + 7 * len(name) + 18
    parts.append("</svg>")
    path.write_text("\n".join(parts) + "\n")


def main() -> None:
    life = latest_life()
    grep = life["grep"]
    cam_sum = life["cam-sum"]
    cam_rrf = life["cam-rrf"]
    labels = [short for _, short in TYPE_ORDER]

    bar_chart(
        HERE / "solution-headline.svg",
        title="coding-agent-life-v1 · headline R@5 and P@5 / ceiling",
        labels=["grep", "cam sum", "cam RRF", "agentmemory"],
        series=[
            ("R@5", BLUE, [grep["summary"]["R@k"], cam_sum["summary"]["R@k"], cam_rrf["summary"]["R@k"], HYBRID_R]),
            (
                "P@5 / ceiling",
                ORANGE,
                [
                    grep["summary"]["P@k"] / 0.24,
                    cam_sum["summary"]["P@k"] / 0.24,
                    cam_rrf["summary"]["P@k"] / 0.24,
                    HYBRID_P / 0.24,
                ],
            ),
        ],
        y_max=1.0,
        y_ticks=[0.0, 0.25, 0.5, 0.75, 1.0],
    )
    line_chart(
        HERE / "solution-recall.svg",
        title="coding-agent-life-v1 · R@5 by question type",
        labels=labels,
        series=[
            ("grep", SLATE, r_by_type(grep), ""),
            ("cam sum", BLUE_LIGHT, r_by_type(cam_sum), ""),
            ("agentmemory hybrid", GREEN, [HYBRID_BY_TYPE[k] for k, _ in TYPE_ORDER], "6 4"),
            ("cam RRF", BLUE, r_by_type(cam_rrf), ""),
        ],
        y_max=1.0,
        y_ticks=[0.0, 0.25, 0.5, 0.75, 1.0],
    )

    cam_ops = load_op(EVAL / "longmemcode" / "results" / "cam-clap-20260915-214803.json", "cam")
    cg_ops = load_op(EVAL / "longmemcode" / "results" / "codegraph-clap-20260914-222758.json", "codegraph")
    bar_chart(
        HERE / "code-graph-bars.svg",
        title="LongMemCode clap · accuracy (cam vs codegraph)",
        labels=OPS,
        series=[
            ("cam", BLUE, cam_ops),
            ("codegraph", ORANGE, cg_ops),
        ],
        y_max=1.0,
        y_ticks=[0.0, 0.25, 0.5, 0.75, 1.0],
    )
    line_chart(
        HERE / "code-graph.svg",
        title="LongMemCode clap · accuracy by operation",
        labels=OPS,
        series=[
            ("cam", BLUE, cam_ops, ""),
            ("codegraph", ORANGE, cg_ops, "6 4"),
        ],
        y_max=1.0,
        y_ticks=[0.0, 0.25, 0.5, 0.75, 1.0],
    )

    llm = json.loads((EVAL / "llm_multiturn" / "results" / "llm-multiturn-20260915-235953.json").read_text())
    tracks = llm["summary"]["tracks"]
    sol = tracks["solutions"]["compare"]
    code = tracks["code"]["compare"]
    bar_chart(
        HERE / "multiturn-bars.svg",
        title="DeepSeek multi-turn · prompt tokens",
        labels=["solutions", "code"],
        series=[
            ("full dump", GRAY, [sol["full_tokens"], code["full_tokens"]]),
            ("cam retrieve", BLUE, [sol["cam_tokens"], code["cam_tokens"]]),
        ],
        y_max=180000,
        y_ticks=[0, 45000, 90000, 135000, 180000],
        y_fmt="{:,.0f}",
        value_fmt="{:,.0f}",
        width=560,
    )
    line_chart(
        HERE / "multiturn-tokens.svg",
        title="DeepSeek multi-turn · prompt tokens (full dump vs cam)",
        labels=["solutions", "code"],
        series=[
            ("full dump", GRAY, [sol["full_tokens"], code["full_tokens"]], ""),
            ("cam retrieve", BLUE, [sol["cam_tokens"], code["cam_tokens"]], ""),
        ],
        y_max=180000,
        y_ticks=[0, 45000, 90000, 135000, 180000],
        y_fmt="{:,.0f}",
        width=560,
        height=300,
    )
    print("wrote", ", ".join(p.name for p in sorted(HERE.glob("*.svg"))))


if __name__ == "__main__":
    main()
