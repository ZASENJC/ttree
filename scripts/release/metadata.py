#!/usr/bin/env python3
"""Single source of release metadata; no dependencies or signing material."""
import argparse
import os
from pathlib import Path
import re
import tomllib

ROOT = Path(__file__).resolve().parents[2]


def version():
    value = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    if not re.fullmatch(r"\d+\.\d+\.\d+", value):
        raise ValueError("workspace version must be X.Y.Z")
    return value


def resolve(event, ref_type, ref_name, run_number, attempt):
    app_version = version()
    stable_tag = f"v{app_version}"
    publish = "false"
    prerelease = "true"
    release_tag = f"{stable_tag}-dev.{run_number}.{attempt}"
    if event == "push" and ref_type == "tag":
        if ref_name != stable_tag:
            raise ValueError(f"tag {ref_name} must match {stable_tag}")
        release_tag, publish, prerelease = stable_tag, "true", "false"
    elif event == "push" and ref_type == "branch" and ref_name == "main":
        publish = "true"
    return dict(app_version=app_version, release_tag=release_tag,
                publish=publish, prerelease=prerelease)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--github-output", type=Path)
    args = parser.parse_args()
    if args.github_output:
        values = resolve(os.environ["GITHUB_EVENT_NAME"], os.getenv("GITHUB_REF_TYPE", ""),
                         os.getenv("GITHUB_REF_NAME", ""), os.environ["GITHUB_RUN_NUMBER"],
                         os.getenv("GITHUB_RUN_ATTEMPT", "1"))
        with args.github_output.open("a") as output:
            output.write("".join(f"{key}={value}\n" for key, value in values.items()))
    else:
        print(version())
