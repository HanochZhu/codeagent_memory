"""Memory systems that can run here with DeepSeek and no extra server.

cam uses its own one-fact write rule and potion recall.
mem0 is the open-source library, not Mem0 Cloud, with its default extractor.
"""

from __future__ import annotations

import re
import tempfile
from pathlib import Path

from llm import chat, complete, model_name
from run import Mcp, tool_schema

RULE = (
    "Store what this dialogue states about the user with cam_add. One memory is one fact. "
    "If two pieces can be asked about separately, store them as two memories. "
    "Put the memories from this dialogue in one cam_add call as memories. "
    "If a new fact replaces one of the existing memories listed below, set supersedes to that id. "
    "Do not store a claim the user did not make. Summary must be English. "
    "Call cam_add. Do not answer in prose until the tool call is done."
)
EMBED_MODEL = "BAAI/bge-small-en-v1.5"
EMBED_DIMS = 384
UPDATE_K = 10
QA_K = 20


def dialogue_lines(session: dict) -> str:
    lines = []
    for turn in session.get("dialogue") or []:
        role = turn.get("role") or "unknown"
        stamp = turn.get("timestamp") or ""
        lines.append(f"[{stamp}] {role}: {turn.get('content', '')}")
    return "\n".join(lines)


def messages_of(session: dict) -> list[dict]:
    messages = []
    for turn in session.get("dialogue") or []:
        role = str(turn.get("role") or "user")
        if role not in ("user", "assistant", "system"):
            role = "user"
        stamp = turn.get("timestamp") or ""
        messages.append({"role": role, "content": f"[{stamp}] {turn.get('content', '')}"})
    return messages


class CamSystem:
    name = "cam"
    note = "cam one-fact cam_add; recall uses potion-multilingual-128M"

    def __init__(self, hash_embed: bool):
        self.hash_embed = hash_embed
        self.root = Path(tempfile.mkdtemp(prefix="halu-cam-"))
        self.mcp = Mcp(self.root, hash_embed)
        self.known: list[dict] = []

    def close(self) -> None:
        self.mcp.close()

    def add_session(self, session: dict) -> list[str]:
        listing = "Existing memories: none."
        recent = self.known[-40:]
        if recent:
            listing = "Existing memories:\n" + "\n".join(
                f"{item['id']}: {item.get('summary')}" for item in recent
            )
        messages = [
            {"role": "system", "content": RULE},
            {"role": "user", "content": listing + "\n\nDialogue:\n" + dialogue_lines(session)},
        ]
        tools = tool_schema()
        texts = []
        for _ in range(3):
            data = chat(messages, tools)
            message = (data.get("choices") or [{}])[0].get("message") or {}
            tool_calls = message.get("tool_calls") or []
            if not tool_calls:
                break
            messages.append(
                {"role": "assistant", "content": message.get("content") or "", "tool_calls": tool_calls}
            )
            import json

            for call in tool_calls:
                raw = call.get("function", {}).get("arguments") or "{}"
                try:
                    arguments = json.loads(raw)
                except json.JSONDecodeError:
                    arguments = {}
                arguments["hash_embed"] = self.hash_embed
                result = self.mcp.call_tool("cam_add", arguments)
                texts.extend(self._take(result))
                messages.append(
                    {
                        "role": "tool",
                        "tool_call_id": call.get("id"),
                        "content": json.dumps(result),
                    }
                )
        return texts

    def _take(self, result: dict) -> list[str]:
        if not isinstance(result, dict) or result.get("is_error"):
            return []
        items = result.get("memories")
        if items is None and result.get("id"):
            items = [result]
        texts = []
        for item in items or []:
            if not isinstance(item, dict) or not item.get("id"):
                continue
            summary = item.get("summary") or ""
            body = item.get("body") or ""
            if not body:
                shown = self.mcp.call_tool("cam_mem_show", {"id": item["id"]})
                if isinstance(shown, dict):
                    summary = shown.get("summary") or summary
                    body = shown.get("body") or ""
            self.known.append({"id": item["id"], "summary": summary or item["id"]})
            text = "\n".join(part for part in (summary, body) if part)
            if text:
                texts.append(text)
        return texts

    def search(self, query: str, top_k: int) -> list[str]:
        cleaned = re.sub(r"\s+", " ", query).strip()
        if not cleaned:
            return []
        try:
            result = self.mcp.call_tool(
                "cam_recall",
                {"query": cleaned, "limit": max(1, top_k), "expand": False},
            )
        except Exception:
            return []
        rows = result if isinstance(result, list) else []
        lines = []
        for item in rows[:top_k]:
            if not isinstance(item, dict):
                continue
            text = "\n".join(
                part for part in (item.get("summary") or "", item.get("body") or "") if part
            )
            if text:
                lines.append(text)
        return lines


class Mem0System:
    name = "mem0"
    note = (
        "mem0 open-source default extractor on DeepSeek; "
        f"vectors are {EMBED_MODEL} in local Qdrant, not Mem0 Cloud"
    )

    def __init__(self, _hash_embed: bool):
        from mem0 import Memory

        from llm import api_key, base_url

        self.root = Path(tempfile.mkdtemp(prefix="halu-mem0-"))
        self.user_id = "halumem"
        self.memory = Memory.from_config(
            {
                "history_db_path": str(self.root / "history.db"),
                "llm": {
                    "provider": "deepseek",
                    "config": {
                        "model": model_name(),
                        "temperature": 0,
                        "api_key": api_key(),
                        "deepseek_base_url": base_url(),
                        "max_tokens": 4000,
                    },
                },
                "embedder": {
                    "provider": "fastembed",
                    "config": {"model": EMBED_MODEL, "embedding_dims": EMBED_DIMS},
                },
                "vector_store": {
                    "provider": "qdrant",
                    "config": {
                        "collection_name": "halumem",
                        "path": str(self.root / "qdrant"),
                        "on_disk": True,
                        "embedding_model_dims": EMBED_DIMS,
                    },
                },
            }
        )

    def close(self) -> None:
        return None

    def add_session(self, session: dict) -> list[str]:
        result = self.memory.add(messages_of(session), user_id=self.user_id)
        rows = result.get("results") if isinstance(result, dict) else []
        return [str(item.get("memory")) for item in rows or [] if item.get("memory")]

    def search(self, query: str, top_k: int) -> list[str]:
        result = self.memory.search(
            query,
            top_k=max(1, top_k),
            filters={"user_id": self.user_id},
        )
        rows = result.get("results") if isinstance(result, dict) else []
        return [str(item.get("memory")) for item in rows or [] if item.get("memory")]


SYSTEMS = {
    "cam": CamSystem,
    "mem0": Mem0System,
}


def answer(context: str, question: str) -> str:
    from judge import READER

    return complete(READER.format(context=context or "(none)", question=question), max_tokens=200).strip()
