#!/usr/bin/env python3
"""Which tests are slower than their tier allows.

The tiers live in .config/nextest.toml as filters; this reads the JUnit
report of a run of every test and each tier's list from `cargo nextest
list`, and prints every test over its tier's limit — the ones to move down a
tier, or to make faster.

    cargo nextest run -P daily -j 1
    python3 tools/test-tiers.py target/nextest/daily/junit.xml

One at a time (`-j 1`, ~12 min): side by side with the others a test can
take twenty times longer than alone, and the report would be noise.

Limits are for a developer machine; a CI runner is slower, so it passes
`--scale` (2 on a GitHub macOS runner). Prints, never fails:
a runner that stalled once is not a reason to go red.
"""

import argparse
import subprocess
import sys
import re
import xml.etree.ElementTree as ET

# Budgets stay medium however long they run (DNA, postulates 1 and 6): the
# same tests .config/nextest.toml runs alone.
BUDGETS = re.compile(r"^(runity::scaling|.*::iteration_budget) |a_big_scene_stays_quick$")


def tier_list(profile):
    out = subprocess.run(
        ["cargo", "nextest", "list", "--workspace", "-P", profile, "--color", "never"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    tests = set()
    binary = None
    for line in out.splitlines():
        if not line.strip():
            continue
        if line.startswith(" "):
            tests.add((binary, line.strip()))
        else:
            binary, _, name = line.partition(" ")
            if name:
                tests.add((binary, name))
                binary = None
    return tests


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("junit")
    parser.add_argument("--fast", type=float, default=1.0, help="seconds a fast test may take")
    parser.add_argument("--medium", type=float, default=5.0, help="seconds a medium test may take")
    parser.add_argument("--scale", type=float, default=1.0, help="multiply both limits")
    args = parser.parse_args()

    times = {}
    for case in ET.parse(args.junit).getroot().iter("testcase"):
        times[(case.get("classname"), case.get("name"))] = float(case.get("time", 0))

    fast = tier_list("default")
    ci = tier_list("ci")
    limits = [("fast", fast, args.fast * args.scale), ("medium", ci - fast, args.medium * args.scale)]

    over = []
    for tier, tests, limit in limits:
        for test in sorted(tests):
            t = times.get(test)
            if BUDGETS.search(" ".join(test)):
                continue
            if t is not None and t > limit:
                over.append((t, tier, limit, test))
    if not over:
        print(f"every test within its tier (fast ≤ {limits[0][2]:g} s, medium ≤ {limits[1][2]:g} s)")
        return
    print(f"{len(over)} tests slower than their tier:")
    for t, tier, limit, (binary, name) in sorted(over, reverse=True):
        print(f"  {t:7.2f} s  {tier:6} (≤ {limit:g} s)  {binary} {name}")


if __name__ == "__main__":
    sys.exit(main())
