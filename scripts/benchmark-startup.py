#!/usr/bin/env python3
"""Measure CLI-backed desktop reads using synthetic, temporary agent homes only."""

import argparse
import concurrent.futures
import json
import os
from pathlib import Path
import statistics
import subprocess
import tempfile
import time


def fixtures(root, size):
    counts = {"small": (8, 12, 16, 8), "large": (80, 200, 160, 100)}
    skills, memories, sessions, turns = counts[size]
    homes = {agent: root / agent for agent in ("claude", "codex", "opencode", "pi")}
    payload = "Synthetic benchmark content. " * 32
    for home in homes.values():
        home.mkdir()
        for number in range(skills):
            directory = home / "skills" / f"skill-{number:04}"
            directory.mkdir(parents=True)
            (directory / "SKILL.md").write_text(
                f"---\nname: skill-{number:04}\ndescription: Synthetic skill\n---\n\n{payload}\n"
            )
            for resource in range(8):
                (directory / f"resource-{resource}.txt").write_text(payload)
    project = root / "project"
    project.mkdir()
    encoded = "".join(c if c.isascii() and c.isalnum() else "-" for c in str(project))
    directory = homes["claude"] / "projects" / encoded
    memory = directory / "memory"
    memory.mkdir(parents=True)
    (directory / "session.jsonl").write_text(json.dumps({"cwd": str(project)}) + "\n")
    index = ["# Memory Index\n"]
    for number in range(memories):
        name = f"memory-{number:04}.md"
        (memory / name).write_text(
            f"---\nname: Memory {number}\ndescription: Synthetic memory\ntype: project\n---\n\n{payload}\n"
        )
        index.append(f"- [Memory {number}]({name}) — Synthetic memory\n")
    (memory / "MEMORY.md").write_text("\n".join(index))
    for number in range(sessions):
        native_id = f"synthetic-{number:04}"
        with (directory / f"{native_id}.jsonl").open("w") as output:
            for turn in range(turns):
                role = "user" if turn % 2 == 0 else "assistant"
                output.write(json.dumps({
                    "type": role, "uuid": f"u-{turn}", "timestamp": "2026-11-11T10:00:00Z",
                    "cwd": str(project), "sessionId": native_id,
                    "message": {"role": role, "content": payload},
                }) + "\n")
    return homes, {"skills_per_agent": skills, "memories": memories,
                   "sessions": sessions, "turns_per_session": turns}


def measure(binary, size, mode, runs, compare=None):
    with tempfile.TemporaryDirectory(prefix="nativemem-startup-") as temporary:
        root = Path(temporary)
        homes, counts = fixtures(root, size)
        env = {key: value for key, value in os.environ.items()
               if not key.startswith("NATIVEMEM_") and key not in (
                   "CLAUDE_CONFIG_DIR", "CODEX_HOME", "OPENCODE_CONFIG_DIR",
                   "PI_CODING_AGENT_DIR", "XDG_DATA_HOME")}
        env.update(HOME=str(root), PATH=str(root / "bin"),
                   XDG_CONFIG_HOME=str(root / "config"))
        roots = []
        for agent, home in homes.items():
            roots.extend((f"--{agent}-home", str(home)))
        roots.extend(("--opencode-data-home", str(root / "opencode-data"),
                      "--state-dir", str(root / "state")))

        def invoke(executable, arguments):
            started = time.perf_counter()
            result = subprocess.run([str(executable), *roots, *arguments, "--json"], env=env,
                                    cwd=root, capture_output=True, check=False)
            if result.returncode not in (0, 3):
                raise RuntimeError(f"Command failed ({result.returncode}): {arguments}")
            document = json.loads(result.stdout)
            return (time.perf_counter() - started) * 1000, document

        commands = {"items": ["items"], "sessions": ["items", "--kind", "session"],
                    "status": ["status"], "doctor": ["doctor"]}
        profiles = {mode: binary} if compare is None else {"commands": compare, "snapshot": binary}
        timings = {profile: {key: [] for key in (*commands, "first_content", "content_with_sessions")}
                   for profile in profiles}
        for iteration in range(runs):
            # Alternate order when comparing binaries on the exact same fixture tree.
            order = list(profiles.items())
            if iteration % 2:
                order.reverse()
            for profile, executable in order:
                samples = timings[profile]
                started = time.perf_counter()
                if profile == "commands":
                    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
                        pending = {key: pool.submit(invoke, executable, args)
                                   for key, args in commands.items()}
                        documents = {}
                        for key, future in pending.items():
                            elapsed, document = future.result()
                            samples[key].append(elapsed)
                            documents[key] = document
                    elapsed = (time.perf_counter() - started) * 1000
                    samples["first_content"].append(elapsed)
                    samples["content_with_sessions"].append(elapsed)
                else:
                    elapsed, document = invoke(executable, ["--no-sessions", "snapshot"])
                    samples["first_content"].append(elapsed)
                    session_ms, sessions = invoke(executable, commands["sessions"])
                    samples["sessions"].append(session_ms)
                    samples["content_with_sessions"].append((time.perf_counter() - started) * 1000)
                    documents = {"items": document["items"], "sessions": sessions,
                                 "status": document["status"], "doctor": document["doctor"]}
                # Verify meaningful work, never print entry text or transcript bodies.
                assert len(documents["items"]["items"]) == counts["memories"] + 4 * counts["skills_per_agent"]
                assert len(documents["sessions"]["items"]) == counts["sessions"]
                assert all(item["body"] == "" for item in documents["sessions"]["items"])
        return {"fixture": size, "runs": runs, "counts": counts,
                "profiles": {profile: {
                    key: {"median": round(statistics.median(values), 2),
                          "min": round(min(values), 2), "max": round(max(values), 2)}
                    for key, values in samples.items() if values
                } for profile, samples in timings.items()},
                "units": "milliseconds",
                "notes": "Fresh processes; OS filesystem caches not flushed. Core CLI reads only, not rendered app launch, hub loading or network tests."}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--compare", type=Path, help="Compare commands from this binary against snapshot on the same fixture tree")
    parser.add_argument("--mode", choices=("commands", "snapshot"), default="snapshot")
    parser.add_argument("--size", choices=("small", "large"), default="large")
    parser.add_argument("--runs", type=int, default=5)
    args = parser.parse_args()
    if args.runs < 1:
        parser.error("--runs must be positive")
    compare = args.compare.resolve(strict=True) if args.compare else None
    print(json.dumps(measure(args.binary.resolve(strict=True), args.size, args.mode, args.runs, compare), indent=2))


if __name__ == "__main__":
    main()
