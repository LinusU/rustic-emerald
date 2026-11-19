#!/usr/bin/env python3
"""Compare the C and Rust game implementations with identical host devices and input."""

import argparse
import fcntl
from itertools import zip_longest
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_REPLAY = ROOT / "vendor/pokeemerald/tools/wasm_replays/mudkip_starter.txt"
GAME_TEST = "game::tests::mudkip_wins_birch_rescue_and_cartridge_save_survives_reboot"


def build(target, reference):
    options = ["--target-dir", str(target), "--release", "--locked"]
    if reference:
        options.extend(["--no-default-features", "--features", "reference-c"])
    command = ["cargo", "build", *options, "--bin", "rustic-emerald",
               "--message-format=json-render-diagnostics"]
    result = subprocess.run(command, cwd=ROOT, stdout=subprocess.PIPE, text=True, check=True)
    for line in result.stdout.splitlines():
        msg = json.loads(line)
        if msg.get("executable") and msg["target"]["name"] == "rustic-emerald":
            return Path(msg["executable"]), options
    raise RuntimeError("Cargo did not report the Rustic Emerald executable")


def trace_script(source, destination, extend, starter):
    lines = []
    last = -1
    for number, original in enumerate(source.read_text().splitlines(), 1):
        parts = original.split("#", 1)[0].split()
        if not parts:
            continue
        if len(parts) < 3 or not parts[0].isdigit():
            raise ValueError(f"{source}:{number}: invalid replay command")
        last = max(last, int(parts[0]))
        # Birch's bag opens with the cursor on Torchic. These two right
        # presses are the only differences between the three recorded routes.
        if extend and parts[0] in {"26108", "26112", "26172", "26176"} and parts[1:3] == ["button", "right"]:
            if starter == "torchic":
                continue
            if starter == "treecko":
                parts[2] = "left"
        if parts[1] != "screenshot":
            lines.append(" ".join(parts))
    if last < 0:
        raise ValueError(f"{source} has no replay commands")
    if extend:
        # Same inputs as the starter victory test. Release B after the upstream
        # route, use the first move and advance the post-battle dialogue in the lab.
        lines += ["27229 button b off", "27229 button a off"]
        for frame in range(27240, 35000, 24):
            lines += [f"{frame} button a on", f"{frame + 4} button a off"]
        last = 35000
        lines.append(f"{last} probe birch-starter {starter}")
        # Reboot using the same four keys as the cartridge, then release them.
        for key in ("a", "b", "select", "start"):
            lines += [f"{last + 1} button {key} on", f"{last + 3} button {key} off"]
        last += 360
    lines.extend(f"{frame} screenshot parity" for frame in range(last + 1))
    destination.write_text("\n".join(lines) + "\n")
    return last + 1


def render(executable, replay, path):
    with path.open("w") as out:
        return subprocess.run([executable, "--mute", "--replay", replay], cwd=ROOT, stdout=out)


def compare(expected, actual, count):
    with expected.open() as ref, actual.open() as candidate:
        frames = 0
        for frame, (a, b) in enumerate(zip_longest(ref, candidate, fillvalue="")):
            left, right = a.split(), b.split()
            if left != right or len(left) != 3 or left[0] != str(frame) or left[2] != "parity":
                raise ValueError(f"frame {frame} differs\n  C:    {a.strip()}\n  Rust: {b.strip()}\n"
                                 f"Complete traces: {expected} and {actual}")
            frames += 1
        if frames != count:
            raise ValueError(f"expected {count} frames, compared {frames}")
    return frames


def run():
    parser = argparse.ArgumentParser(description=__doc__)
    route = parser.add_mutually_exclusive_group()
    route.add_argument("--all-starters", action="store_true", help="compare the opening with each starter")
    route.add_argument("--replay", type=Path, help="compare a different replay without extending its route")
    parser.add_argument("--output", type=Path, default=ROOT / "target/parity",
                        help="directory for the C build, replay and frame traces")
    parser.add_argument("--frames-only", action="store_true",
                        help="compare the replay alone, skipping party, heap and flash checks")
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    with (output / ".compare.lock").open("a+b") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        compare_builds(output, args.replay, args.frames_only, args.all_starters)


def compare_builds(output, replay, frames_only, all_starters):
    ref, ref_options = build(output / "reference-target", True)
    candidate, rust_options = build(ROOT / "target", False)
    starters = ("mudkip", "treecko", "torchic") if all_starters else ("mudkip",)
    for starter in starters:
        directory = output / starter if all_starters else output
        directory.mkdir(exist_ok=True)
        script = directory / "inputs.txt"
        count = trace_script(replay or DEFAULT_REPLAY, script, replay is None, starter)
        reference_trace, candidate_trace = directory / "reference.tsv", directory / "rust.tsv"
        render(ref, script, reference_trace).check_returncode()
        candidate_run = render(candidate, script, candidate_trace)
        # A crash still leaves a prefix of the trace. Surface the first
        # divergent or missing frame before reporting the process exit.
        frames = compare(reference_trace, candidate_trace, count)
        candidate_run.check_returncode()
        label = replay.name if replay else starter.capitalize()
        print(f"{label}: {frames:,} RGBA frames match the C reference.", flush=True)
    if not frames_only:
        reference_save, rust_save = output / "reference-parity.sav", output / "rust-parity.sav"
        for options, save in ((ref_options, reference_save), (rust_options, rust_save)):
            # Require an executed test: a stale name would otherwise exit 0.
            save.unlink(missing_ok=True)
            result = subprocess.run(["cargo", "test", *options, "--bin", "rustic-emerald",
                                     GAME_TEST, "--", "--exact"], cwd=ROOT, check=True,
                                    env={**os.environ, "RUSTIC_PARITY_SAVE": str(save)},
                                    text=True, stdout=subprocess.PIPE)
            if "1 passed;" not in result.stdout:
                raise RuntimeError(f"save and reset test did not run:\n{result.stdout}")
        expected, actual = reference_save.read_bytes(), rust_save.read_bytes()
        if len(expected) != 128 * 1024 or len(actual) != len(expected):
            raise ValueError(f"unexpected flash lengths: C {len(expected)}, Rust {len(actual)}")
        if actual != expected:
            offset = next(i for i, (a, b) in enumerate(zip(expected, actual)) if a != b)
            raise ValueError(f"flash offset {offset:#07x} differs: C {expected[offset]:#04x}, "
                             f"Rust {actual[offset]:#04x}\nComplete saves: {reference_save} and {rust_save}")
        print("C and Rust matched 1,158 party records and the entire 128 KiB flash save. "
              "Both passed the cold boot and reset tests.")


if __name__ == "__main__":
    try:
        run()
    except (ValueError, RuntimeError, OSError, subprocess.CalledProcessError) as error:
        sys.exit(str(error))
