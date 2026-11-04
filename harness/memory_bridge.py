#!/usr/bin/env python3
"""
Bridge to read and write Tuzygent Harness Lessons adhering to HarnessLesson Rust struct schema.
"""

import json
import os
import uuid
from datetime import datetime, timezone
from pathlib import Path
from typing import List, Dict, Optional

MEMORY_DIR = Path("storage/harness_memory")
MEMORY_FILE = MEMORY_DIR / "harness_lessons.json"


def ensure_memory_dir():
    MEMORY_DIR.mkdir(parents=True, exist_ok=True)


def load_lessons() -> List[Dict]:
    if not MEMORY_FILE.exists():
        return []
    try:
        with open(MEMORY_FILE, "r", encoding="utf-8") as f:
            return json.load(f)
    except Exception as e:
        print(f"[Warning] Failed to load harness lessons: {e}")
        return []


def save_lessons(lessons: List[Dict]):
    ensure_memory_dir()
    temp_file = MEMORY_FILE.with_suffix(f".tmp.{uuid.uuid4().hex[:8]}")
    with open(temp_file, "w", encoding="utf-8") as f:
        json.dump(lessons, f, indent=2, ensure_ascii=False)
    temp_file.replace(MEMORY_FILE)


def record_lesson(
    node_type: str,
    identifier: str,
    error_pattern: str,
    remediation_strategy: str,
) -> Dict:
    lessons = load_lessons()
    now_iso = datetime.now(timezone.utc).isoformat()

    # Deduplicate / update existing lesson if similar
    for l in lessons:
        if (
            l.get("node_type") == node_type
            and l.get("identifier") == identifier
            and (
                error_pattern in l.get("error_pattern", "")
                or l.get("error_pattern", "") in error_pattern
            )
        ):
            l["success_count"] = l.get("success_count", 0) + 1
            l["last_applied_at"] = now_iso
            l["remediation_strategy"] = remediation_strategy
            save_lessons(lessons)
            return l

    new_lesson = {
        "id": f"les-{uuid.uuid4().hex[:8]}",
        "node_type": node_type,
        "identifier": identifier,
        "error_pattern": error_pattern,
        "remediation_strategy": remediation_strategy,
        "success_count": 1,
        "created_at": now_iso,
        "last_applied_at": now_iso,
    }
    lessons.append(new_lesson)
    save_lessons(lessons)
    return new_lesson


def find_lessons(node_type: Optional[str] = None) -> List[Dict]:
    lessons = load_lessons()
    if not node_type:
        return lessons
    return [l for l in lessons if l.get("node_type") == node_type or l.get("identifier") == "*"]


if __name__ == "__main__":
    import sys
    if len(sys.argv) > 1 and sys.argv[1] == "list":
        for l in load_lessons():
            print(f"- [{l['id']}] ({l['node_type']}) {l['error_pattern'][:60]} -> {l['remediation_strategy'][:60]}")
