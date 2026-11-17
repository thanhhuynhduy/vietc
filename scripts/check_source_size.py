#!/usr/bin/env python3
"""Prevent oversized Rust production modules from growing unnoticed."""

from pathlib import Path

MAX_NEW_FILE_LINES = 1_000
EXISTING_LIMITS = {
    "daemon/src/app_state.rs": 1_239,
    "daemon/src/evdev_loop.rs": 1_402,
    "daemon/src/ibus_engine.rs": 1_404,
    "protocol/src/uinput_monitor/clipboard.rs": 1_229,
    "protocol/src/wayland_clipboard.rs": 1_791,
    "uinputd/src/main.rs": 1_285,
}
SOURCE_ROOTS = ("cli", "daemon", "engine", "protocol", "uinputd", "vietcctl")


def line_count(path: Path) -> int:
    with path.open(encoding="utf-8") as source:
        return sum(1 for _ in source)


def main() -> int:
    violations: list[str] = []

    for root_name in SOURCE_ROOTS:
        source_root = Path(root_name) / "src"
        if not source_root.exists():
            continue

        for path in sorted(source_root.rglob("*.rs")):
            relative_path = path.as_posix()
            lines = line_count(path)
            limit = EXISTING_LIMITS.get(relative_path, MAX_NEW_FILE_LINES)

            if lines > limit:
                category = (
                    "existing oversized module grew"
                    if relative_path in EXISTING_LIMITS
                    else "production module exceeds limit"
                )
                violations.append(
                    f"{relative_path}: {lines} lines, limit {limit} ({category})"
                )

    if violations:
        print("Rust source-size quality gate failed:")
        for violation in violations:
            print(f"  - {violation}")
        print(
            "Split orchestration, platform adapters, state machines, or focused helpers "
            "before adding more code."
        )
        return 1

    print(
        "Rust source-size quality gate passed: no existing oversized module grew "
        f"and no new production module exceeds {MAX_NEW_FILE_LINES} lines."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())