#!/usr/bin/env python3
"""M1c live API smoke test.

This script assumes Postgres and `cargo run -p plexus-server` are already
running. It does not start services, reset databases, or manage Docker.

Environment is loaded from, in order:
  1. /home/yucheng/Documents/GitHub/Plexus/.env
  2. /home/yucheng/Documents/GitHub/Plexus/scripts/.env
  3. the current process environment, which wins over file values

Required values:
  ADMIN_TOKEN
  LLM_API_KEY
  LLM_API_BASE_URL or LLM_ENDPOINT
  LLM_MODEL_NAME or LLM_MODEL

Optional values:
  API defaults to http://127.0.0.1:8080
  M1C_SMOKE_TIMEOUT_SECONDS defaults to 240
"""

from __future__ import annotations

import argparse
import json
import os
import queue
import random
import shlex
import socket
import string
import sys
import threading
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable


SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parent
DEFAULT_ENV_FILES = (REPO_ROOT / ".env", SCRIPT_DIR / ".env")
INLINE_PNG_DATA_URL = (
    "data:image/png;base64,"
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/"
    "x8AAwMCAO+/p9sAAAAASUVORK5CYII="
)


class SmokeError(Exception):
    pass


@dataclass(frozen=True)
class Config:
    api: str
    admin_token: str
    llm_endpoint: str
    llm_model: str
    llm_api_key: str
    timeout_seconds: float
    skip_provider_precheck: bool
    skip_image: bool


@dataclass
class HttpResult:
    status: int
    headers: dict[str, str]
    body_text: str
    json_body: Any


@dataclass
class SseEvent:
    event: str
    data: str
    event_id: str | None
    json_data: Any


class SseParser:
    def __init__(self) -> None:
        self._event = ""
        self._data: list[str] = []
        self._id: str | None = None

    def feed(self, raw_line: bytes) -> SseEvent | None:
        line = raw_line.decode("utf-8", errors="replace").rstrip("\r\n")
        if line == "":
            if self._event == "" and not self._data and self._id is None:
                return None
            event = self._event or "message"
            data = "\n".join(self._data)
            event_id = self._id
            self._event = ""
            self._data = []
            self._id = None
            parsed: Any = None
            if data:
                try:
                    parsed = json.loads(data)
                except json.JSONDecodeError:
                    parsed = None
            return SseEvent(event, data, event_id, parsed)

        if line.startswith(":"):
            return None

        field, _, value = line.partition(":")
        if value.startswith(" "):
            value = value[1:]
        if field == "event":
            self._event = value
        elif field == "data":
            self._data.append(value)
        elif field == "id":
            self._id = value
        return None


class SseClient:
    def __init__(self, url: str, token: str, timeout_seconds: float, last_event_id: str | None = None) -> None:
        self._url = url
        self._token = token
        self._timeout_seconds = timeout_seconds
        self._last_event_id = last_event_id
        self._events: list[SseEvent] = []
        self._events_lock = threading.Lock()
        self._queue: queue.Queue[SseEvent | Exception] = queue.Queue()
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None
        self._response: Any = None

    def start(self) -> None:
        if self._thread is not None:
            raise SmokeError("SSE client already started")
        self._thread = threading.Thread(target=self._run, name="m1c-smoke-sse", daemon=True)
        self._thread.start()

    def wait_for(self, predicate: Callable[[SseEvent], bool], label: str, timeout_seconds: float | None = None) -> SseEvent:
        deadline = time.monotonic() + (timeout_seconds or self._timeout_seconds)
        checked = 0
        while time.monotonic() < deadline:
            with self._events_lock:
                events = list(self._events)
            for event in events[checked:]:
                if predicate(event):
                    return event
            checked = len(events)

            remaining = deadline - time.monotonic()
            if remaining <= 0:
                break
            try:
                item = self._queue.get(timeout=min(1.0, remaining))
            except queue.Empty:
                continue
            if isinstance(item, Exception):
                raise SmokeError(f"SSE stream failed while waiting for {label}: {item}") from item

        with self._events_lock:
            seen = [(event.event, event.event_id) for event in self._events]
        raise SmokeError(f"Timed out waiting for {label}; SSE events seen: {seen}")

    def snapshot(self) -> list[SseEvent]:
        with self._events_lock:
            return list(self._events)

    def close(self) -> None:
        self._stop.set()
        if self._response is not None:
            try:
                self._response.close()
            except Exception:
                pass
        if self._thread is not None:
            self._thread.join(timeout=5)

    def _run(self) -> None:
        parser = SseParser()
        headers = {"Authorization": f"Bearer {self._token}"}
        if self._last_event_id:
            headers["Last-Event-ID"] = self._last_event_id
        request = urllib.request.Request(self._url, headers=headers, method="GET")

        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                self._response = response
                while not self._stop.is_set():
                    try:
                        raw_line = response.readline()
                    except socket.timeout:
                        continue
                    if raw_line == b"":
                        break
                    event = parser.feed(raw_line)
                    if event is None:
                        continue
                    with self._events_lock:
                        self._events.append(event)
                    self._queue.put(event)
        except ValueError:
            if not self._stop.is_set():
                self._queue.put(SmokeError("SSE stream closed unexpectedly"))
        except Exception as exc:
            if not self._stop.is_set():
                self._queue.put(exc)


def main() -> int:
    args = parse_args()
    try:
        env = load_env(args.env_file)
        cfg = build_config(env, args)
        run_smoke(cfg)
    except SmokeError as exc:
        print(f"[FAIL] {exc}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        print("\n[FAIL] interrupted", file=sys.stderr)
        return 130
    return 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run the M1c live API smoke test against a running Plexus server.")
    parser.add_argument(
        "--env-file",
        action="append",
        type=Path,
        help="Extra env file to load after the default .env files. Can be repeated.",
    )
    parser.add_argument("--api", help="Plexus API base URL. Overrides API from env.")
    parser.add_argument("--timeout-seconds", type=float, help="Per-step timeout. Overrides M1C_SMOKE_TIMEOUT_SECONDS.")
    parser.add_argument("--skip-provider-precheck", action="store_true", help="Skip direct provider /models precheck.")
    parser.add_argument("--skip-image", action="store_true", help="Skip inline-image message smoke step.")
    return parser.parse_args()


def load_env(extra_files: list[Path] | None) -> dict[str, str]:
    file_env: dict[str, str] = {}
    for path in [*DEFAULT_ENV_FILES, *(extra_files or [])]:
        if path.exists():
            file_env.update(parse_env_file(path))
    return {**file_env, **os.environ}


def parse_env_file(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for line_number, raw_line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("export "):
            line = line[len("export ") :].strip()
        if "=" not in line:
            raise SmokeError(f"{path}:{line_number}: expected KEY=value")
        key, value = line.split("=", 1)
        key = key.strip()
        value = strip_env_value(value.strip())
        if not key:
            raise SmokeError(f"{path}:{line_number}: empty env key")
        values[key] = value
    return values


def strip_env_value(value: str) -> str:
    if not value:
        return value
    if value[0] in {"'", '"'}:
        try:
            parts = shlex.split(value)
        except ValueError as exc:
            raise SmokeError(f"invalid quoted env value: {exc}") from exc
        if len(parts) != 1:
            raise SmokeError("quoted env value must contain exactly one shell token")
        return parts[0]
    return value


def build_config(env: dict[str, str], args: argparse.Namespace) -> Config:
    api = (args.api or env.get("API") or "http://127.0.0.1:8080").rstrip("/")
    admin_token = required_env(env, "ADMIN_TOKEN")
    llm_api_key = required_env(env, "LLM_API_KEY")
    llm_endpoint = env.get("LLM_API_BASE_URL") or env.get("LLM_ENDPOINT")
    llm_model = env.get("LLM_MODEL_NAME") or env.get("LLM_MODEL")
    if not llm_endpoint:
        raise SmokeError("missing LLM_API_BASE_URL or LLM_ENDPOINT")
    if not llm_model:
        raise SmokeError("missing LLM_MODEL_NAME or LLM_MODEL")

    timeout = args.timeout_seconds
    if timeout is None:
        timeout = float(env.get("M1C_SMOKE_TIMEOUT_SECONDS", "240"))
    if timeout <= 0:
        raise SmokeError("timeout must be positive")

    return Config(
        api=api,
        admin_token=admin_token,
        llm_endpoint=llm_endpoint.rstrip("/"),
        llm_model=llm_model,
        llm_api_key=llm_api_key,
        timeout_seconds=timeout,
        skip_provider_precheck=args.skip_provider_precheck,
        skip_image=args.skip_image,
    )


def required_env(env: dict[str, str], key: str) -> str:
    value = env.get(key)
    if value is None or value.strip() == "":
        raise SmokeError(f"missing required env var {key}")
    return value


def run_smoke(cfg: Config) -> None:
    print(f"[info] Plexus API: {cfg.api}")
    print(f"[info] LLM endpoint: {cfg.llm_endpoint}")
    print(f"[info] LLM model: {cfg.llm_model}")
    print("[info] LLM API key loaded: yes")

    if not cfg.skip_provider_precheck:
        provider_precheck(cfg)

    email = unique_email()
    password = "smoke-password-123456"

    auth = step(
        "register admin",
        lambda: request_json(
            "POST",
            join_url(cfg.api, "/api/auth/register"),
            body={
                "email": email,
                "password": password,
                "name": "M1c Smoke Admin",
                "admin_token": cfg.admin_token,
            },
            expected_status={201},
            timeout_seconds=cfg.timeout_seconds,
        ),
    )
    token = require_string(auth.json_body, "jwt", "register response")
    user = auth.json_body.get("user")
    if not isinstance(user, dict) or user.get("is_admin") is not True:
        raise SmokeError("registered user is not admin; check ADMIN_TOKEN")

    me = step("GET /api/me", lambda: authed_json(cfg, "GET", "/api/me", token))
    if me.json_body.get("email") != email:
        raise SmokeError("GET /api/me returned a different user")

    patched = step(
        "PATCH /api/admin/config",
        lambda: authed_json(
            cfg,
            "PATCH",
            "/api/admin/config",
            token,
            body={
                "llm_endpoint": cfg.llm_endpoint,
                "llm_api_key": cfg.llm_api_key,
                "llm_model": cfg.llm_model,
                "llm_max_concurrent_requests": 0,
            },
        ),
    )
    assert_redacted_config(patched.json_body, cfg)

    current_config = step("GET /api/admin/config", lambda: authed_json(cfg, "GET", "/api/admin/config", token))
    assert_redacted_config(current_config.json_body, cfg)

    session = step(
        "POST /api/sessions",
        lambda: authed_json(cfg, "POST", "/api/sessions", token, body={"title": "M1c live smoke"}, expected_status={201}),
    )
    session_id = require_string(session.json_body, "id", "session response")
    if session.json_body.get("channel") != "web":
        raise SmokeError("created session is not a web session")

    sessions = step("GET /api/sessions", lambda: authed_json(cfg, "GET", "/api/sessions", token))
    if not any(isinstance(item, dict) and item.get("id") == session_id for item in sessions.json_body):
        raise SmokeError("created session was not returned by GET /api/sessions")

    step("GET /api/sessions/{id}", lambda: authed_json(cfg, "GET", f"/api/sessions/{session_id}", token))

    live = SseClient(join_url(cfg.api, f"/api/sessions/{session_id}/stream?replay_limit=0"), token, cfg.timeout_seconds)
    live.start()
    try:
        step("SSE history_end", lambda: live.wait_for(lambda event: event.event == "history_end", "history_end"))

        text_post = step(
            "POST text message",
            lambda: authed_json(
                cfg,
                "POST",
                f"/api/sessions/{session_id}/messages",
                token,
                body={
                    "reasoning_effort": "none",
                    "content": "Reply with the word pong and one short sentence.",
                },
                expected_status={202},
            ),
        )
        text_message_id = require_string(text_post.json_body, "message_id", "text post response")
        step(
            "SSE user message",
            lambda: live.wait_for(
                lambda event: event.event == "message"
                and isinstance(event.json_data, dict)
                and event.json_data.get("id") == text_message_id
                and event.json_data.get("role") == "user",
                "text user message",
            ),
        )
        text_assistant = step(
            "SSE assistant message",
            lambda: live.wait_for(
                lambda event: event.event == "message"
                and isinstance(event.json_data, dict)
                and event.json_data.get("role") == "assistant",
                "assistant response to text",
            ),
        )
        assert_no_secret(json.dumps(text_assistant.json_data, sort_keys=True), cfg)

        history = step("GET message history", lambda: get_history_chronological(cfg, session_id, token))
        assert_history_has_role(history, text_message_id, "user")
        assistant_id = require_string(text_assistant.json_data, "id", "assistant SSE message")
        assert_history_has_role(history, assistant_id, "assistant")

        replay_events = step("SSE replay", lambda: replay_until_history_end(cfg, session_id, token))
        replay_ids = [event.event_id for event in replay_events if event.event == "message"]
        if text_message_id not in replay_ids or assistant_id not in replay_ids:
            raise SmokeError("SSE replay did not include persisted text user and assistant messages")

        if not cfg.skip_image:
            run_image_step(cfg, session_id, token, live)

        run_pending_queue_step(cfg, session_id, token)
    finally:
        live.close()

    print("[PASS] M1c live API smoke passed")


def provider_precheck(cfg: Config) -> None:
    def run() -> HttpResult:
        return request_json(
            "GET",
            join_url(cfg.llm_endpoint, "/models"),
            headers={"Authorization": f"Bearer {cfg.llm_api_key}"},
            expected_status={200},
            timeout_seconds=cfg.timeout_seconds,
        )

    result = step("provider GET /models", run)
    data = result.json_body.get("data") if isinstance(result.json_body, dict) else None
    if not isinstance(data, list):
        raise SmokeError("provider /models response did not contain a data array")
    model_ids = [item.get("id") for item in data if isinstance(item, dict) and isinstance(item.get("id"), str)]
    if cfg.llm_model not in model_ids:
        sample = ", ".join(model_ids[:20])
        raise SmokeError(f"provider /models did not list {cfg.llm_model!r}; first model ids: {sample}")


def run_image_step(cfg: Config, session_id: str, token: str, live: SseClient) -> None:
    before = step("history before image", lambda: get_history_chronological(cfg, session_id, token))
    before_ids = {message.get("id") for message in before}
    post = step(
        "POST inline image message",
        lambda: authed_json(
            cfg,
            "POST",
            f"/api/sessions/{session_id}/messages",
            token,
            body={
                "reasoning_effort": "none",
                "content": [
                    {"type": "text", "text": "Describe this image briefly."},
                    {"type": "image_url", "image_url": {"url": INLINE_PNG_DATA_URL}},
                ],
            },
            expected_status={202},
        ),
    )
    image_message_id = require_string(post.json_body, "message_id", "image post response")
    step(
        "SSE image user message",
        lambda: live.wait_for(
            lambda event: event.event == "message"
            and isinstance(event.json_data, dict)
            and event.json_data.get("id") == image_message_id
            and event.json_data.get("role") == "user",
            "image user message",
        ),
    )
    assistant = step(
        "SSE image assistant or diagnostic",
        lambda: live.wait_for(
            lambda event: event.event == "message"
            and isinstance(event.json_data, dict)
            and event.json_data.get("role") == "assistant"
            and event.json_data.get("id") not in before_ids,
            "assistant response to image",
        ),
    )
    assert_no_secret(json.dumps(assistant.json_data, sort_keys=True), cfg)


def run_pending_queue_step(cfg: Config, session_id: str, token: str) -> None:
    before = step("history before queue burst", lambda: get_history_chronological(cfg, session_id, token))
    before_ids = {message.get("id") for message in before}

    prompts = [
        "Queue smoke U1: answer with the phrase queue-one and one short sentence.",
        "Queue smoke U2: answer with the phrase queue-two and one short sentence.",
        "Queue smoke U3: answer with the phrase queue-three and one short sentence.",
    ]
    results: list[HttpResult | None] = [None, None, None]
    errors: list[BaseException] = []

    def post(index: int, prompt: str) -> None:
        try:
            results[index] = authed_json(
                cfg,
                "POST",
                f"/api/sessions/{session_id}/messages",
                token,
                body={"reasoning_effort": "none", "content": prompt},
                expected_status={202},
            )
        except BaseException as exc:
            errors.append(exc)

    threads = [threading.Thread(target=post, args=(index, prompt), daemon=True) for index, prompt in enumerate(prompts)]
    step("POST queue burst", lambda: start_and_join(threads, errors))
    queue_ids = [require_string(result.json_body, "message_id", "queue post response") for result in results if result is not None]
    if len(queue_ids) != 3:
        raise SmokeError(f"queue burst returned {len(queue_ids)} message ids, expected 3")

    def queue_condition(history: list[dict[str, Any]]) -> bool:
        positions = positions_by_id(history)
        if not all(message_id in positions for message_id in queue_ids):
            return False
        last_queue_pos = max(positions[message_id] for message_id in queue_ids)
        return any(index > last_queue_pos and message.get("role") == "assistant" for index, message in enumerate(history))

    history = step(
        "queue burst persisted and answered",
        lambda: wait_for_history(cfg, session_id, token, queue_condition, "queued messages to persist and receive assistant response"),
    )
    assert_pending_queue_order(history, before_ids, queue_ids)


def start_and_join(threads: list[threading.Thread], errors: list[BaseException]) -> None:
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    if errors:
        raise SmokeError(f"queue post failed: {errors[0]}") from errors[0]


def assert_pending_queue_order(history: list[dict[str, Any]], before_ids: set[Any], queue_ids: list[str]) -> None:
    new_messages = [message for message in history if message.get("id") not in before_ids]
    first_assistant_index = next(
        (index for index, message in enumerate(new_messages) if message.get("role") == "assistant"),
        None,
    )
    if first_assistant_index is None:
        raise SmokeError("queue burst produced no assistant message")

    user_ids_before_first_assistant = [
        message.get("id")
        for message in new_messages[:first_assistant_index]
        if message.get("role") == "user" and message.get("id") in queue_ids
    ]
    if len(user_ids_before_first_assistant) > 1:
        roles = [(message.get("role"), message.get("id")) for message in new_messages]
        raise SmokeError(
            "queue burst persisted multiple new user messages before the first assistant boundary; "
            f"roles after burst: {roles}"
        )


def wait_for_history(
    cfg: Config,
    session_id: str,
    token: str,
    predicate: Callable[[list[dict[str, Any]]], bool],
    label: str,
) -> list[dict[str, Any]]:
    deadline = time.monotonic() + cfg.timeout_seconds
    last_history: list[dict[str, Any]] = []
    while time.monotonic() < deadline:
        last_history = get_history_chronological(cfg, session_id, token)
        if predicate(last_history):
            return last_history
        time.sleep(2)
    roles = [(message.get("role"), message.get("id")) for message in last_history]
    raise SmokeError(f"timed out waiting for {label}; history roles: {roles}")


def get_history_chronological(cfg: Config, session_id: str, token: str) -> list[dict[str, Any]]:
    result = authed_json(cfg, "GET", f"/api/sessions/{session_id}/messages?limit=200", token)
    if not isinstance(result.json_body, list):
        raise SmokeError("message history response was not a list")
    return list(reversed(result.json_body))


def replay_until_history_end(cfg: Config, session_id: str, token: str) -> list[SseEvent]:
    client = SseClient(join_url(cfg.api, f"/api/sessions/{session_id}/stream?replay_limit=50"), token, cfg.timeout_seconds)
    client.start()
    try:
        client.wait_for(lambda event: event.event == "history_end", "replay history_end")
        return client.snapshot()
    finally:
        client.close()


def authed_json(
    cfg: Config,
    method: str,
    path: str,
    token: str,
    body: Any | None = None,
    expected_status: set[int] | None = None,
) -> HttpResult:
    return request_json(
        method,
        join_url(cfg.api, path),
        headers={"Authorization": f"Bearer {token}"},
        body=body,
        expected_status=expected_status or {200},
        timeout_seconds=cfg.timeout_seconds,
    )


def request_json(
    method: str,
    url: str,
    headers: dict[str, str] | None = None,
    body: Any | None = None,
    expected_status: set[int] | None = None,
    timeout_seconds: float = 240,
) -> HttpResult:
    request_headers = {"Accept": "application/json", **(headers or {})}
    data: bytes | None = None
    if body is not None:
        data = json.dumps(body).encode("utf-8")
        request_headers["Content-Type"] = "application/json"
    request = urllib.request.Request(url, data=data, headers=request_headers, method=method)

    try:
        with urllib.request.urlopen(request, timeout=timeout_seconds) as response:
            status = response.status
            body_text = response.read().decode("utf-8", errors="replace")
            response_headers = dict(response.headers.items())
    except urllib.error.HTTPError as exc:
        body_text = exc.read().decode("utf-8", errors="replace")
        raise SmokeError(f"{method} {url} returned HTTP {exc.code}: {truncate(body_text)}") from exc
    except urllib.error.URLError as exc:
        raise SmokeError(f"{method} {url} failed: {exc.reason}") from exc

    expected = expected_status or {200}
    if status not in expected:
        raise SmokeError(f"{method} {url} returned HTTP {status}, expected {sorted(expected)}: {truncate(body_text)}")

    json_body: Any = None
    if body_text.strip():
        try:
            json_body = json.loads(body_text)
        except json.JSONDecodeError as exc:
            raise SmokeError(f"{method} {url} returned non-JSON body: {truncate(body_text)}") from exc

    return HttpResult(status=status, headers=response_headers, body_text=body_text, json_body=json_body)


def assert_redacted_config(body: Any, cfg: Config) -> None:
    if not isinstance(body, dict):
        raise SmokeError("admin config response was not an object")
    if body.get("llm_endpoint") != cfg.llm_endpoint:
        raise SmokeError("admin config response has unexpected llm_endpoint")
    if body.get("llm_model") != cfg.llm_model:
        raise SmokeError("admin config response has unexpected llm_model")
    if body.get("llm_api_key") != "<redacted>":
        raise SmokeError("admin config response did not redact llm_api_key")
    assert_no_secret(json.dumps(body, sort_keys=True), cfg)


def assert_history_has_role(history: list[dict[str, Any]], message_id: str, role: str) -> None:
    if not any(message.get("id") == message_id and message.get("role") == role for message in history):
        roles = [(message.get("role"), message.get("id")) for message in history]
        raise SmokeError(f"history did not contain {role} message {message_id}; roles: {roles}")


def assert_no_secret(text: str, cfg: Config) -> None:
    if cfg.llm_api_key and cfg.llm_api_key in text:
        raise SmokeError("response leaked LLM_API_KEY")


def positions_by_id(history: list[dict[str, Any]]) -> dict[str, int]:
    return {message["id"]: index for index, message in enumerate(history) if isinstance(message.get("id"), str)}


def require_string(body: Any, key: str, label: str) -> str:
    if not isinstance(body, dict) or not isinstance(body.get(key), str) or body[key] == "":
        raise SmokeError(f"{label} missing string field {key!r}")
    return body[key]


def step(name: str, action: Callable[[], Any]) -> Any:
    print(f"[step] {name} ... ", end="", flush=True)
    started = time.monotonic()
    result = action()
    elapsed = time.monotonic() - started
    print(f"ok ({elapsed:.1f}s)")
    return result


def unique_email() -> str:
    suffix = "".join(random.choice(string.ascii_lowercase + string.digits) for _ in range(8))
    return f"m1c-smoke+{int(time.time())}-{suffix}@example.com"


def join_url(base: str, path: str) -> str:
    return f"{base.rstrip('/')}/{path.lstrip('/')}"


def truncate(text: str, limit: int = 1000) -> str:
    if len(text) <= limit:
        return text
    return text[:limit] + "...<truncated>"


if __name__ == "__main__":
    raise SystemExit(main())
