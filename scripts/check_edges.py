#!/usr/bin/env python3
"""Checks the capability-layering rules between workspace crates (see CLAUDE.md).

Only normal and build dependencies count; dev-dependencies may reach anywhere.
"""

import json
import subprocess
import sys

INTERFACES = {"arch-interfaces", "platform-interfaces", "irq-interfaces", "boot"}
FACADES = {"arch": "arch-interfaces", "platform": "platform-interfaces"}
# Concrete backend -> the only crates that may depend on it.
IMPLEMENTATIONS = {
    "arch-x86_64": {"arch"},
    "platform-pc-x86_64": {"platform"},
    # x86 register helpers shared with `core_local`'s gs-base access.
    "arch-x86_64-primitives": {"arch-x86_64", "core_local"},
}
# The composition root. Nothing may depend on it.
BINARY = "kernel"
# Host test helpers: dev-dependency only.
TEST_ONLY = {"test-host"}


def workspace_edges():
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    packages = json.loads(out)["packages"]
    names = {p["name"] for p in packages}
    return {
        p["name"]: {
            d["name"] for d in p["dependencies"] if d["name"] in names and d["kind"] != "dev"
        }
        for p in packages
    }


def violations(edges):
    found = []
    for crate, deps in sorted(edges.items()):
        for dep in sorted(deps):
            if dep == BINARY:
                found.append(f"{crate} depends on the binary crate {dep}")
            if dep in TEST_ONLY:
                found.append(f"{crate} depends on {dep}, which may only be a dev-dependency")
            if dep in IMPLEMENTATIONS and crate not in IMPLEMENTATIONS[dep]:
                allowed = ", ".join(sorted(IMPLEMENTATIONS[dep]))
                found.append(f"{crate} depends on {dep}, only {allowed} may")
            if crate in INTERFACES and dep not in INTERFACES:
                found.append(f"interface crate {crate} depends on non-interface crate {dep}")
        if crate in FACADES:
            extra = deps - {FACADES[crate]} - {i for i, users in IMPLEMENTATIONS.items() if crate in users}
            for dep in sorted(extra):
                found.append(f"facade {crate} depends on {dep}")
    return found


def main():
    found = violations(workspace_edges())
    for line in found:
        print(f"edge violation: {line}")
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main())
