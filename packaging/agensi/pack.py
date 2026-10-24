"""Build an Agensi skill zip from skills/codeagent-memory.

The archive has one top-level folder, SKILL.md at its root, and text files only.
"""

from __future__ import annotations

import re
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SKILL_DIR = ROOT / "skills" / "codeagent-memory"
OUT = ROOT / "dist" / "agensi" / "codeagent-memory.zip"
FOLDER = "codeagent-memory"

FILES = (
    "SKILL.md",
    "README.md",
    "LICENSE.txt",
    "references/commands.md",
    "references/memory-rules.md",
    "references/mcp.md",
    "references/install.md",
    "examples/session.md",
)

TEXT_SUFFIXES = {".md", ".txt"}
NAME_RE = re.compile(r"^[a-z0-9]+(?:-[a-z0-9]+)*$")

# Patterns that match the published Agensi scan categories closely enough
# to catch a bad package before upload. This is a local gate, not their scanner.
BLOCKERS = (
    ("private key", re.compile(r"BEGIN (?:RSA |OPENSSH |EC )?PRIVATE KEY")),
    ("aws access key", re.compile(r"AKIA[0-9A-Z]{16}")),
    ("github token", re.compile(r"ghp_[A-Za-z0-9]{20,}")),
    ("slack token", re.compile(r"xox[baprs]-[A-Za-z0-9-]{10,}")),
    ("piped remote script", re.compile(r"(?:curl|wget)\b[^\n]{0,160}\|\s*(?:sh|bash)\b", re.I)),
    ("encoded shell", re.compile(r"powershell(?:\.exe)?\s+-enc\b", re.I)),
    ("invoke-expression", re.compile(r"\biex\s*\(|Invoke-Expression", re.I)),
    ("recursive delete", re.compile(r"\brm\s+-[^\n]*r")),
    ("privilege escalation", re.compile(r"\bsudo\b")),
    ("world-writable mode", re.compile(r"chmod\s+777")),
    ("prompt override", re.compile(r"ignore\s+(?:all\s+)?(?:previous|prior)\s+instructions", re.I)),
)

NOTES = (
    ("outbound url", re.compile(r"https?://")),
    ("package install command", re.compile(r"\bcargo install\b")),
)


def fail(message: str) -> None:
    print(f"error: {message}", file=sys.stderr)
    raise SystemExit(1)


def frontmatter(text: str) -> dict[str, str]:
    if not text.startswith("---\n"):
        fail("SKILL.md is missing opening frontmatter")
    end = text.find("\n---\n", 4)
    if end < 0:
        fail("SKILL.md frontmatter is not closed")
    body = text[end + 5 :].strip()
    if not body:
        fail("SKILL.md has no instructions after frontmatter")
    meta: dict[str, str] = {}
    for line in text[4:end].splitlines():
        if not line.strip():
            continue
        if ":" not in line:
            fail(f"SKILL.md frontmatter line is not key: value ({line})")
        key, value = line.split(":", 1)
        meta[key.strip()] = value.strip().strip('"').strip("'")
    return meta


def check_skill(text: str) -> None:
    meta = frontmatter(text)
    name = meta.get("name", "")
    description = meta.get("description", "")
    if set(meta) - {"name", "description"}:
        extra = ", ".join(sorted(set(meta) - {"name", "description"}))
        fail(f"SKILL.md has extra frontmatter fields: {extra}")
    if name != FOLDER:
        fail(f"SKILL.md name {name!r} does not match folder {FOLDER!r}")
    if not NAME_RE.fullmatch(name) or len(name) > 64:
        fail(f"SKILL.md name {name!r} is not a lowercase slug of at most 64 characters")
    if not description or len(description) > 1024:
        fail("SKILL.md description must be 1 to 1024 characters")


def scan(rel: str, text: str) -> list[str]:
    notes: list[str] = []
    for label, pattern in BLOCKERS:
        if pattern.search(text):
            fail(f"{rel} matched blocked pattern: {label}")
    for label, pattern in NOTES:
        if pattern.search(text):
            notes.append(f"note: {rel} contains {label}; confirm it is documented and user-initiated")
    return notes


def build() -> None:
    notes: list[str] = []
    payloads: list[tuple[str, bytes]] = []
    for rel in FILES:
        path = SKILL_DIR / rel
        if not path.is_file():
            fail(f"missing {path}")
        if path.is_symlink():
            fail(f"symlink is not allowed: {rel}")
        if any(part.startswith(".") for part in Path(rel).parts):
            fail(f"hidden path is not allowed: {rel}")
        if path.suffix.lower() not in TEXT_SUFFIXES:
            fail(f"unexpected file type: {rel}")
        data = path.read_bytes()
        if b"\0" in data:
            fail(f"binary content is not allowed: {rel}")
        text = data.decode("utf-8").replace("\r\n", "\n").replace("\r", "\n")
        if text.startswith("\ufeff"):
            fail(f"BOM is not allowed: {rel}")
        if rel == "SKILL.md":
            check_skill(text)
        notes.extend(scan(rel, text))
        payloads.append((f"{FOLDER}/{rel}", text.encode("utf-8")))

    OUT.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(OUT, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for arcname, data in payloads:
            info = zipfile.ZipInfo(arcname)
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o644 << 16
            archive.writestr(info, data)

    with zipfile.ZipFile(OUT) as archive:
        names = archive.namelist()
    tops = {name.split("/", 1)[0] for name in names}
    if tops != {FOLDER}:
        fail(f"zip top-level entries are {sorted(tops)}")
    if f"{FOLDER}/SKILL.md" not in names:
        fail("zip is missing SKILL.md at the skill root")
    if any(name.count("/") == 0 for name in names):
        fail("a file sits at the zip root; expected one containing folder")

    print(f"wrote {OUT}")
    print(f"entries: {len(names)}")
    for name in names:
        print(f"  {name}")
    for note in notes:
        print(note)


if __name__ == "__main__":
    build()
