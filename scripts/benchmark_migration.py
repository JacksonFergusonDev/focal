"""Measure public Focal commands on a reproducible warm-cache repository fixture."""

import argparse
import json
import os
import platform
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path


def measure(binary: Path, args: list[str], repo: Path) -> tuple[float, int, bytes]:
    """Capture elapsed seconds, child peak RSS in bytes, and the command payload."""
    with tempfile.TemporaryFile() as output, tempfile.TemporaryFile() as error:
        start = time.perf_counter()
        with subprocess.Popen(
            [binary, *args], cwd=repo, stdout=output, stderr=error
        ) as proc:
            _, status, usage = os.wait4(proc.pid, 0)
            proc.returncode = os.waitstatus_to_exitcode(status)
            elapsed = time.perf_counter() - start
            if proc.returncode:
                error.seek(0)
                raise RuntimeError(error.read().decode(errors="replace"))
        output.seek(0)
        rss = usage.ru_maxrss * (1 if sys.platform == "darwin" else 1024)
        return elapsed, rss, output.read()


def main() -> None:
    """Benchmark each supplied binary against the same fixture and print JSON lines."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", nargs="+", type=Path)
    parser.add_argument(
        "--command", choices=["tree", "files", "context"], default="files"
    )
    parser.add_argument("--files", type=int, default=100)
    parser.add_argument("--samples", type=int, default=3)
    options = parser.parse_args()
    if min(options.files, options.samples) < 1:
        parser.error("files and samples must be positive")
    args = (
        [options.command] if options.command == "tree" else [options.command, "--all"]
    )
    with tempfile.TemporaryDirectory() as scratch:
        repo = Path(scratch)
        subprocess.run(["git", "init", "-q", "-b", "main"], cwd=repo, check=True)
        for index in range(options.files):
            path = repo / f"pkg{index % 8}" / f"module{index}.py"
            path.parent.mkdir(exist_ok=True)
            path.write_text(f"def value():\n    return {index}\n")
        reference = None
        for binary in options.binary:
            binary = binary.resolve()
            # One warm-up per binary. Compare complete blocks independent of discovery order.
            _, _, payload = measure(binary, args, repo)
            if options.command == "files":
                header, body = payload.split(b"\n---\n\n", 1)
                blocks = sorted(
                    body.removesuffix(b"\n</context>\n").split(b"\n\n")[:-1]
                )
                signature = (header, blocks)
                if len(blocks) != options.files or (
                    reference is not None and signature != reference
                ):
                    raise ValueError("file outputs differ on the benchmark fixture")
                reference = signature
            samples = [measure(binary, args, repo) for _ in range(options.samples)]
            print(
                json.dumps(
                    {
                        "binary": str(binary),
                        "args": args,
                        "platform": platform.platform(),
                        "files": options.files,
                        "samples": len(samples),
                        "cache": "warm",
                        "median_seconds": statistics.median(s[0] for s in samples),
                        "median_peak_rss_bytes": statistics.median(
                            s[1] for s in samples
                        ),
                        "output_bytes": len(payload),
                    }
                ),
                flush=True,
            )


if __name__ == "__main__":
    main()
