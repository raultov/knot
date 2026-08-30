#!/usr/bin/env python3
"""Measure the LLM token cost of answering code-exploration questions with knot
versus answering them by grepping and reading source files.

The benchmark runs three task types against already-indexed repositories:

  discovery  "Where is X implemented?"
             knot     -> `knot search "<query>" --repo <r> --output markdown`
             baseline -> `rg -l <keyword>` (candidate list) + full read of the
                         files that actually answer the question
  callers    "Who calls/uses X?"
             knot     -> `knot callers "<symbol>" --repo <r> --output markdown`
             baseline -> `rg -n "\\b<symbol>\\b"` + full read of the first
                         `--max-baseline-files` distinct files with hits
  explore    "What is inside this file?"
             knot     -> `knot explore "<file>" --repo <r> --output markdown`
             baseline -> full read of the file

The baseline greps are restricted to the source files of the repository's
language (`rg_types` in the config), which is what a competent agent does — no
changelogs, no generated documentation. Both sides of the comparison are
measured on the exact bytes an LLM would receive as tool output.

Token counts use tiktoken's `cl100k_base` encoding when available and fall back
to a chars/4 approximation otherwise (the fallback is reported in the output).

Requirements: ripgrep (`rg`) on PATH, a built `knot` binary, the repositories of
the config already indexed, and (optionally) `pip install tiktoken`.

Usage:
    python3 scripts/token_savings_benchmark.py --config scripts/token_savings_tasks.json
    python3 scripts/token_savings_benchmark.py --config <cfg> --format json
    python3 scripts/token_savings_benchmark.py --config <cfg> --save-json out.json
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
from dataclasses import dataclass, asdict
from pathlib import Path

DEFAULT_KNOT_BIN = "./target/release/knot"
# `--sort path` forces a deterministic traversal order so repeated runs pick the
# same baseline files and produce reproducible token counts.
RG_EXCLUDES = [
    "--sort", "path",
    "-g", "!node_modules",
    "-g", "!target",
    "-g", "!.git",
    "-g", "!dist",
]


def build_counter():
    """Return (count_fn, tokenizer_name)."""
    try:
        import tiktoken  # type: ignore

        enc = tiktoken.get_encoding("cl100k_base")
        return (lambda text: len(enc.encode(text, disallowed_special=()))), "cl100k_base"
    except Exception:
        return (lambda text: (len(text) + 3) // 4), "chars/4 (approximation)"


@dataclass
class TaskResult:
    repo: str
    language: str
    task_type: str
    question: str
    knot_command: str
    knot_tokens: int
    baseline_description: str
    baseline_tokens: int
    baseline_files_read: int

    @property
    def reduction_pct(self) -> float:
        if self.baseline_tokens == 0:
            return 0.0
        return 100.0 * (1.0 - self.knot_tokens / self.baseline_tokens)

    @property
    def ratio(self) -> float:
        if self.knot_tokens == 0:
            return float("inf")
        return self.baseline_tokens / self.knot_tokens


def run(cmd: list[str], cwd: str | None = None) -> str:
    proc = subprocess.run(
        cmd, cwd=cwd, capture_output=True, text=True, errors="replace"
    )
    return proc.stdout


def read_text(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return ""


def rg_available() -> bool:
    return shutil.which("rg") is not None


def rg_type_flags(types: list[str]) -> list[str]:
    flags: list[str] = []
    for t in types:
        flags += ["-t", t]
    return flags


def baseline_discovery(
    root: Path, keyword: str, answer_files: list[str], types: list[str], count
) -> tuple[int, int, str]:
    """Candidate file list from ripgrep + full read of the answering files."""
    listing = run(["rg", "-l", "--no-messages", *RG_EXCLUDES, *rg_type_flags(types), keyword, str(root)])
    tokens = count(listing)
    for rel in answer_files:
        tokens += count(read_text(root / rel))
    desc = f"`rg -l {keyword}` + full read of {len(answer_files)} file(s)"
    return tokens, len(answer_files), desc


def baseline_callers(
    root: Path, symbol: str, types: list[str], max_files: int, count
) -> tuple[int, int, str]:
    """Ripgrep hit list + full read of the first N distinct files with hits."""
    hits = run(
        ["rg", "-n", "--no-messages", *RG_EXCLUDES, *rg_type_flags(types), rf"\b{symbol}\b", str(root)]
    )
    tokens = count(hits)
    seen: list[str] = []
    for line in hits.splitlines():
        path = line.split(":", 1)[0]
        if path not in seen:
            seen.append(path)
        if len(seen) >= max_files:
            break
    for path in seen:
        tokens += count(read_text(Path(path)))
    desc = f"`rg -n '\\b{symbol}\\b'` + full read of {len(seen)} hit file(s)"
    return tokens, len(seen), desc


def baseline_explore(root: Path, file_rel: str, count) -> tuple[int, int, str]:
    tokens = count(read_text(root / file_rel))
    return tokens, 1, f"full read of `{file_rel}`"


def run_task(task: dict, knot_bin: str, knot_cwd: str, max_files: int, count) -> TaskResult:
    repo = task["repo"]
    root = Path(os.path.expanduser(os.path.expandvars(task["root"])))
    ttype = task["type"]

    types = task.get("rg_types", [])

    if ttype == "discovery":
        knot_cmd = [
            knot_bin, "search", task["query"], "--repo", repo,
            "--max-results", str(task.get("max_results", 5)), "--output", "markdown",
        ]
        base_tokens, files_read, base_desc = baseline_discovery(
            root, task["grep_keyword"], task["answer_files"], types, count
        )
    elif ttype == "callers":
        knot_cmd = [knot_bin, "callers", task["symbol"], "--repo", repo, "--output", "markdown"]
        base_tokens, files_read, base_desc = baseline_callers(
            root, task["symbol"], types, max_files, count
        )
    elif ttype == "explore":
        knot_cmd = [knot_bin, "explore", task["file"], "--repo", repo, "--output", "markdown"]
        base_tokens, files_read, base_desc = baseline_explore(root, task["file"], count)
    else:
        raise ValueError(f"unknown task type: {ttype}")

    knot_out = run(knot_cmd, cwd=knot_cwd)
    return TaskResult(
        repo=repo,
        language=task["language"],
        task_type=ttype,
        question=task["question"],
        knot_command=" ".join(knot_cmd[1:]),
        knot_tokens=count(knot_out),
        baseline_description=base_desc,
        baseline_tokens=base_tokens,
        baseline_files_read=files_read,
    )


def markdown_report(results: list[TaskResult], tokenizer: str) -> str:
    lines = [
        f"Tokenizer: {tokenizer}",
        "",
        "| Repo | Lang | Task | knot tokens | Read-the-code tokens | Reduction | Ratio |",
        "|------|------|------|------------:|---------------------:|----------:|------:|",
    ]
    for r in results:
        lines.append(
            f"| {r.repo} | {r.language} | {r.task_type} | {r.knot_tokens:,} | "
            f"{r.baseline_tokens:,} | {r.reduction_pct:.1f}% | {r.ratio:.0f}x |"
        )
    total_knot = sum(r.knot_tokens for r in results)
    total_base = sum(r.baseline_tokens for r in results)
    reduction = 100.0 * (1.0 - total_knot / total_base) if total_base else 0.0
    ratio = total_base / total_knot if total_knot else float("inf")
    lines.append(
        f"| **TOTAL** | — | {len(results)} tasks | **{total_knot:,}** | "
        f"**{total_base:,}** | **{reduction:.1f}%** | **{ratio:.0f}x** |"
    )
    return "\n".join(lines)


def json_payload(results: list[TaskResult], tokenizer: str) -> dict:
    total_knot = sum(r.knot_tokens for r in results)
    total_base = sum(r.baseline_tokens for r in results)
    return {
        "tokenizer": tokenizer,
        "totals": {
            "knot_tokens": total_knot,
            "baseline_tokens": total_base,
            "reduction_pct": round(100.0 * (1.0 - total_knot / total_base), 2) if total_base else 0.0,
            "ratio": round(total_base / total_knot, 2) if total_knot else None,
        },
        "results": [
            asdict(r) | {"reduction_pct": round(r.reduction_pct, 2), "ratio": round(r.ratio, 2)}
            for r in results
        ],
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--config", required=True, help="JSON file describing the benchmark tasks")
    parser.add_argument("--knot-bin", default=os.environ.get("KNOT_BIN", DEFAULT_KNOT_BIN))
    parser.add_argument("--knot-cwd", default=".", help="Working directory used to invoke the knot CLI")
    parser.add_argument("--max-baseline-files", type=int, default=5)
    parser.add_argument("--format", choices=["markdown", "json"], default="markdown")
    parser.add_argument("--save-json", help="Also write the raw measurements to this path")
    args = parser.parse_args()

    if not rg_available():
        print("error: ripgrep (`rg`) is required for the baseline measurements", file=sys.stderr)
        return 1

    count, tokenizer = build_counter()
    config = json.loads(Path(args.config).read_text(encoding="utf-8"))

    results = [
        run_task(task, args.knot_bin, args.knot_cwd, args.max_baseline_files, count)
        for task in config["tasks"]
    ]

    if args.save_json:
        Path(args.save_json).write_text(
            json.dumps(json_payload(results, tokenizer), indent=2) + "\n", encoding="utf-8"
        )

    if args.format == "json":
        print(json.dumps(json_payload(results, tokenizer), indent=2))
    else:
        print(markdown_report(results, tokenizer))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
