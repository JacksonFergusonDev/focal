"""Bounded GitHub collection for release context, with visible progress."""

import json
import os
import signal
import subprocess
import time
from contextlib import suppress
from typing import Any

from focal.errors import print_info, print_warning


class ReleaseGitHub:
    """Share a deadline and failure circuit across all release GitHub requests."""

    def __init__(self, request_timeout: float = 10, github_timeout: float = 60) -> None:
        self.request_timeout = request_timeout
        self.github_timeout = github_timeout
        self.deadline = time.monotonic() + github_timeout
        self.stopped = False
        self.notes: list[str] = []
        self.requests = 0

    def _stop(self, reason: str) -> None:
        if not self.stopped:
            self.stopped = True
            self.notes.append(
                f"WARNING: {reason}; remaining GitHub enrichment skipped. Local Git evidence is preserved."
            )
            print_warning(self.notes[-1])

    @staticmethod
    def _kill(process: subprocess.Popen[str]) -> None:
        """Terminate the request and its credential-helper process group."""
        with suppress(ProcessLookupError):
            os.killpg(process.pid, signal.SIGKILL)
        try:
            process.communicate(timeout=1)
        except subprocess.TimeoutExpired:
            # Do not block cleanup on pipes inherited by detached descendants.
            if process.stdout:
                process.stdout.close()
            if process.stderr:
                process.stderr.close()
            process.wait(timeout=1)

    def request(self, args: list[str], **_kwargs: Any) -> Any:
        """Run one noninteractive gh request with bounded process-tree cleanup."""
        if self.stopped:
            return None
        remaining = self.deadline - time.monotonic()
        if remaining <= 0:
            self._stop(
                f"GitHub collection exceeded its {self.github_timeout:g}s budget"
            )
            return None
        timeout = min(self.request_timeout, remaining)
        self.requests += 1
        operation = " ".join(args[:2])
        print_info(
            f"GitHub request {self.requests}: {operation} (timeout {timeout:.1f}s)"
        )
        env = {**os.environ, "GH_PROMPT_DISABLED": "1", "GH_PAGER": "cat"}
        try:
            process = subprocess.Popen(
                ["gh", *args],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                env=env,
                start_new_session=True,
            )
        except OSError as error:
            self._stop(f"could not start gh: {error}")
            return None
        try:
            stdout, stderr = process.communicate(timeout=timeout)
        except subprocess.TimeoutExpired:
            self._kill(process)
            self._stop(f"gh {operation} timed out after {timeout:.1f}s")
            return None
        except KeyboardInterrupt:
            self._kill(process)
            raise
        try:
            result = json.loads(stdout)
        except json.JSONDecodeError:
            result = None
        errors = result.get("errors", []) if isinstance(result, dict) else []
        error_text = (stderr + " " + json.dumps(errors)).lower()
        if any(
            term in error_text
            for term in (
                "rate limit",
                "rate_limit",
                "ratelimited",
                "http 429",
                "abuse detection",
                "retry-after",
            )
        ):
            self._stop(
                "GitHub rate limit reached; rerun after the limit resets (no automatic retries)"
            )
            if isinstance(result, dict) and result.get("data"):
                return result
            return None
        if any(
            term in error_text
            for term in (
                "http 401",
                "http 403",
                "authentication",
                "authenticate",
                "gh auth login",
                "bad credentials",
                "resource not accessible",
                "forbidden",
            )
        ):
            self._stop("GitHub authentication or permission failure")
            return None
        if process.returncode != 0 or errors:
            message = f"GitHub request failed: gh {operation}: {stderr.strip() or json.dumps(errors)}"
            self.notes.append(f"WARNING: {message}")
            print_warning(message)
            # GraphQL can return useful objects alongside errors for other aliases.
            if isinstance(result, dict) and result.get("data"):
                return result
            if any(
                term in error_text
                for term in (
                    "timeout",
                    "connection",
                    "network",
                    "tls",
                    "dial tcp",
                    "http 50",
                )
            ):
                self._stop("GitHub connection or service failure")
            return None
        if result is None:
            self._stop(f"gh {operation} returned invalid JSON")
        if isinstance(result, dict):
            rate_limit = (result.get("data") or {}).get("rateLimit")
            if isinstance(rate_limit, dict) and rate_limit.get("remaining") == 0:
                self._stop(
                    f"GitHub GraphQL quota exhausted; reset at {rate_limit.get('resetAt', 'the server reset time')}"
                )
        return result
