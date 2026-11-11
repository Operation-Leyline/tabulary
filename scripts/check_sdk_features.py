"""Check the resolved normal/build dependency graph, without adding test crates.

Run with the same Cargo toolchain and feature arguments as the build under test:
    python scripts/check_sdk_features.py --all-features
    python scripts/check_sdk_features.py --no-default-features
"""

from pathlib import Path
import subprocess
import sys
import tomllib


ROOT = Path(__file__).resolve().parents[1]


def check(feature_args):
    manifest = tomllib.loads((ROOT / "crates/tabulary/Cargo.toml").read_text(encoding="utf-8"))
    dependency = manifest["dependencies"]["aws-sdk-dynamodb"]
    assert dependency["version"] == "1.130.0", "keep the declared SDK API floor"
    assert dependency["default-features"] is False, "the application owns transport features"

    result = subprocess.run(
        [
            "cargo", "tree", "-p", "tabulary", "--edges", "normal,build",
            "--prefix", "none", "--format", "{p}|{f}", "--no-dedupe",
            "--locked", *feature_args,
        ],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    # Cargo tree selects normal/build features as well as edges, excluding
    # transport enabled only by an application's or endpoint test's dev deps.
    packages = [line.split("|", 1) for line in result.stdout.splitlines()]
    sdk = {package for package, _ in packages if package.split()[0] == "aws-sdk-dynamodb"}
    assert len(sdk) == 1, "the runtime must depend on exactly one DynamoDB SDK"
    assert all(not features for package, features in packages if package in sdk), (
        "the runtime must not enable SDK features"
    )

    # Detect transport introduced transitively as well as direct SDK features.
    transport_crates = {
        "aws-smithy-http-client", "hyper", "hyper-util", "hyper-rustls",
        "rustls", "native-tls", "reqwest",
    }
    enabled_transport = sorted(
        {package.split()[0] for package, _ in packages} & transport_crates
    )
    assert not enabled_transport, f"transport dependencies enabled: {enabled_transport}"

    has_derive = any(package.split()[0] == "tabulary-derive" for package, _ in packages)
    assert has_derive == ("--no-default-features" not in feature_args), (
        "derive must be optional and enabled by default"
    )
    print(f"{sdk.pop()}: no transport enabled ({' '.join(feature_args) or 'default'})")


if __name__ == "__main__":
    check(sys.argv[1:])
